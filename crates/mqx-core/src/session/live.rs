use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::TrySendError,
    },
    time::SystemTime,
};

use rumqttc::{AsyncClient, ConnectionError, Event, EventLoop, Incoming};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Duration, Instant, MissedTickBehavior, interval_at},
};
use tracing::{debug, warn};

use super::{
    Session, Status, Sub,
    dto::{SessionStats, SessionStatus, TreeBatch, TreeNodeDto},
    tls::{mqtt_options, qos_from_rumqttc},
};
use crate::{
    config::UiConfig,
    error::{Error, Result},
    message::{Inbound, decode_inbound},
    profiles::ConnectionProfile,
    session::dto::MessageDto,
};

const BATCH_INTERVAL: Duration = Duration::from_millis(50);
const STATS_INTERVAL: Duration = Duration::from_secs(1);
const BACKOFF_START: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(8);
const DECODE_QUEUE: usize = 1024;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum SessionEvent {
    Status(SessionStatus),
    Stats(SessionStats),
    TreeBatch(TreeBatch),
    TopicMessage(MessageDto),
}

impl SessionEvent {
    pub fn epoch(&self) -> u64 {
        match self {
            Self::Status(payload) => payload.epoch,
            Self::Stats(payload) => payload.epoch,
            Self::TreeBatch(payload) => payload.epoch,
            Self::TopicMessage(payload) => payload.epoch,
        }
    }
}

pub struct LiveHandle {
    session: Arc<Mutex<Session>>,
    shutdown: watch::Sender<bool>,
    client: AsyncClient,
    task: Mutex<Option<JoinHandle<()>>>,
    epoch: u64,
}

impl LiveHandle {
    pub fn spawn(
        profile: ConnectionProfile,
        password: Option<String>,
        ui: &UiConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SessionEvent>)> {
        let options = mqtt_options(&profile, password)?;
        let request_cap = profile.subscriptions.len().saturating_add(16).max(64);
        let (client, eventloop) = AsyncClient::new(options, request_cap);
        let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        let session = Session::new(&profile, client.clone(), ui, epoch);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let _ = event_tx.send(SessionEvent::Status(session.status_event()));

        let (raw_tx, raw_rx) = std::sync::mpsc::sync_channel::<Inbound>(DECODE_QUEUE);
        let (decoded_tx, decoded_rx) = mpsc::channel(DECODE_QUEUE);
        // WHY: JSON decode is CPU-heavy; keep rumqttc poll() off that work.
        let decode = std::thread::Builder::new()
            .name("mqx-decode".into())
            .spawn(move || {
                while let Ok(inbound) = raw_rx.recv() {
                    if decoded_tx.blocking_send(decode_inbound(inbound)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| Error::Mqtt(format!("decode worker: {e}")))?;

        let session = Arc::new(Mutex::new(session));
        let (shutdown, shutdown_rx) = watch::channel(false);

        let task = tokio::spawn(run_live(
            Arc::clone(&session),
            client.clone(),
            eventloop,
            event_tx,
            shutdown_rx,
            raw_tx,
            decoded_rx,
            decode,
        ));

        let handle = Self {
            session,
            shutdown,
            client,
            task: Mutex::new(Some(task)),
            epoch,
        };

        Ok((handle, event_rx))
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn session(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Session>> {
        self.session.lock()
    }

    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.client.disconnect().await;
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl Drop for LiveHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_live(
    session: Arc<Mutex<Session>>,
    client: AsyncClient,
    mut eventloop: EventLoop,
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    mut shutdown: watch::Receiver<bool>,
    raw_tx: std::sync::mpsc::SyncSender<Inbound>,
    mut decoded_rx: mpsc::Receiver<crate::Message>,
    decode: std::thread::JoinHandle<()>,
) {
    let mut batch = PendingBatch::default();
    let mut pending_subs: VecDeque<Sub> = VecDeque::new();
    let mut window_count = 0u64;
    let mut backoff = BACKOFF_START;
    let mut backoff_deadline: Option<Instant> = None;
    let start = Instant::now();
    let mut batch_tick = interval_at(start + BATCH_INTERVAL, BATCH_INTERVAL);
    batch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut stats_tick = interval_at(start + STATS_INTERVAL, STATS_INTERVAL);
    stats_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let (profile_id, epoch) = {
        let guard = lock(&session);
        (guard.id.clone(), guard.epoch())
    };

    loop {
        if shutting_down(&shutdown) {
            break;
        }

        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || shutting_down(&shutdown) {
                    break;
                }
            }
            event = eventloop.poll(), if backoff_deadline.is_none() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        backoff = BACKOFF_START;
                        let subs = {
                            let mut guard = lock(&session);
                            guard.set_status(Status::Connected);
                            guard.subscriptions().to_vec()
                        };
                        pending_subs = subs.into();
                        emit_status(&session, &event_tx);
                    }
                    Ok(Event::Incoming(Incoming::Publish(publish))) => {
                        let inbound = Inbound {
                            topic: publish.topic,
                            payload: publish.payload,
                            retain: publish.retain,
                            qos: qos_from_rumqttc(publish.qos),
                            timestamp: SystemTime::now(),
                        };
                        match raw_tx.try_send(inbound) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                debug!("dropping inbound; decode queue full");
                            }
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    Ok(_) => {}
                    Err(ConnectionError::RequestsDone) => break,
                    Err(error) if fatal_connection_error(&error) => {
                        {
                            let mut guard = lock(&session);
                            guard.set_status(Status::Error {
                                msg: error.to_string(),
                            });
                        }
                        emit_status(&session, &event_tx);
                        break;
                    }
                    Err(error) => {
                        debug!(%error, "mqtt connection error");
                        {
                            let mut guard = lock(&session);
                            guard.set_io_error(error.to_string());
                        }
                        emit_status(&session, &event_tx);
                        pending_subs.clear();
                        backoff_deadline = Some(Instant::now() + backoff);
                        backoff = (backoff.saturating_mul(2)).min(BACKOFF_MAX);
                    }
                }
            }
            sub_result = subscribe_front(&client, pending_subs.front()), if !pending_subs.is_empty() && backoff_deadline.is_none() => {
                let sub = pending_subs.pop_front().expect("guarded");
                if let Err(error) = sub_result {
                    warn!(topic = %sub.topic, %error, "subscribe failed");
                    {
                        let mut guard = lock(&session);
                        guard.set_subscribe_error(error.to_string());
                    }
                    emit_status(&session, &event_tx);
                }
            }
            Some(message) = decoded_rx.recv() => {
                window_count = window_count.saturating_add(1);
                let (applied, status) = {
                    let mut guard = lock(&session);
                    let before = guard.ram_exhausted();
                    let applied = guard.ingest(message);
                    let status = if guard.ram_exhausted() != before {
                        Some(guard.status_event())
                    } else {
                        None
                    };
                    (applied, status)
                };
                if let Some(status) = status {
                    let _ = event_tx.send(SessionEvent::Status(status));
                }
                if let Some(dto) = applied.topic_message {
                    let _ = event_tx.send(SessionEvent::TopicMessage(dto));
                }
                batch.merge(applied.upserts, applied.deletes);
            }
            _ = wait_backoff(backoff_deadline) => {
                backoff_deadline = None;
            }
            _ = batch_tick.tick() => {
                if let Some(payload) = batch.take(&profile_id, epoch) {
                    let _ = event_tx.send(SessionEvent::TreeBatch(payload));
                }
            }
            _ = stats_tick.tick() => {
                let rate = window_count as f64;
                window_count = 0;
                let stats = {
                    let guard = lock(&session);
                    guard.stats(rate)
                };
                let _ = event_tx.send(SessionEvent::Stats(stats));
            }
        }
    }

    drop(raw_tx);
    decoded_rx.close();
    while decoded_rx.try_recv().is_ok() {}
    let _ = decode.join();

    {
        let mut guard = lock(&session);
        guard.clear_tree();
        if !matches!(guard.status, Status::Error { .. }) {
            guard.set_status(Status::Disconnected);
        }
    }
    emit_status(&session, &event_tx);
}

async fn subscribe_front(
    client: &AsyncClient,
    sub: Option<&Sub>,
) -> std::result::Result<(), rumqttc::ClientError> {
    let sub = sub.expect("subscribe polled without a pending filter");
    client.subscribe(sub.topic.clone(), sub.qos).await
}

fn fatal_connection_error(error: &ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::ConnectionRefused(_) | ConnectionError::Tls(_)
    )
}

fn shutting_down(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_backoff(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn emit_status(session: &Mutex<Session>, event_tx: &mpsc::UnboundedSender<SessionEvent>) {
    let status = lock(session).status_event();
    let _ = event_tx.send(SessionEvent::Status(status));
}

fn lock(session: &Mutex<Session>) -> std::sync::MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct PendingBatch {
    upserts: HashMap<String, TreeNodeDto>,
    deletes: Vec<String>,
}

impl PendingBatch {
    fn merge(&mut self, upserts: Vec<TreeNodeDto>, deletes: Vec<String>) {
        for path in deletes {
            self.upserts.remove(&path);
            if !self.deletes.iter().any(|existing| existing == &path) {
                self.deletes.push(path);
            }
        }
        for dto in upserts {
            self.deletes.retain(|path| path != &dto.path);
            self.upserts.insert(dto.path.clone(), dto);
        }
    }

    fn take(&mut self, profile_id: &str, epoch: u64) -> Option<TreeBatch> {
        if self.upserts.is_empty() && self.deletes.is_empty() {
            return None;
        }
        Some(TreeBatch {
            profile_id: profile_id.to_string(),
            epoch,
            upserts: self.upserts.drain().map(|(_, dto)| dto).collect(),
            deletes: std::mem::take(&mut self.deletes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{Format, Freshness},
        profiles::{Protocol, SessionConfig, TlsConfig},
    };

    fn dto(path: &str) -> TreeNodeDto {
        TreeNodeDto {
            segment: path.rsplit('/').next().unwrap_or(path).into(),
            path: path.into(),
            child_count: 0,
            has_payload: true,
            retain: false,
            freshness: Freshness::Fresh,
            format: Format::Text,
            last_ms: 0,
            history_len: 1,
        }
    }

    fn unreachable_profile(id: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: id.into(),
            name: id.into(),
            protocol: Protocol::Mqtt,
            host: "127.0.0.1".into(),
            port: 1,
            client_id: format!("mqx-{id}"),
            username: String::new(),
            tls: TlsConfig::default(),
            session: SessionConfig::default(),
            subscriptions: Vec::new(),
            last_will: None,
            websocket_path: None,
        }
    }

    #[test]
    fn pending_batch_last_write_wins_and_delete_clears_upsert() {
        let mut batch = PendingBatch::default();
        batch.merge(vec![dto("home/lamp")], vec![]);
        batch.merge(vec![], vec!["home/lamp".into()]);
        let taken = batch.take("p", 1).expect("batch");
        assert!(taken.upserts.is_empty());
        assert_eq!(taken.deletes, ["home/lamp"]);
        assert_eq!(taken.epoch, 1);

        batch.merge(vec![], vec!["home/lamp".into()]);
        batch.merge(vec![dto("home/lamp")], vec![]);
        let taken = batch.take("p", 2).expect("batch");
        assert!(taken.deletes.is_empty());
        assert_eq!(taken.upserts.len(), 1);
        assert_eq!(taken.upserts[0].path, "home/lamp");
    }

    #[tokio::test]
    async fn stop_during_backoff_returns_promptly_and_closes_events() {
        let ui = UiConfig::default();
        let (handle, mut events) =
            LiveHandle::spawn(unreachable_profile("old"), None, &ui).unwrap();
        let first_epoch = handle.epoch();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_retry = false;
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                Ok(Some(SessionEvent::Status(status)))
                    if status.status == crate::StatusKind::Reconnecting
                        || status.status == crate::StatusKind::Error =>
                {
                    saw_retry = true;
                    break;
                }
                Ok(Some(_)) | Ok(None) | Err(_) => {}
            }
        }
        assert!(saw_retry, "expected reconnect/error after refused broker");

        let started = Instant::now();
        handle.stop().await;
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "stop must not wait out the 500ms+ backoff"
        );

        let mut last = None;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(200), events.recv()).await
        {
            last = Some(event);
        }
        if let Some(SessionEvent::Status(status)) = last {
            assert_eq!(status.epoch, first_epoch);
            assert!(
                status.status == crate::StatusKind::Disconnected
                    || status.status == crate::StatusKind::Error
            );
        }
        assert!(events.recv().await.is_none());

        let (handle2, mut events2) =
            LiveHandle::spawn(unreachable_profile("new"), None, &ui).unwrap();
        assert_ne!(handle2.epoch(), first_epoch);
        let second = tokio::time::timeout(Duration::from_secs(1), events2.recv())
            .await
            .ok()
            .flatten()
            .expect("second session event");
        assert_eq!(second.epoch(), handle2.epoch());
        assert_ne!(second.epoch(), first_epoch);
        handle2.stop().await;
    }
}

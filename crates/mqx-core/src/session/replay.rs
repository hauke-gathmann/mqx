use std::time::Duration;

use rumqttc::AsyncClient;
use serde::Serialize;
use tokio::{
    sync::mpsc,
    time::{Instant, sleep},
};
use tracing::info;

use super::{Session, SessionEvent, tls::qos_from_u8};
use crate::record::{RecordEvent, wait_duration};

const PROGRESS_EVERY: Duration = Duration::from_millis(100);
const PROGRESS_FRAMES: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Stopped,
    Ended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackProgress {
    pub file: String,
    pub index: u64,
    pub total: u64,
    pub t_ms: u64,
    pub t_end_ms: u64,
    pub state: PlaybackState,
    #[serde(default)]
    pub epoch: u64,
    #[serde(default)]
    pub generation: u64,
}

pub struct ReplayJob {
    pub client: AsyncClient,
    pub events: Vec<RecordEvent>,
    pub file: String,
    pub epoch: u64,
    pub generation: u64,
    pub events_tx: mpsc::UnboundedSender<SessionEvent>,
}

/// Replay target is the open session. Does not change ingest / Live vs Detached.
pub fn assert_replay_target(session: &Session, requested: Option<&str>) -> Result<(), String> {
    if let Some(id) = requested.map(str::trim).filter(|id| !id.is_empty())
        && id != session.id
    {
        return Err("already connected to a different profile".into());
    }
    Ok(())
}

pub async fn run_replay(job: ReplayJob) {
    let ReplayJob {
        client,
        events,
        file,
        epoch,
        generation,
        events_tx,
    } = job;
    if events.is_empty() {
        return;
    }

    let total = events.len() as u64;
    let t0 = events[0].t_ms;
    let t_end_ms = events.last().map(|event| event.t_ms).unwrap_or(t0);
    info!(file = %file, total, "replay start");

    let mut guard = StopOnDrop {
        file: file.clone(),
        index: 0,
        total,
        t_ms: t0,
        t_end_ms,
        epoch,
        generation,
        tx: events_tx.clone(),
        done: false,
    };
    emit(
        &events_tx,
        &file,
        0,
        total,
        t0,
        t_end_ms,
        PlaybackState::Playing,
        epoch,
        generation,
    );

    let start = Instant::now();
    let mut last_progress = Instant::now();
    let mut since_progress = 0u32;

    for (i, event) in events.iter().enumerate() {
        sleep(wait_duration(t0, event.t_ms, 1.0, start.elapsed())).await;
        let payload = match event.payload_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "replay payload");
                break;
            }
        };
        if let Err(error) = client
            .publish(&event.topic, qos_from_u8(event.qos), event.retain, payload)
            .await
        {
            tracing::warn!(%error, topic = %event.topic, "replay publish");
            break;
        }

        let index = (i as u64).saturating_add(1);
        guard.index = index;
        guard.t_ms = event.t_ms;
        since_progress = since_progress.saturating_add(1);
        let due = since_progress >= PROGRESS_FRAMES
            || last_progress.elapsed() >= PROGRESS_EVERY
            || index == total;
        if due {
            emit(
                &events_tx,
                &file,
                index,
                total,
                event.t_ms,
                t_end_ms,
                PlaybackState::Playing,
                epoch,
                generation,
            );
            last_progress = Instant::now();
            since_progress = 0;
        }
    }

    if guard.index == total {
        info!(file = %file, total, "replay ended");
        emit(
            &events_tx,
            &file,
            total,
            total,
            t_end_ms,
            t_end_ms,
            PlaybackState::Ended,
            epoch,
            generation,
        );
        guard.done = true;
    } else {
        info!(file = %file, index = guard.index, total, "replay stopped");
    }
}

struct StopOnDrop {
    file: String,
    index: u64,
    total: u64,
    t_ms: u64,
    t_end_ms: u64,
    epoch: u64,
    generation: u64,
    tx: mpsc::UnboundedSender<SessionEvent>,
    done: bool,
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        emit(
            &self.tx,
            &self.file,
            self.index,
            self.total,
            self.t_ms,
            self.t_end_ms,
            PlaybackState::Stopped,
            self.epoch,
            self.generation,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit(
    tx: &mpsc::UnboundedSender<SessionEvent>,
    file: &str,
    index: u64,
    total: u64,
    t_ms: u64,
    t_end_ms: u64,
    state: PlaybackState,
    epoch: u64,
    generation: u64,
) {
    let _ = tx.send(SessionEvent::Playback(PlaybackProgress {
        file: file.to_string(),
        index,
        total,
        t_ms,
        t_end_ms,
        state,
        epoch,
        generation,
    }));
}

#[cfg(test)]
mod tests {
    use rumqttc::MqttOptions;
    use tokio::time::Duration;

    use super::*;
    use crate::{
        config::UiConfig,
        profiles::{ConnectionProfile, Protocol, SessionConfig, Subscription, TlsConfig},
        session::{Status, StatusKind},
    };

    fn event(t_ms: u64, topic: &str) -> RecordEvent {
        RecordEvent {
            t_ms,
            topic: topic.into(),
            qos: 0,
            retain: false,
            dup: false,
            payload: String::new(),
        }
    }

    fn profile(id: &str) -> ConnectionProfile {
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
            subscriptions: vec![Subscription {
                topic: "#".into(),
                qos: 0,
            }],
            last_will: None,
            websocket_path: None,
        }
    }

    fn session() -> crate::Session {
        let (client, _eventloop) = AsyncClient::new(MqttOptions::new("t", "localhost", 1883), 10);
        crate::Session::new(&profile("profile-1"), client, &UiConfig::default(), 1)
    }

    #[test]
    fn start_replay_while_connected_to_another_profile_errors() {
        let session = session();
        let err = assert_replay_target(&session, Some("other")).unwrap_err();
        assert!(err.contains("different profile"), "{err}");
        assert_replay_target(&session, Some("profile-1")).unwrap();
        assert_replay_target(&session, None).unwrap();
    }

    #[test]
    fn start_replay_while_detached_does_not_set_ingest() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.set_ingest(false);
        assert!(matches!(session.status, Status::Detached { .. }));
        assert!(!session.ingest_enabled);

        assert_replay_target(&session, Some("profile-1")).unwrap();

        assert!(!session.ingest_enabled);
        assert!(matches!(session.status, Status::Detached { .. }));
        assert_eq!(session.status.kind(), StatusKind::Detached);
    }

    #[tokio::test]
    async fn start_replay_while_detached_leaves_handle_detached() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.set_ingest(false);
        assert_replay_target(&session, Some("profile-1")).unwrap();

        let (client, mut eventloop) =
            AsyncClient::new(MqttOptions::new("replay-detached", "127.0.0.1", 1), 10);
        let pump = tokio::spawn(async move {
            loop {
                if eventloop.poll().await.is_err() {
                    sleep(Duration::from_millis(20)).await;
                }
            }
        });

        let (tx, _rx) = mpsc::unbounded_channel();
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            run_replay(ReplayJob {
                client,
                events: vec![event(1, "t")],
                file: "lamp.jsonl".into(),
                epoch: 1,
                generation: 1,
                events_tx: tx,
            }),
        )
        .await;

        assert!(!session.ingest_enabled);
        assert_eq!(session.status.kind(), StatusKind::Detached);
        pump.abort();
    }

    #[tokio::test]
    async fn abort_mid_replay_emits_stopped() {
        let (client, mut eventloop) =
            AsyncClient::new(MqttOptions::new("replay-abort", "127.0.0.1", 1), 10);
        let pump = tokio::spawn(async move {
            loop {
                if eventloop.poll().await.is_err() {
                    sleep(Duration::from_millis(20)).await;
                }
            }
        });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_replay(ReplayJob {
            client,
            events: vec![event(0, "a"), event(10_000, "b")],
            file: "lamp.jsonl".into(),
            epoch: 7,
            generation: 3,
            events_tx: tx,
        }));

        let playing = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Some(SessionEvent::Playback(progress))
                        if progress.state == PlaybackState::Playing =>
                    {
                        return progress;
                    }
                    Some(_) => {}
                    None => panic!("progress channel closed"),
                }
            }
        })
        .await
        .expect("playing progress");
        assert_eq!(playing.epoch, 7);
        assert!(playing.index < playing.total);

        task.abort();
        let stopped = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Some(SessionEvent::Playback(progress))
                        if progress.state == PlaybackState::Stopped =>
                    {
                        return progress;
                    }
                    Some(_) => {}
                    None => panic!("progress channel closed before stopped"),
                }
            }
        })
        .await
        .expect("stopped progress");
        assert_eq!(stopped.state, PlaybackState::Stopped);
        assert_eq!(stopped.epoch, 7);
        assert!(stopped.index < stopped.total);

        pump.abort();
    }
}

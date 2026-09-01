use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mqx_core::{
    ConnectionProfile, HistoryItemDto, HistoryMeta, JqApplyResult, JqHistory, LiveHandle,
    MessageDto, ProfileStore, ProfileSummary, RECORDING_KIND, RecordStatus, RecordingContext,
    RecordingHeader, RecordingInfo, ReplayJob, SearchHitDto, SearchMode, SessionEvent,
    SessionStatus, Status, StoppedRecording, TreeBatch, TreeNodeDto, assert_replay_target,
    clamp_ram_limit_bytes, list_recordings as scan_recordings, load_replay_events, run_replay,
    unix_ms_to_rfc3339,
};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, State,
    ipc::{InvokeBody, Request},
};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

fn err(e: impl ToString) -> String {
    e.to_string()
}

fn locked_store(state: &AppState) -> Result<MutexGuard<'_, ProfileStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "profile store lock poisoned".into())
}

fn locked_live(state: &AppState) -> Result<MutexGuard<'_, Option<LiveHandle>>, String> {
    state
        .live
        .lock()
        .map_err(|_| "session lock poisoned".into())
}

fn with_session<R>(
    state: &AppState,
    f: impl FnOnce(&mqx_core::Session) -> Result<R, String>,
) -> Result<R, String> {
    let live = locked_live(state)?;
    let handle = live
        .as_ref()
        .ok_or_else(|| "no active session".to_string())?;
    let session = handle
        .session()
        .map_err(|_| "session lock poisoned".to_string())?;
    f(&session)
}

fn locked_jq(state: &AppState) -> Result<MutexGuard<'_, JqHistory>, String> {
    state
        .jq
        .lock()
        .map_err(|_| "jq history lock poisoned".into())
}

fn locked_config(state: &AppState) -> Result<MutexGuard<'_, mqx_core::AppConfig>, String> {
    state
        .config
        .lock()
        .map_err(|_| "config lock poisoned".into())
}

fn locked_pending(state: &AppState) -> Result<MutexGuard<'_, Option<PendingRecording>>, String> {
    state
        .pending_recording
        .lock()
        .map_err(|_| "pending recording lock poisoned".into())
}

#[derive(Clone, Debug)]
pub struct PendingRecording {
    pub temp_path: PathBuf,
    pub messages: u64,
    pub topics: u64,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub profile_id: String,
    pub profile_name: String,
    pub broker: String,
}

impl PendingRecording {
    fn from_stopped(stopped: &StoppedRecording, ctx: &RecordingContext) -> Self {
        Self {
            temp_path: stopped.temp_path.clone(),
            messages: stopped.messages,
            topics: stopped.topics,
            started_ms: stopped.started_ms,
            ended_ms: stopped.ended_ms,
            profile_id: ctx.profile_id.clone(),
            profile_name: ctx.profile_name.clone(),
            broker: ctx.broker.clone(),
        }
    }

    fn matches_path(&self, temp_path: &str) -> bool {
        self.temp_path.as_path() == std::path::Path::new(temp_path)
    }
}

fn locked_replay(
    state: &AppState,
) -> Result<MutexGuard<'_, Option<tauri::async_runtime::JoinHandle<()>>>, String> {
    state
        .replay
        .lock()
        .map_err(|_| "replay lock poisoned".into())
}

fn locked_scans(state: &AppState) -> Result<MutexGuard<'_, mqx_core::RecordingScanCache>, String> {
    state
        .recording_scans
        .lock()
        .map_err(|_| "recording scan cache lock poisoned".into())
}

async fn abort_replay(state: &AppState) -> Result<(), String> {
    let job = locked_replay(state)?.take();
    if let Some(job) = job {
        job.abort();
        let _ = job.await;
    }
    Ok(())
}

fn normalize_theme(theme: &str) -> Result<String, String> {
    match theme {
        "dark" | "light" | "system" => Ok(theme.to_string()),
        other => Err(format!("invalid theme {other:?}")),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    theme: String,
    ram_limit_bytes: u64,
    record_directory: String,
}

fn ui_settings(config: &mqx_core::AppConfig) -> Result<UiSettings, String> {
    Ok(UiSettings {
        theme: normalize_theme(&config.ui.theme).unwrap_or_else(|_| "dark".into()),
        ram_limit_bytes: config.ui.ram_limit_bytes,
        record_directory: config
            .recordings_dir()
            .map_err(err)?
            .to_string_lossy()
            .into_owned(),
    })
}

fn recordings_dir(state: &AppState) -> Result<PathBuf, String> {
    locked_config(state)?.recordings_dir().map_err(err)
}

pub fn persist_theme(state: &AppState, theme: &str) -> Result<String, String> {
    let theme = normalize_theme(theme)?;
    let mut config = locked_config(state)?;
    if config.ui.theme != theme {
        config.ui.theme.clone_from(&theme);
        config.save().map_err(err)?;
    }
    state.theme_menu.select(&theme).map_err(err)?;
    Ok(theme)
}

#[tauri::command(rename = "getSettings")]
pub fn get_settings(state: State<AppState>) -> Result<UiSettings, String> {
    let config = locked_config(&state)?;
    ui_settings(&config)
}

#[tauri::command(rename = "checkForUpdates")]
pub async fn check_for_updates(app: AppHandle) {
    crate::updater::check_and_prompt(app).await;
}

#[tauri::command(rename = "setTheme")]
pub fn set_theme(state: State<AppState>, theme: String) -> Result<UiSettings, String> {
    persist_theme(&state, &theme)?;
    let config = locked_config(&state)?;
    ui_settings(&config)
}

#[tauri::command(rename = "setRamLimit")]
pub fn set_ram_limit(
    app: AppHandle,
    state: State<AppState>,
    bytes: u64,
) -> Result<UiSettings, String> {
    let bytes = clamp_ram_limit_bytes(bytes);
    {
        let mut config = locked_config(&state)?;
        if config.ui.ram_limit_bytes != bytes {
            config.ui.ram_limit_bytes = bytes;
            config.save().map_err(err)?;
        }
    }
    if let Ok(live) = locked_live(&state)
        && let Some(handle) = live.as_ref()
        && let Ok(mut session) = handle.session()
    {
        let upserts = session.set_ram_limit(bytes);
        let _ = app.emit("session/status", session.status_event());
        let _ = app.emit("session/stats", session.stats_snapshot());
        if !upserts.is_empty() {
            let _ = app.emit(
                "tree/batch",
                TreeBatch {
                    profile_id: session.id.clone(),
                    epoch: session.epoch(),
                    upserts,
                    deletes: Vec::new(),
                },
            );
        }
    }
    let config = locked_config(&state)?;
    ui_settings(&config)
}

fn missing_payload_err(session: &mqx_core::Session, topic: &str, index: Option<usize>) -> String {
    match index {
        Some(seq) if session.has_leaf(topic) => format!("message {seq} not found on {topic}"),
        _ => format!("topic {topic} not found"),
    }
}

#[tauri::command(rename = "setRecordDirectory")]
pub fn set_record_directory(
    state: State<AppState>,
    directory: String,
) -> Result<UiSettings, String> {
    let mut config = locked_config(&state)?;
    let trimmed = directory.trim();
    config.record.directory = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    config.save().map_err(err)?;
    ui_settings(&config)
}

fn with_session_mut<R>(
    state: &AppState,
    f: impl FnOnce(&mut mqx_core::Session) -> Result<R, String>,
) -> Result<R, String> {
    let live = locked_live(state)?;
    let handle = live
        .as_ref()
        .ok_or_else(|| "no active session".to_string())?;
    let mut session = handle
        .session()
        .map_err(|_| "session lock poisoned".to_string())?;
    f(&mut session)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    #[serde(flatten)]
    profile: ConnectionProfile,
    has_password: bool,
}

#[derive(Serialize)]
pub struct IdDto {
    id: String,
}

#[derive(Serialize)]
pub struct PathDto {
    path: String,
}

#[derive(Serialize)]
pub struct StatusReply {
    status: &'static str,
    epoch: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReply {
    status: &'static str,
    epoch: u64,
    generation: u64,
    file: String,
    total: u64,
    t_ms: u64,
    t_end_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveProfileRequest {
    #[serde(flatten)]
    profile: ConnectionProfile,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Ca,
    Cert,
    Key,
}

#[tauri::command(rename = "listProfiles")]
pub fn list_profiles(state: State<AppState>) -> Result<Vec<ProfileSummary>, String> {
    locked_store(&state)?.list().map_err(err)
}

#[tauri::command(rename = "getProfile")]
pub fn get_profile(state: State<AppState>, id: String) -> Result<ProfileDto, String> {
    match locked_store(&state)?.get(&id).map_err(err)? {
        Some(view) => Ok(ProfileDto {
            profile: view.profile,
            has_password: view.has_password,
        }),
        None => Err(mqx_core::Error::ProfileNotFound(id).to_string()),
    }
}

#[tauri::command(rename = "saveProfile")]
pub fn save_profile(state: State<AppState>, request: Request<'_>) -> Result<IdDto, String> {
    let value = match request.body() {
        InvokeBody::Json(value) => value.clone(),
        InvokeBody::Raw(_) => return Err("expected JSON body".into()),
    };
    let SaveProfileRequest { profile, password } = serde_json::from_value(value).map_err(err)?;
    let id = locked_store(&state)?.save(profile, password).map_err(err)?;
    Ok(IdDto { id })
}

#[tauri::command(rename = "deleteProfile")]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<&'static str, String> {
    locked_store(&state)?.delete(&id).map_err(err)?;
    Ok("ok")
}

#[tauri::command(rename = "pickFile")]
pub fn pick_file(app: AppHandle, kind: FileKind) -> Result<PathDto, String> {
    let (title, filter_name, extensions) = match kind {
        FileKind::Ca => (
            "Choose CA certificate",
            "Certificates",
            &["pem", "crt", "cer", "der"][..],
        ),
        FileKind::Cert => (
            "Choose client certificate",
            "Certificates",
            &["pem", "crt", "cer", "der"][..],
        ),
        FileKind::Key => ("Choose client key", "Keys", &["pem", "key", "der"][..]),
    };

    let file = app
        .dialog()
        .file()
        .set_title(title)
        .add_filter(filter_name, extensions)
        .add_filter("All files", &["*"])
        .blocking_pick_file()
        .ok_or_else(|| "cancelled".to_string())?;

    let path = file
        .simplified()
        .into_path()
        .map_err(err)?
        .to_string_lossy()
        .into_owned();
    Ok(PathDto { path })
}

#[tauri::command(rename = "pickFolder")]
pub fn pick_folder(app: AppHandle) -> Result<PathDto, String> {
    let folder = app
        .dialog()
        .file()
        .set_title("Choose recordings folder")
        .blocking_pick_folder()
        .ok_or_else(|| "cancelled".to_string())?;

    let path = folder
        .simplified()
        .into_path()
        .map_err(err)?
        .to_string_lossy()
        .into_owned();
    Ok(PathDto { path })
}

#[tauri::command(rename = "connect")]
pub async fn connect(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<StatusReply, String> {
    let (profile, password) = {
        let store = locked_store(&state)?;
        let view = store
            .get(&id)
            .map_err(err)?
            .ok_or_else(|| mqx_core::Error::ProfileNotFound(id.clone()).to_string())?;
        let password = store.secret(&id).map_err(err)?;
        (view.profile, password)
    };

    let _replace = state.replace.lock().await;
    // Drop old forwarder events before abort/join so a late Stopped cannot
    // apply after the UI has already gone idle (or onto a new session).
    state.current_epoch.store(0, Ordering::SeqCst);
    abort_replay(&state).await?;
    let previous = locked_live(&state)?.take();
    if let Some(previous) = previous {
        finish_handle_recording(&app, &state, previous).await?;
    }

    let ui = locked_config(&state)?.ui.clone();
    let (handle, mut events) = match LiveHandle::spawn(profile, password, &ui) {
        Ok(spawned) => spawned,
        Err(error) => {
            state.current_epoch.store(0, Ordering::SeqCst);
            return Err(err(error));
        }
    };
    let epoch = handle.epoch();
    state.current_epoch.store(epoch, Ordering::SeqCst);
    *locked_live(&state)? = Some(handle);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let Some(state) = app.try_state::<AppState>() else {
                break;
            };
            if state.current_epoch.load(Ordering::SeqCst) != event.epoch() {
                continue;
            }
            let _ = match event {
                SessionEvent::Status(payload) => app.emit("session/status", payload),
                SessionEvent::Stats(payload) => app.emit("session/stats", payload),
                SessionEvent::TreeBatch(payload) => app.emit("tree/batch", payload),
                SessionEvent::TopicMessage(payload) => app.emit("topic/message", payload),
                SessionEvent::Record(payload) => {
                    if !payload.active {
                        let _ = stash_stopped_from_session(&state);
                    }
                    app.emit("record/status", payload)
                }
                SessionEvent::Playback(payload) => app.emit("playback/progress", payload),
            };
        }
    });

    Ok(StatusReply {
        status: "connecting",
        epoch,
    })
}

#[tauri::command(rename = "disconnect")]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<StatusReply, String> {
    let _replace = state.replace.lock().await;
    state.current_epoch.store(0, Ordering::SeqCst);
    abort_replay(&state).await?;
    let handle = locked_live(&state)?.take();
    if let Some(handle) = handle {
        finish_handle_recording(&app, &state, handle).await?;
    }
    let _ = app.emit("session/status", SessionStatus::idle());
    Ok(StatusReply {
        status: "disconnected",
        epoch: 0,
    })
}

async fn finish_handle_recording(
    app: &AppHandle,
    state: &AppState,
    handle: LiveHandle,
) -> Result<(), String> {
    if let Some((stopped, ctx)) = handle.stop_with_recording().await? {
        stash_pending(state, &stopped, &ctx)?;
        let _ = app.emit("record/status", stopped.status(ctx.epoch));
    }
    Ok(())
}

fn stash_pending(
    state: &AppState,
    stopped: &StoppedRecording,
    ctx: &RecordingContext,
) -> Result<(), String> {
    *locked_pending(state)? = Some(PendingRecording::from_stopped(stopped, ctx));
    Ok(())
}

fn stash_stopped_from_session(state: &AppState) -> Result<(), String> {
    let pending = {
        let live = locked_live(state)?;
        let Some(handle) = live.as_ref() else {
            return Ok(());
        };
        let mut session = handle
            .session()
            .map_err(|_| "session lock poisoned".to_string())?;
        let Some(stopped) = session.stopped_recording() else {
            return Ok(());
        };
        if !stopped.temp_path.exists() {
            session.take_stopped_recording();
            return Ok(());
        }
        PendingRecording::from_stopped(stopped, &session.recording_context())
    };
    *locked_pending(state)? = Some(pending);
    Ok(())
}

fn pending_for_save(state: &AppState, temp_path: &str) -> Result<PendingRecording, String> {
    {
        let pending = locked_pending(state)?;
        if let Some(current) = pending.as_ref() {
            if current.matches_path(temp_path) {
                return Ok(current.clone());
            }
            return Err("recording does not match".into());
        }
    }
    let recovered = {
        let live = locked_live(state)?;
        let handle = live
            .as_ref()
            .ok_or_else(|| "no recording to save".to_string())?;
        handle
            .clone_stopped_matching(std::path::Path::new(temp_path))?
            .ok_or_else(|| "no recording to save".to_string())?
    };
    stash_pending(state, &recovered.0, &recovered.1)?;
    Ok(PendingRecording::from_stopped(&recovered.0, &recovered.1))
}

pub fn should_defer_exit(state: &AppState) -> bool {
    if locked_pending(state)
        .map(|pending| pending.is_some())
        .unwrap_or(false)
    {
        return true;
    }
    locked_live(state)
        .ok()
        .and_then(|live| {
            live.as_ref()
                .map(|handle| handle.has_unsaved_recording().ok())
        })
        .flatten()
        .unwrap_or(false)
}

#[tauri::command(rename = "exitApp")]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}

fn apply_start_recording(
    live: Option<&LiveHandle>,
    directory: PathBuf,
) -> Result<RecordStatus, String> {
    let handle = live.ok_or_else(|| "no active session".to_string())?;
    handle.start_recording(directory)
}

#[tauri::command(rename = "startRecording")]
pub fn start_recording(state: State<AppState>) -> Result<RecordStatus, String> {
    let directory = recordings_dir(&state)?;
    let live = locked_live(&state)?;
    apply_start_recording(live.as_ref(), directory)
}

#[tauri::command(rename = "stopRecording")]
pub fn stop_recording(state: State<AppState>) -> Result<StoppedRecording, String> {
    let taken = {
        let live = locked_live(&state)?;
        let handle = live
            .as_ref()
            .ok_or_else(|| "no active session".to_string())?;
        handle.take_recorder()?
    };
    let (stopped, ctx) = if let Some((recorder, ctx)) = taken {
        (recorder.stop().map_err(err)?, ctx)
    } else {
        let live = locked_live(&state)?;
        let handle = live
            .as_ref()
            .ok_or_else(|| "no active session".to_string())?;
        handle
            .take_stopped_recording()?
            .ok_or_else(|| "not recording".to_string())?
    };
    stash_pending(&state, &stopped, &ctx)?;
    let status = stopped.status(ctx.epoch);
    if let Ok(live) = locked_live(&state)
        && let Some(handle) = live.as_ref()
    {
        handle.emit_record(status);
    }
    Ok(stopped)
}

#[tauri::command(rename = "saveRecording")]
pub fn save_recording(
    app: AppHandle,
    state: State<AppState>,
    temp_path: String,
    name: String,
) -> Result<PathDto, String> {
    mqx_core::validate_recording_name(&name).map_err(err)?;
    let current = pending_for_save(&state, &temp_path)?;
    let directory = recordings_dir(&state)?;
    let header = RecordingHeader {
        kind: RECORDING_KIND.into(),
        v: 1,
        started_at: unix_ms_to_rfc3339(current.started_ms),
        ended_at: unix_ms_to_rfc3339(current.ended_ms),
        profile_id: current.profile_id,
        profile_name: current.profile_name,
        broker: current.broker,
        messages: current.messages,
        topics: current.topics,
        app_version: app.package_info().version.to_string(),
    };
    let dest =
        mqx_core::save_recording(&current.temp_path, &directory, &name, &header).map_err(err)?;
    if let Ok(mut pending) = locked_pending(&state)
        && pending
            .as_ref()
            .is_some_and(|item| item.matches_path(&temp_path))
    {
        *pending = None;
    }
    clear_stopped_matching(&state, &temp_path);
    Ok(PathDto {
        path: dest.to_string_lossy().into_owned(),
    })
}

#[tauri::command(rename = "discardRecording")]
pub fn discard_recording(
    state: State<AppState>,
    temp_path: String,
) -> Result<&'static str, String> {
    let path = {
        let mut pending = locked_pending(&state)?;
        let path = if let Some(current) = pending.as_ref() {
            if current.matches_path(&temp_path) {
                current.temp_path.clone()
            } else {
                PathBuf::from(&temp_path)
            }
        } else {
            PathBuf::from(&temp_path)
        };
        if pending
            .as_ref()
            .is_some_and(|current| current.matches_path(&temp_path))
        {
            *pending = None;
        }
        path
    };
    mqx_core::discard_recording(&path).map_err(err)?;
    clear_stopped_matching(&state, &temp_path);
    Ok("ok")
}

fn clear_stopped_matching(state: &AppState, temp_path: &str) {
    if let Ok(live) = locked_live(state)
        && let Some(handle) = live.as_ref()
    {
        let _ = handle.take_stopped_matching(std::path::Path::new(temp_path));
    }
}

fn apply_set_ingest(live: Option<&LiveHandle>, enabled: bool) -> Result<SessionStatus, String> {
    let handle = live.ok_or_else(|| "no active session".to_string())?;
    handle.set_ingest(enabled)
}

#[tauri::command(rename = "setIngest")]
pub fn set_ingest(state: State<AppState>, enabled: bool) -> Result<SessionStatus, String> {
    let live = locked_live(&state)?;
    apply_set_ingest(live.as_ref(), enabled)
}

fn replay_target_for_live(
    live: Option<&LiveHandle>,
    profile_id: Option<&str>,
) -> Result<(), String> {
    let handle = live.ok_or_else(|| "no active session".to_string())?;
    let session = handle
        .session()
        .map_err(|_| "session lock poisoned".to_string())?;
    assert_replay_target(&session, profile_id)
}

async fn wait_session_ready(state: &AppState, epoch: u64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        {
            let live = locked_live(state)?;
            let handle = live.as_ref().ok_or_else(|| "session closed".to_string())?;
            if handle.epoch() != epoch {
                return Err("session replaced".into());
            }
            let session = handle
                .session()
                .map_err(|_| "session lock poisoned".to_string())?;
            match &session.status {
                Status::Connected | Status::Detached { .. } => return Ok(()),
                Status::Error { msg } => return Err(msg.clone()),
                Status::Disconnected => return Err("disconnected".into()),
                Status::Connecting | Status::Reconnecting { .. } => {}
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("timed out waiting for broker connection".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingsList {
    directory: String,
    recordings: Vec<RecordingInfo>,
}

#[tauri::command(rename = "listRecordings")]
pub fn list_recordings(state: State<AppState>) -> Result<RecordingsList, String> {
    let directory = recordings_dir(&state)?;
    let recordings = {
        let mut cache = locked_scans(&state)?;
        scan_recordings(&directory, &mut cache).map_err(err)?
    };
    Ok(RecordingsList {
        directory: directory.to_string_lossy().into_owned(),
        recordings,
    })
}

#[tauri::command(rename = "startReplay")]
pub async fn start_replay(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    profile_id: Option<String>,
) -> Result<ReplayReply, String> {
    let path = PathBuf::from(path);
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "invalid recording path".to_string())?;
    let path_for_load = path.clone();
    let events = tauri::async_runtime::spawn_blocking(move || load_replay_events(&path_for_load))
        .await
        .map_err(|error| format!("replay load: {error}"))?
        .map_err(err)?;
    let total = events.len() as u64;
    let t_ms = events[0].t_ms;
    let t_end_ms = events.last().map(|event| event.t_ms).unwrap_or(t_ms);

    let connected = locked_live(&state)?.is_some();
    if connected {
        replay_target_for_live(locked_live(&state)?.as_ref(), profile_id.as_deref())?;
    } else {
        let id = profile_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "profileId is required when no session is open".to_string())?
            .to_string();
        connect(app.clone(), state.clone(), id).await?;
    }

    wait_session_ready(&state, {
        let live = locked_live(&state)?;
        let handle = live
            .as_ref()
            .ok_or_else(|| "no active session".to_string())?;
        handle.epoch()
    })
    .await?;

    let generation = state.replay_generation.fetch_add(1, Ordering::SeqCst) + 1;
    abort_replay(&state).await?;

    let (client, epoch, events_tx) = {
        let live = locked_live(&state)?;
        replay_target_for_live(live.as_ref(), profile_id.as_deref())?;
        let handle = live
            .as_ref()
            .ok_or_else(|| "no active session".to_string())?;
        (handle.client(), handle.epoch(), handle.events())
    };

    let job = tauri::async_runtime::spawn(run_replay(ReplayJob {
        client,
        events,
        file: file.clone(),
        epoch,
        generation,
        events_tx,
    }));
    *locked_replay(&state)? = Some(job);

    Ok(ReplayReply {
        status: "playing",
        epoch,
        generation,
        file,
        total,
        t_ms,
        t_end_ms,
    })
}

#[tauri::command(rename = "stopReplay")]
pub async fn stop_replay(state: State<'_, AppState>) -> Result<&'static str, String> {
    abort_replay(&state).await?;
    Ok("ok")
}

#[tauri::command(rename = "treeChildren")]
pub fn tree_children(
    state: State<AppState>,
    path: Vec<String>,
) -> Result<Vec<TreeNodeDto>, String> {
    match locked_live(&state)?.as_ref() {
        Some(handle) => {
            let session = handle
                .session()
                .map_err(|_| "session lock poisoned".to_string())?;
            Ok(session.tree_children(&path))
        }
        None => Ok(Vec::new()),
    }
}

#[tauri::command(rename = "selectTopic")]
pub fn select_topic(state: State<AppState>, topic: Option<String>) -> Result<&'static str, String> {
    with_session_mut(&state, |session| {
        session.select_topic(topic);
        Ok("ok")
    })
}

#[tauri::command(rename = "getMessage")]
pub fn get_message(
    state: State<AppState>,
    topic: String,
    index: Option<usize>,
) -> Result<MessageDto, String> {
    with_session(&state, |session| {
        session
            .get_message(&topic, index)
            .ok_or_else(|| missing_payload_err(session, &topic, index))
    })
}

#[tauri::command(rename = "getHistoryMeta")]
pub fn get_history_meta(state: State<AppState>, topic: String) -> Result<HistoryMeta, String> {
    with_session(&state, |session| {
        session
            .get_history_meta(&topic)
            .ok_or_else(|| format!("topic {topic} not found"))
    })
}

#[tauri::command(rename = "listHistory")]
pub fn list_history(state: State<AppState>, topic: String) -> Result<Vec<HistoryItemDto>, String> {
    with_session(&state, |session| {
        session
            .list_history(&topic)
            .ok_or_else(|| format!("topic {topic} not found"))
    })
}

#[tauri::command(rename = "treeSearch")]
pub fn tree_search(
    state: State<AppState>,
    query: String,
    mode: SearchMode,
) -> Result<Vec<SearchHitDto>, String> {
    match locked_live(&state)?.as_ref() {
        Some(handle) => {
            let session = handle
                .session()
                .map_err(|_| "session lock poisoned".to_string())?;
            Ok(session.tree_search(&query, mode))
        }
        None => Ok(Vec::new()),
    }
}

#[tauri::command(rename = "applyJq")]
pub fn apply_jq(
    state: State<AppState>,
    topic: String,
    index: Option<usize>,
    filter: String,
) -> Result<JqApplyResult, String> {
    let result = with_session(&state, |session| {
        session
            .apply_jq(&topic, index, &filter)
            .ok_or_else(|| missing_payload_err(session, &topic, index))
    })?;
    if result.should_commit() {
        let mut history = locked_jq(&state)?;
        history.stage(&filter);
        history.commit(&topic);
    }
    Ok(result)
}

#[tauri::command(rename = "jqHistory")]
pub fn jq_history(state: State<AppState>, topic: String) -> Result<Vec<String>, String> {
    Ok(locked_jq(&state)?.list(&topic))
}

#[cfg(test)]
mod tests {
    use super::{apply_set_ingest, apply_start_recording, replay_target_for_live};

    #[test]
    fn set_ingest_without_session_errors() {
        let err = apply_set_ingest(None, true).unwrap_err();
        assert_eq!(err, "no active session");
        let err = apply_set_ingest(None, false).unwrap_err();
        assert_eq!(err, "no active session");
    }

    #[test]
    fn start_recording_without_session_errors() {
        let err = apply_start_recording(None, std::path::PathBuf::from("/tmp")).unwrap_err();
        assert_eq!(err, "no active session");
    }

    #[test]
    fn start_replay_without_session_errors() {
        let err = replay_target_for_live(None, Some("p")).unwrap_err();
        assert_eq!(err, "no active session");
    }
}

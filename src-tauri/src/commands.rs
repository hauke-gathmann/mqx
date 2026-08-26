use std::sync::MutexGuard;

use std::sync::atomic::Ordering;

use mqx_core::{
    ConnectionProfile, HistoryItemDto, HistoryMeta, JqApplyResult, JqHistory, LiveHandle,
    MessageDto, ProfileStore, ProfileSummary, SearchHitDto, SearchMode, SessionEvent,
    SessionStatus, TreeBatch, TreeNodeDto, clamp_ram_limit_bytes,
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
}

fn ui_settings(config: &mqx_core::AppConfig) -> UiSettings {
    UiSettings {
        theme: normalize_theme(&config.ui.theme).unwrap_or_else(|_| "dark".into()),
        ram_limit_bytes: config.ui.ram_limit_bytes,
    }
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
    Ok(ui_settings(&config))
}

#[tauri::command(rename = "setTheme")]
pub fn set_theme(state: State<AppState>, theme: String) -> Result<UiSettings, String> {
    persist_theme(&state, &theme)?;
    let config = locked_config(&state)?;
    Ok(ui_settings(&config))
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
    Ok(ui_settings(&config))
}

fn missing_payload_err(session: &mqx_core::Session, topic: &str, index: Option<usize>) -> String {
    match index {
        Some(seq) if session.has_leaf(topic) => format!("message {seq} not found on {topic}"),
        _ => format!("topic {topic} not found"),
    }
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
    let previous = locked_live(&state)?.take();
    // Drop old forwarder events before join so same-profile reconnect
    // cannot apply the previous session's disconnected/stats.
    state.current_epoch.store(0, Ordering::SeqCst);
    if let Some(previous) = previous {
        previous.stop().await;
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
    let handle = locked_live(&state)?.take();
    state.current_epoch.store(0, Ordering::SeqCst);
    if let Some(handle) = handle {
        handle.stop().await;
    }
    let _ = app.emit("session/status", SessionStatus::idle());
    Ok(StatusReply {
        status: "disconnected",
        epoch: 0,
    })
}

fn apply_set_ingest(live: Option<&LiveHandle>, enabled: bool) -> Result<SessionStatus, String> {
    let handle = live.ok_or_else(|| "no active session".to_string())?;
    let mut session = handle
        .session()
        .map_err(|_| "session lock poisoned".to_string())?;
    session.set_ingest(enabled);
    Ok(session.status_event())
}

#[tauri::command(rename = "setIngest")]
pub fn set_ingest(
    app: AppHandle,
    state: State<AppState>,
    enabled: bool,
) -> Result<SessionStatus, String> {
    let live = locked_live(&state)?;
    let status = apply_set_ingest(live.as_ref(), enabled)?;
    let _ = app.emit("session/status", status.clone());
    Ok(status)
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
    use super::apply_set_ingest;

    #[test]
    fn set_ingest_without_session_errors() {
        let err = apply_set_ingest(None, true).unwrap_err();
        assert_eq!(err, "no active session");
        let err = apply_set_ingest(None, false).unwrap_err();
        assert_eq!(err, "no active session");
    }
}

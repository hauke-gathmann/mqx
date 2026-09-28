use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

pub async fn check_and_prompt(app: AppHandle) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        show(
            &app,
            "Updates",
            "Install the new mqx package using your package manager or download it from https://github.com/hauke-gathmann/mqx/releases. In-app updates are available for AppImage installations.",
            MessageDialogKind::Info,
        );
        return;
    }
    let updater = match app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(updater) => updater,
        Err(error) => {
            show(
                &app,
                "Check for Updates",
                &unsigned_message(&error.to_string()),
                MessageDialogKind::Info,
            );
            return;
        }
    };

    match updater.check().await {
        Ok(Some(mut update)) => {
            update.timeout = Some(Duration::from_secs(600));
            let notes = update.body.as_deref().unwrap_or("").trim().to_string();
            let body = if notes.is_empty() {
                format!(
                    "Version {} is available. Install this update now?",
                    update.version
                )
            } else {
                format!(
                    "Version {} is available.\n\n{notes}\n\nInstall this update now?",
                    update.version
                )
            };
            let install = message_dialog(&app, "Update available", &body, MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancel)
                .blocking_show();
            if !install {
                return;
            }
            let state = app.state::<crate::AppState>();
            // Serialize installation against connection replacement, recording starts,
            // and replay starts. No new activity can begin between this check and restart.
            let Ok(_replace) = state.replace.try_lock() else {
                show(
                    &app,
                    "Update postponed",
                    "Another operation is in progress. Try again when it finishes.",
                    MessageDialogKind::Info,
                );
                return;
            };
            let replay_active = state
                .replay
                .lock()
                .map(|job| job.as_ref().is_some_and(|job| !job.inner().is_finished()))
                .unwrap_or(true);
            if let Some(reason) =
                activity_blocker(crate::commands::should_defer_exit(&state), replay_active)
            {
                show(&app, "Update postponed", reason, MessageDialogKind::Info);
                return;
            }
            if let Err(error) = update.download_and_install(|_, _| {}, || {}).await {
                show(
                    &app,
                    "Update failed",
                    &error.to_string(),
                    MessageDialogKind::Error,
                );
                return;
            }
            app.restart();
        }
        Ok(None) => {
            show(
                &app,
                "Check for Updates",
                "You're on the latest version.",
                MessageDialogKind::Info,
            );
        }
        Err(error) => {
            show(
                &app,
                "Check for Updates",
                &unsigned_message(&error.to_string()),
                MessageDialogKind::Info,
            );
        }
    }
}

fn unsigned_message(detail: &str) -> String {
    format!(
        "Could not check for updates. Check your connection and try again, or download a release from https://github.com/hauke-gathmann/mqx/releases.\n\n{detail}"
    )
}

fn activity_blocker(unsaved: bool, replay: bool) -> Option<&'static str> {
    if unsaved {
        Some("Stop recording and save or discard the recording before installing an update.")
    } else if replay {
        Some("Stop playback before installing an update.")
    } else {
        None
    }
}

fn show(app: &AppHandle, title: &str, message: &str, kind: MessageDialogKind) {
    message_dialog(app, title, message, kind).blocking_show();
}

fn message_dialog(
    app: &AppHandle,
    title: &str,
    message: &str,
    kind: MessageDialogKind,
) -> tauri_plugin_dialog::MessageDialogBuilder<tauri::Wry> {
    let dialog = app.dialog().message(message).title(title).kind(kind);
    if let Some(window) = app.get_webview_window("main") {
        dialog.parent(&window)
    } else {
        dialog
    }
}

#[cfg(test)]
mod tests {
    use super::activity_blocker;

    #[test]
    fn update_requires_no_recording_or_playback() {
        assert!(activity_blocker(true, false).is_some());
        assert!(activity_blocker(false, true).is_some());
        assert!(activity_blocker(true, true).is_some());
        assert!(activity_blocker(false, false).is_none());
    }
}

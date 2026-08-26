use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

pub async fn check_and_prompt(app: AppHandle) {
    let updater = match app.updater() {
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
        Ok(Some(update)) => {
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
            let install = app
                .dialog()
                .message(body)
                .title("Update available")
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancel)
                .blocking_show();
            if !install {
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
        "Could not check for updates. This build may be unsigned, or no release metadata has been published yet.\n\n{detail}"
    )
}

fn show(app: &AppHandle, title: &str, message: &str, kind: MessageDialogKind) {
    app.dialog()
        .message(message)
        .title(title)
        .kind(kind)
        .blocking_show();
}

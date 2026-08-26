mod commands;
mod menu;
mod updater;

use std::sync::{Mutex, atomic::AtomicU64};

use mqx_core::{AppConfig, JqHistory, LiveHandle, ProfileStore, RecordingScanCache};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::menu::ThemeMenu;

pub struct AppState {
    store: Mutex<ProfileStore>,
    live: Mutex<Option<LiveHandle>>,
    replay: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    replay_generation: AtomicU64,
    replace: tokio::sync::Mutex<()>,
    current_epoch: AtomicU64,
    config: Mutex<AppConfig>,
    jq: Mutex<JqHistory>,
    theme_menu: ThemeMenu<tauri::Wry>,
    pending_recording: Mutex<Option<commands::PendingRecording>>,
    recording_scans: Mutex<RecordingScanCache>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let store = match ProfileStore::open() {
                Ok(store) => store,
                Err(error) => {
                    app.dialog()
                        .message(format!("Could not open the profile store:\n{error}"))
                        .title("mqx")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    return Err(Box::new(error));
                }
            };
            let config = AppConfig::load().unwrap_or_default();
            let jq = match JqHistory::load() {
                Ok(history) => history,
                Err(_) => JqHistory::load_from(std::env::temp_dir().join("mqx-history.jq"))
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            };
            let (menu, theme_menu) = menu::build(app, &config.ui.theme)?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                menu::on_event(app, event.id().as_ref());
            });
            app.manage(AppState {
                store: Mutex::new(store),
                live: Mutex::new(None),
                replay: Mutex::new(None),
                replay_generation: AtomicU64::new(0),
                replace: tokio::sync::Mutex::new(()),
                current_epoch: AtomicU64::new(0),
                config: Mutex::new(config),
                jq: Mutex::new(jq),
                theme_menu,
                pending_recording: Mutex::new(None),
                recording_scans: Mutex::new(RecordingScanCache::default()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::get_profile,
            commands::save_profile,
            commands::delete_profile,
            commands::pick_file,
            commands::connect,
            commands::disconnect,
            commands::set_ingest,
            commands::start_replay,
            commands::stop_replay,
            commands::tree_children,
            commands::select_topic,
            commands::get_message,
            commands::get_history_meta,
            commands::list_history,
            commands::tree_search,
            commands::apply_jq,
            commands::jq_history,
            commands::get_settings,
            commands::set_theme,
            commands::set_ram_limit,
            commands::start_recording,
            commands::stop_recording,
            commands::save_recording,
            commands::discard_recording,
            commands::list_recordings,
            commands::pick_folder,
            commands::set_record_directory,
            commands::exit_app,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let Some(state) = window.try_state::<AppState>() else {
                    return;
                };
                if commands::should_defer_exit(&state) {
                    api.prevent_close();
                    let _ = window.emit("app/close-requested", ());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

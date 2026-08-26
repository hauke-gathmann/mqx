mod commands;
mod menu;
mod updater;

use std::sync::{Mutex, atomic::AtomicU64};

use mqx_core::{AppConfig, JqHistory, LiveHandle, ProfileStore};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::menu::ThemeMenu;

pub struct AppState {
    store: Mutex<ProfileStore>,
    live: Mutex<Option<LiveHandle>>,
    replace: tokio::sync::Mutex<()>,
    current_epoch: AtomicU64,
    config: Mutex<AppConfig>,
    jq: Mutex<JqHistory>,
    theme_menu: ThemeMenu<tauri::Wry>,
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
                replace: tokio::sync::Mutex::new(()),
                current_epoch: AtomicU64::new(0),
                config: Mutex::new(config),
                jq: Mutex::new(jq),
                theme_menu,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

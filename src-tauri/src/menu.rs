use tauri::{
    AppHandle, Emitter, Manager, Runtime,
    menu::{
        AboutMetadata, CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder,
        PredefinedMenuItem, SubmenuBuilder,
    },
};

use crate::{AppState, commands};

pub struct ThemeMenu<R: Runtime> {
    dark: CheckMenuItem<R>,
    light: CheckMenuItem<R>,
    system: CheckMenuItem<R>,
}

impl<R: Runtime> ThemeMenu<R> {
    pub fn select(&self, theme: &str) -> tauri::Result<()> {
        self.dark.set_checked(theme == "dark")?;
        self.light.set_checked(theme == "light")?;
        self.system.set_checked(theme == "system")?;
        Ok(())
    }
}

pub fn build<R: Runtime>(
    app: &impl Manager<R>,
    theme: &str,
) -> tauri::Result<(Menu<R>, ThemeMenu<R>)> {
    let about = PredefinedMenuItem::about(
        app,
        Some("About mqx"),
        Some(AboutMetadata {
            name: Some("mqx".into()),
            version: Some(app.package_info().version.to_string()),
            comments: Some("Desktop MQTT explorer".into()),
            license: Some("MIT".into()),
            ..Default::default()
        }),
    )?;
    let settings = MenuItemBuilder::with_id("settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;

    let app_menu = SubmenuBuilder::new(app, "mqx")
        .item(&about)
        .separator()
        .item(&settings)
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let new_connection = MenuItemBuilder::with_id("new-connection", "New Connection")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let detach = MenuItemBuilder::with_id("detach", "Detach").build(app)?;
    let go_live = MenuItemBuilder::with_id("go-live", "Go Live").build(app)?;
    let disconnect = MenuItemBuilder::with_id("disconnect", "Disconnect").build(app)?;
    let connections = SubmenuBuilder::new(app, "Connections")
        .item(&new_connection)
        .separator()
        .item(&detach)
        .item(&go_live)
        .separator()
        .item(&disconnect)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let theme_menu = ThemeMenu {
        dark: CheckMenuItemBuilder::with_id("theme-dark", "Dark")
            .checked(theme == "dark")
            .build(app)?,
        light: CheckMenuItemBuilder::with_id("theme-light", "Light")
            .checked(theme == "light")
            .build(app)?,
        system: CheckMenuItemBuilder::with_id("theme-system", "System")
            .checked(theme == "system")
            .build(app)?,
    };
    let theme_submenu = SubmenuBuilder::new(app, "Theme")
        .item(&theme_menu.dark)
        .item(&theme_menu.light)
        .item(&theme_menu.system)
        .build()?;
    let search = MenuItemBuilder::with_id("search", "Search Topics")
        .accelerator("CmdOrCtrl+K")
        .build(app)?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&theme_submenu)
        .separator()
        .item(&search)
        .build()?;

    let check_updates =
        MenuItemBuilder::with_id("check-updates", "Check for Updates…").build(app)?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&check_updates)
        .build()?;

    let menu = MenuBuilder::new(app)
        .item(&app_menu)
        .item(&connections)
        .item(&edit)
        .item(&view)
        .item(&help)
        .build()?;
    Ok((menu, theme_menu))
}

pub fn on_event(app: &AppHandle, id: &str) {
    match id {
        "settings" => {
            let _ = app.emit("menu/settings", ());
        }
        "new-connection" => {
            let _ = app.emit("menu/new-connection", ());
        }
        "disconnect" => {
            let _ = app.emit("menu/disconnect", ());
        }
        "detach" => {
            let _ = app.emit("menu/set-ingest", false);
        }
        "go-live" => {
            let _ = app.emit("menu/set-ingest", true);
        }
        "search" => {
            let _ = app.emit("menu/search", ());
        }
        "theme-dark" => set_theme(app, "dark"),
        "theme-light" => set_theme(app, "light"),
        "theme-system" => set_theme(app, "system"),
        "check-updates" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::updater::check_and_prompt(handle).await;
            });
        }
        _ => {}
    }
}

fn set_theme(app: &AppHandle, theme: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match commands::persist_theme(&state, theme) {
        Ok(theme) => {
            let _ = app.emit("settings/theme", theme);
        }
        Err(error) => {
            tracing_or_dialog(app, &error);
        }
    }
}

fn tracing_or_dialog(app: &AppHandle, error: &str) {
    let _ = app.emit("settings/error", error.to_string());
}

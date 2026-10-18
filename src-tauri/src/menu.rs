//! Native menu for Windows, replacing the Electron `Menu` template.
//!
//! Differences from the macOS original:
//! - no app menu role (macOS only); the app icon menu does not exist
//! - `Ctrl` accelerators instead of `Cmd`
//! - "Open Recent" is rebuilt whenever the recent list changes
use std::path::Path;

use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Manager};

pub const MENU_OPEN: &str = "open";
pub const MENU_RECENT_PREFIX: &str = "recent-";
pub const MENU_EXIT: &str = "exit";
pub const MENU_RELOAD: &str = "reload";
pub const MENU_DEVTOOLS: &str = "devtools";
pub const MENU_CHECK_UPDATES: &str = "check-updates";

/// (Re)builds the window menu with the current recent-files list.
pub fn rebuild(app: &AppHandle) {
    let recent = app.state::<crate::state::AppState>().recent_list();
    let Ok(menu) = build_menu(app, &recent) else {
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_menu(menu);
    }
}

fn build_menu(
    app: &AppHandle,
    recent: &[std::path::PathBuf],
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let open = MenuItemBuilder::new("Open")
        .id(MENU_OPEN)
        .accelerator("Ctrl+O")
        .build(app)?;
    let exit = MenuItemBuilder::new("Exit")
        .id(MENU_EXIT)
        .accelerator("Ctrl+Q")
        .build(app)?;

    let mut recent_submenu = SubmenuBuilder::new(app, "Open Recent");
    if recent.is_empty() {
        recent_submenu = recent_submenu.item(
            &MenuItemBuilder::new("No Recent Files")
                .id("recent-empty")
                .enabled(false)
                .build(app)?,
        );
    } else {
        for (index, path) in recent.iter().enumerate() {
            recent_submenu = recent_submenu.item(
                &MenuItemBuilder::new(path_label(path))
                    .id(format!("{MENU_RECENT_PREFIX}{index}"))
                    .build(app)?,
            );
        }
    }

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&open)
        .item(&recent_submenu.build()?)
        .separator()
        .item(&exit)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .separator()
        .select_all()
        .build()?;

    let reload = MenuItemBuilder::new("Reload")
        .id(MENU_RELOAD)
        .accelerator("Ctrl+R")
        .build(app)?;
    let devtools = MenuItemBuilder::new("Toggle Developer Tools")
        .id(MENU_DEVTOOLS)
        .accelerator("F12")
        .build(app)?;
    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&reload)
        .item(&devtools)
        .build()?;

    let check_updates = MenuItemBuilder::new("Check for Updates…")
        .id(MENU_CHECK_UPDATES)
        .build(app)?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&check_updates)
        .build()?;

    MenuBuilder::new(app)
        .items(&[&file_menu, &edit_menu, &view_menu, &help_menu])
        .build()
}

fn path_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Menu event dispatch.
pub fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().0.as_str();
    match id {
        MENU_OPEN => {
            tauri::async_runtime::spawn({
                let app = app.clone();
                async move {
                    let state = app.state::<crate::state::AppState>();
                    let app_for_command = app.clone();
                    let _ = crate::commands::open_file_dialog(app_for_command, state).await;
                }
            });
        }
        MENU_EXIT => {
            app.exit(0);
        }
        MENU_RELOAD => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.reload()");
            }
        }
        MENU_DEVTOOLS => {
            // devtools APIs only exist in dev builds (Tauri gates them behind
            // the `dev` cfg, which is off in release profiles)
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_devtools_open() {
                        window.close_devtools();
                    } else {
                        window.open_devtools();
                    }
                }
            }
        }
        MENU_CHECK_UPDATES => {
            tauri::async_runtime::spawn({
                let app = app.clone();
                async move {
                    let state = app.state::<crate::state::AppState>();
                    let mut update = state.update.lock().await;
                    update.check(&app).await;
                }
            });
        }
        other => {
            if let Some(index) = other.strip_prefix(MENU_RECENT_PREFIX) {
                if let Ok(index) = index.parse::<usize>() {
                    let recent = app.state::<crate::state::AppState>().recent_list();
                    if let Some(path) = recent.get(index) {
                        let path_string = path.to_string_lossy().into_owned();
                        tauri::async_runtime::spawn({
                            let app = app.clone();
                            async move {
                                let state = app.state::<crate::state::AppState>();
                                crate::commands::open_path_internal(&app, &state, &path_string);
                            }
                        });
                    }
                }
            }
        }
    }
}

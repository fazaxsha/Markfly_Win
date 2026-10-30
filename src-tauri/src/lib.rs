//! Markfly desktop shell (Windows) — Rust backend for the Markfly markdown
//! viewer. Replaces the Electron main process with Tauri + Rust.
//!
//! Process model parity with the original app:
//! - this crate = the "main process" (window, menu, file access, watcher,
//!   clipboard, updates, drag & drop, CLI file association)
//! - the Vue 3 renderer is the frontend, loaded from `../dist`
//! - all system access flows through the `commands` module, which exposes
//!   the same API surface the original `electronAPI` bridge had
mod commands;
mod menu;
mod state;
mod update;

use tauri::{DragDropEvent, Manager, WindowEvent};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = handle
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("Markfly"));
            let _ = std::fs::create_dir_all(&data_dir);
            let version = handle.package_info().version.to_string();
            app.manage(state::AppState::new(data_dir.clone(), version));

            // Windows file association / CLI: `Markfly.exe notes.md`.
            if let Some(arg) = std::env::args()
                .skip(1)
                .find(|arg| markfly_core::fsops::is_markdown_path(arg))
            {
                let state = handle.state::<state::AppState>();
                if let Ok(normalized) = state.authorize(&arg) {
                    if let Ok(mut inner) = state.inner.lock() {
                        inner.pending.push(normalized);
                    }
                }
            }

            menu::rebuild(&handle);

            // Rate-limited update check on startup (24h window).
            let app_for_update = handle.clone();
            tauri::async_runtime::spawn(async move {
                let state = app_for_update.state::<state::AppState>();
                let mut update = state.update.lock().await;
                update.check_on_startup(&app_for_update, &data_dir).await;
            });

            Ok(())
        })
        .on_menu_event(menu::handle_menu_event)
        .on_window_event(|window, event| {
            // Native drag & drop replaces the renderer-side webUtils path
            // logic of the Electron app.
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                for path in paths {
                    let path_string = path.to_string_lossy().into_owned();
                    tauri::async_runtime::spawn({
                        let app = window.app_handle().clone();
                        async move {
                            let state = app.state::<state::AppState>();
                            commands::open_path_internal(&app, &state, &path_string);
                        }
                    });
                }
                let _ = window.set_focus();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::read_file,
            commands::save_file,
            commands::copy_text,
            commands::fetch_markdown_url,
            commands::watch_file,
            commands::get_args,
            commands::renderer_ready,
            commands::open_file_dialog,
            commands::new_file_dialog,
            commands::open_file_path,
            commands::open_external,
            commands::get_theme,
            commands::get_recent_files,
            commands::load_pref,
            commands::save_pref,
            commands::render_preview,
            commands::get_outline,
            commands::get_update_state,
            commands::update_check,
            commands::update_download,
            commands::update_open_installer,
            commands::update_open_release,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Markfly");
}

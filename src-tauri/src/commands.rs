//! Tauri command handlers — the Rust counterpart of the original
//! `window.electronAPI` bridge (see `electron/preload.cjs`).
//!
//! Every handler keeps the same security posture as the Electron main
//! process: reads only succeed for previously authorized markdown files,
//! external links are validated as safe HTTPS URLs, and preferences are
//! sanitized on both save and load.
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use markfly_core::documents;
use markfly_core::fsops;
use markfly_core::markdown;
use markfly_core::netfetch;
use markfly_core::prefs::Prefs;
use serde::Serialize;

use crate::state::AppState;

pub const EVENT_FILE_CHANGED: &str = "markfly://file-changed";
pub const EVENT_OPEN_FILE: &str = "markfly://open-file";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedMarkdown {
    pub content: String,
    pub url: String,
}

#[tauri::command]
pub async fn read_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let inner = state.inner.lock().unwrap_or_else(|err| err.into_inner());
    fsops::read_authorized_markdown(&inner.authorized, &path).map_err(|err| err.to_string())
}

/// Persists markdown content for a file the user already opened or created.
#[tauri::command]
pub async fn save_file(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<bool, String> {
    state.write_markdown(&path, &content)?;
    state.authorize_and_add_recent(&path)?;
    Ok(true)
}

#[tauri::command]
pub async fn watch_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let normalized = {
        let inner = state.inner.lock().unwrap_or_else(|err| err.into_inner());
        if !inner.authorized.has(&path) {
            return Err("File access was not authorized.".to_string());
        }
        fsops::normalize_path(&path).map_err(|err| err.to_string())?
    };

    let handle = app.clone();
    let on_change = Arc::new(move || {
        let _ = handle.emit(EVENT_FILE_CHANGED, ());
    });
    let watcher = markfly_core::watcher::watch_file(&normalized, on_change)
        .map_err(|err| err.to_string())?;

    let mut inner = state.inner.lock().unwrap_or_else(|err| err.into_inner());
    inner.watcher = Some(watcher);
    Ok(())
}

#[tauri::command]
pub async fn get_args(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let file_path = std::env::args()
        .skip(1)
        .find(|arg| fsops::is_markdown_path(arg));
    match file_path {
        Some(file_path) => Ok(Some(
            state.authorize(&file_path)?.to_string_lossy().into_owned(),
        )),
        Option::None => Ok(Option::None),
    }
}

#[tauri::command]
pub async fn renderer_ready(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let pending = {
        let mut inner = state.inner.lock().unwrap_or_else(|err| err.into_inner());
        inner.ready = true;
        std::mem::take(&mut inner.pending)
    };
    for path in pending {
        let _ = app.emit(EVENT_OPEN_FILE, path.to_string_lossy().into_owned());
    }
    Ok(())
}

#[tauri::command]
pub async fn open_file_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("Markdown", &["md", "markdown", "mdown", "markdn"])
        .blocking_pick_file();
    let Some(picked) = picked else {
        return Ok(Option::None);
    };
    let path = picked
        .into_path()
        .map_err(|_| "Only local Markdown files can be opened.".to_string())?;
    Ok(Some(state.authorize_and_add_recent(&path.to_string_lossy())?))
}

/// Creates a new markdown document through the native save dialog.
///
/// The dialog is pre-filled with `untitled.md`, a missing `.md` extension is
/// added automatically and an existing file is never truncated (it is opened
/// instead), so "New" can not destroy user content.
#[tauri::command]
pub async fn new_file_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("New Markdown File")
        .set_file_name("untitled.md")
        .add_filter("Markdown", &["md", "markdown", "mdown", "markdn"])
        .blocking_save_file();
    let Some(picked) = picked else {
        return Ok(Option::None);
    };
    let path = picked
        .into_path()
        .map_err(|_| "Only local Markdown files can be created.".to_string())?;
    let path = fsops::ensure_markdown_extension(&path.to_string_lossy())
        .map_err(|err| err.to_string())?;
    fsops::create_markdown_file(&path).map_err(|err| err.to_string())?;
    let normalized = state.authorize_and_add_recent(&path)?;
    crate::menu::rebuild(&app);
    Ok(Some(normalized))
}

#[tauri::command]
pub async fn open_file_path(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<bool, String> {
    state.authorize_and_add_recent(&path)?;
    crate::menu::rebuild(&app);
    Ok(true)
}

/// Authorizes a path coming from a native drag & drop or CLI argument and
/// queues/emits it to the renderer as `open-file`.
pub fn open_path_internal(app: &AppHandle, state: &AppState, file_path: &str) {
    let normalized = match state.authorize_and_add_recent(file_path) {
        Ok(path) => path,
        Err(_) => return,
    };
    let ready = state
        .inner
        .lock()
        .map(|inner| inner.ready)
        .unwrap_or(false);
    if ready {
        let _ = app.emit(EVENT_OPEN_FILE, normalized);
    } else if let Ok(path) = fsops::normalize_path(file_path) {
        if let Ok(mut inner) = state.inner.lock() {
            inner.pending.push(path);
        }
    }
}

#[tauri::command]
pub async fn fetch_markdown_url(url: String) -> Result<FetchedMarkdown, String> {
    netfetch::fetch_markdown_url(&url)
        .await
        .map(|fetched| FetchedMarkdown {
            content: fetched.content,
            url: fetched.url,
        })
        .map_err(|err| err.to_string())
}

/// Validates and opens an external link with the system browser.
pub fn open_external_impl(app: &AppHandle, url: &str) -> Result<(), String> {
    if !netfetch::is_safe_external_url(url) {
        return Err("Only safe HTTPS links can be opened.".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, Option::None::<&str>)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<bool, String> {
    open_external_impl(&app, &url)?;
    Ok(true)
}

#[tauri::command]
pub async fn copy_text(app: AppHandle, text: String) -> Result<bool, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|err| err.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn get_theme(app: AppHandle) -> Result<String, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable.".to_string())?;
    Ok(match window.theme() {
        Ok(tauri::Theme::Dark) => "dark",
        _ => "light",
    }
    .to_string())
}

#[tauri::command]
pub async fn get_recent_files(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state
        .recent_list()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect())
}

#[tauri::command]
pub async fn load_pref(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let prefs: Prefs = state.load_prefs();
    serde_json::to_value(&prefs).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn save_pref(state: State<'_, AppState>, obj: serde_json::Value) -> Result<(), String> {
    state.save_prefs(obj)
}

#[tauri::command]
pub async fn render_preview(markdown: String) -> Result<String, String> {
    Ok(markdown::render_preview(&markdown))
}

#[tauri::command]
pub async fn get_outline(markdown: String) -> Result<Vec<documents::Heading>, String> {
    Ok(documents::extract_outline(&markdown))
}

#[tauri::command]
pub async fn get_update_state(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let update = state.update.lock().await;
    serde_json::to_value(update.get_state()).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update::UpdateState, String> {
    let mut update = state.update.lock().await;
    Ok(update.check(&app).await)
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update::UpdateState, String> {
    let mut update = state.update.lock().await;
    Ok(update.download(&app).await)
}

#[tauri::command]
pub async fn update_open_installer(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update::UpdateState, String> {
    let mut update = state.update.lock().await;
    Ok(update.open_installer(&app))
}

#[tauri::command]
pub async fn update_open_release(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut update = state.update.lock().await;
    update.open_release(&app).await
}

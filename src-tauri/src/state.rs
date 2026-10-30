//! Application state shared with the Tauri command handlers.
//!
//! Mirrors the module-level state of the original Electron main process
//! (`electron/main.cjs`): authorized files, recent files, the active file
//! watcher, queued Finder-open paths, and the update service.
use std::path::{Path, PathBuf};

use markfly_core::fsops::AuthorizedFiles;
use markfly_core::prefs::Prefs;
use markfly_core::recent::RecentFiles;
use markfly_core::watcher::FileWatcher;
use serde_json::Value;

use crate::update::UpdateService;

pub struct AppState {
    pub data_dir: PathBuf,
    #[allow(dead_code)]
    pub current_version: String,
    pub inner: std::sync::Mutex<InnerState>,
    /// The update service uses an async mutex: commands hold the guard across
    /// `.await` (network / file IO) and Tauri requires `Send` futures.
    pub update: tokio::sync::Mutex<UpdateService>,
}

pub struct InnerState {
    pub authorized: AuthorizedFiles,
    pub recent: RecentFiles,
    pub watcher: Option<FileWatcher>,
    pub pending: Vec<PathBuf>,
    pub ready: bool,
}

impl AppState {
    pub fn new(data_dir: PathBuf, current_version: String) -> Self {
        let update_service = UpdateService::new(&current_version);
        Self {
            data_dir,
            current_version,
            inner: std::sync::Mutex::new(InnerState {
                authorized: AuthorizedFiles::new(),
                recent: RecentFiles::new(),
                watcher: Option::None,
                pending: Vec::new(),
                ready: false,
            }),
            update: tokio::sync::Mutex::new(update_service),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, InnerState> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Validates + registers a path for reading (markdown-only policy).
    pub fn authorize(&self, file_path: &str) -> Result<PathBuf, String> {
        let mut inner = self.lock();
        inner
            .authorized
            .authorize(file_path)
            .map_err(|err| err.to_string())
    }

    /// Writes markdown content for a path the user has opened or created.
    ///
    /// The allow-list is the only gate: every saveable path went through the
    /// open/new dialog (or a saved tab restored from prefs), so the renderer can
    /// never write to a file the user did not pick.
    pub fn write_markdown(&self, file_path: &str, content: &str) -> Result<(), String> {
        let inner = self.lock();
        markfly_core::fsops::write_authorized_markdown(&inner.authorized, file_path, content)
            .map_err(|err| err.to_string())
    }

    /// Authorize + add to recents; returns the normalized path string.
    pub fn authorize_and_add_recent(&self, file_path: &str) -> Result<String, String> {
        let mut inner = self.lock();
        let normalized = inner
            .authorized
            .authorize(file_path)
            .map_err(|err| err.to_string())?;
        inner.recent.add(normalized.clone());
        Ok(normalized.to_string_lossy().into_owned())
    }

    pub fn is_authorized(&self, file_path: &str) -> bool {
        self.lock().authorized.has(file_path)
    }

    pub fn recent_list(&self) -> Vec<PathBuf> {
        self.lock().recent.paths().to_vec()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Loads and sanitizes persisted preferences, authorizing saved tabs.
    pub fn load_prefs(&self) -> Prefs {
        let path = self.data_dir.join("prefs.json");
        let raw = std::fs::read(&path).unwrap_or_default();
        let value: Value =
            serde_json::from_slice(&raw).unwrap_or_else(|_| Value::Object(Default::default()));

        // Authorize saved markdown tabs that still exist on disk, mirroring
        // `authorizeSavedFiles` in the original main process.
        {
            let mut inner = self.lock();
            if let Some(tabs) = value.get("tabs").and_then(|tabs| tabs.as_array()) {
                for tab in tabs {
                    let Some(file_path) = tab
                        .get("path")
                        .and_then(|p| p.as_str())
                        .or_else(|| tab.as_str())
                    else {
                        continue;
                    };
                    if file_path.starts_with("https://") || file_path.starts_with("http://") {
                        continue;
                    }
                    if !markfly_core::fsops::is_markdown_path(file_path) {
                        continue;
                    }
                    if Path::new(file_path).is_file() {
                        let _ = inner.authorized.authorize(file_path);
                    }
                }
            }
        }

        Prefs::sanitize(&value, |file_path| self.is_authorized(file_path))
    }

    /// Sanitizes and persists preferences to `prefs.json` in the data dir.
    pub fn save_prefs(&self, value: Value) -> Result<(), String> {
        let prefs = Prefs::sanitize(&value, |file_path| self.is_authorized(file_path));
        std::fs::create_dir_all(&self.data_dir).map_err(|err| err.to_string())?;
        let json = serde_json::to_vec_pretty(&prefs).map_err(|err| err.to_string())?;
        std::fs::write(self.data_dir.join("prefs.json"), json).map_err(|err| err.to_string())
    }
}

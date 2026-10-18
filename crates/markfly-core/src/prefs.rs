//! Preferences model and sanitization, mirroring `sanitizePreferences` in
//! `electron/main.cjs`.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN, ZOOM_MAX, ZOOM_MIN};
use crate::fsops::{is_markdown_path, is_remote_url};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TabPref {
    pub path: String,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

/// Per-document view state (zoom + raw source toggle).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPref {
    pub zoom: f32,
    pub show_raw: bool,
}

impl Default for ViewPref {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            show_raw: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default)]
    pub dark: Option<bool>,
    #[serde(default)]
    pub sidebar_visible: Option<bool>,
    #[serde(default)]
    pub sidebar_width: Option<u16>,
    #[serde(default)]
    pub tabs: Vec<TabPref>,
    #[serde(default)]
    pub active_path: Option<String>,
    #[serde(default)]
    pub views: BTreeMap<String, ViewPref>,
}

impl Prefs {
    /// Parses and sanitizes a preferences payload, matching the original
    /// `sanitizePreferences`. `is_authorized` mirrors `authorizedFiles.has`
    /// (plus `isSafeExternalUrl`) filtering of saved tabs.
    pub fn sanitize(value: &serde_json::Value, is_authorized: impl Fn(&str) -> bool) -> Prefs {
        let mut result = Prefs::default();

        if let Some(dark) = value.get("dark").and_then(|v| v.as_bool()) {
            result.dark = Some(dark);
        }
        if let Some(visible) = value.get("sidebarVisible").and_then(|v| v.as_bool()) {
            result.sidebar_visible = Some(visible);
        }
        if let Some(width) = value.get("sidebarWidth").and_then(|v| v.as_u64()) {
            let width = width.clamp(SIDEBAR_WIDTH_MIN as u64, SIDEBAR_WIDTH_MAX as u64) as u16;
            result.sidebar_width = Some(width);
        }

        let mut tab_paths = Vec::new();
        if let Some(tabs) = value.get("tabs").and_then(|v| v.as_array()) {
            for tab in tabs {
                let file_path = tab
                    .get("path")
                    .and_then(|v| v.as_str())
                    .or_else(|| tab.as_str());
                let Some(file_path) = file_path else { continue };
                let allowed = is_authorized(file_path) || is_remote_url(file_path);
                if !allowed {
                    continue;
                }
                let active = tab
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| tab.is_string() || tab.get("active").is_none());
                result.tabs.push(TabPref {
                    path: file_path.to_string(),
                    active,
                });
                tab_paths.push(file_path.to_string());
            }
        }

        if let Some(active_path) = value.get("activePath").and_then(|v| v.as_str()) {
            if tab_paths.iter().any(|path| path == active_path) {
                result.active_path = Some(active_path.to_string());
            }
        }

        if let Some(views) = value.get("views").and_then(|v| v.as_object()) {
            for (file_path, view) in views {
                if !tab_paths.iter().any(|path| path == file_path) {
                    continue;
                }
                let Some(view) = view.as_object() else { continue };
                let zoom = view
                    .get("zoom")
                    .and_then(|v| v.as_f64())
                    .map(|zoom| zoom as f32)
                    .unwrap_or(1.0)
                    .clamp(ZOOM_MIN, ZOOM_MAX);
                let show_raw = view
                    .get("showRaw")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                result
                    .views
                    .insert(file_path.clone(), ViewPref { zoom, show_raw });
            }
        }

        result
    }

    /// Convenience for the save path: parse a raw JSON payload into `Prefs`
    /// keeping only structurally valid fields (paths are not re-validated).
    pub fn parse_loose(value: &serde_json::Value) -> Prefs {
        Prefs::sanitize(value, |_| true)
    }

    /// True when the saved tab entry may be re-opened on next launch:
    /// markdown files must exist, remote URLs must be http(s).
    pub fn is_restorable_tab(path: &str) -> bool {
        if is_remote_url(path) {
            return true;
        }
        is_markdown_path(path) && std::path::Path::new(path).is_file()
    }
}

//! Recent files list, mirroring `addRecent` in `electron/main.cjs`.
use std::path::PathBuf;

use crate::config::RECENT_MAX;

/// Most-recent-first list of markdown file paths, capped at `RECENT_MAX`.
#[derive(Debug, Clone, Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a path to the front of the list, removing any previous entry.
    pub fn add(&mut self, path: PathBuf) {
        self.paths.retain(|existing| existing != &path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_MAX);
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

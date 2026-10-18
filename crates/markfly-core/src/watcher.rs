//! File watching built on `notify`, replacing the chokidar watcher.
//!
//! A single watcher instance watches one markdown file; events are debounced
//! (150 ms) so editors that save via rename do not fire duplicate reloads.
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::MarkflyError;

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Watches `path` and invokes `on_change` whenever the file changes on disk.
/// Dropping the returned handle stops the watch.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
}

pub fn watch_file(
    path: &Path,
    on_change: Arc<dyn Fn() + Send + Sync + 'static>,
) -> Result<FileWatcher, MarkflyError> {
    let mut watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            let Ok(event) = result else { return };
        if !matches!(
            event.kind,
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
        ) {
            return;
        }
        // Debounce: ignore bursts of events for the same save.
        let now = Instant::now();
        let should_notify = {
            let last = LAST_EVENT.lock().unwrap();
            last.map(|previous| now.duration_since(previous) >= DEBOUNCE)
                .unwrap_or(true)
        };
        if !should_notify {
            return;
        }
        *LAST_EVENT.lock().unwrap() = Some(now);
        (on_change)();
    })
    .map_err(|err| MarkflyError::Watcher(err.to_string()))?;

    watcher
        .watch(path, RecursiveMode::NonRecursive)
        .map_err(|err| MarkflyError::Watcher(err.to_string()))?;

    Ok(FileWatcher { _watcher: watcher })
}

static LAST_EVENT: Mutex<Option<Instant>> = Mutex::new(None);

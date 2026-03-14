//! Cross-platform file system change watcher built on the `notify` crate.
//! Uses inotify on Linux, kqueue/FSEvents on macOS, ReadDirectoryChangesW on Windows.

use notify::{
    event::{AccessKind, AccessMode},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
    /// configured path → canonical path, used to resolve event paths back
    canonical: HashMap<String, String>,
}

impl FileWatcher {
    pub fn new() -> Result<Self, notify::Error> {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(tx)?;
        Ok(FileWatcher {
            watcher,
            rx,
            canonical: HashMap::new(),
        })
    }

    /// Watch `path` for write events (non-recursive).
    /// Stores the canonical path so event paths can be compared correctly.
    pub fn add_watch(&mut self, path: &str) -> Result<(), notify::Error> {
        let p = std::path::Path::new(path);
        self.watcher.watch(p, RecursiveMode::NonRecursive)?;
        // Resolve symlinks (e.g. /var → /private/var on macOS)
        let canonical = p
            .canonicalize()
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .into_owned();
        self.canonical.insert(canonical, path.to_string());
        log::debug!("watch: {}", path);
        Ok(())
    }

    /// Drain all pending events and return the configured watch paths of files that changed.
    /// Non-blocking — returns an empty Vec if nothing is pending.
    pub fn read_events(&mut self) -> Vec<String> {
        let mut paths = Vec::new();
        while let Ok(result) = self.rx.try_recv() {
            match result {
                Err(e) => log::warn!("watch error: {}", e),
                Ok(event) => {
                    log::debug!("fs event: {:?} paths={:?}", event.kind, event.paths);
                    let is_write = matches!(
                        event.kind,
                        // inotify IN_CLOSE_WRITE on Linux
                        EventKind::Access(AccessKind::Close(AccessMode::Write))
                        // content write (most platforms)
                        | EventKind::Modify(_)
                        // atomic save: editor writes to temp then renames into place
                        | EventKind::Create(_)
                    );
                    if is_write {
                        for p in &event.paths {
                            let resolved = self.resolve_event_path(p);
                            log::debug!("watch triggered: {}", resolved);
                            paths.push(resolved);
                        }
                    }
                }
            }
        }
        paths
    }

    /// Map a raw event path back to the configured (non-canonical) path prefix.
    /// This handles symlink differences like /var vs /private/var on macOS.
    fn resolve_event_path(&self, event_path: &std::path::Path) -> String {
        let event_str = event_path.to_string_lossy();

        // Try to find a canonical watch root that the event path starts with,
        // then rewrite the prefix to the original configured path.
        for (canonical_root, configured_root) in &self.canonical {
            if event_str.starts_with(canonical_root.as_str()) {
                let suffix = &event_str[canonical_root.len()..];
                return format!("{}{}", configured_root, suffix);
            }
        }

        // No match — return canonical form of the event path directly
        event_path
            .canonicalize()
            .unwrap_or_else(|_| event_path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }
}

//! Workspace filesystem watching (IDE-002).
//!
//! A recursive `notify` watcher runs on its own thread. Events are coalesced there for a
//! short quiet period and delivered as batches of changed paths, so bursts (generators,
//! `git checkout`, atomic-save renames) produce one refresh. Watch errors and event
//! overflow are reported as [`Batch::rescan`] so the IDE falls back to a full rescan.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use async_channel::{Receiver, Sender};
use notify::{EventKind, RecursiveMode, Watcher as _};

const QUIET: Duration = Duration::from_millis(120);
const MAX_WAIT: Duration = Duration::from_millis(600);
/// Generated or tool-owned folders whose churn is ignored (except Git metadata).
const IGNORED: &[&str] = &[
    "target",
    "node_modules",
    "zig-cache",
    ".zig-cache",
    "zig-out",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub paths: BTreeSet<PathBuf>,
    /// Events were lost or the watcher failed: rescan everything.
    pub rescan: bool,
    /// Git metadata changed (branch switch, commit, index update).
    pub git: bool,
}

pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
    pub batches: Receiver<Batch>,
}

enum Raw {
    Paths(Vec<PathBuf>),
    Rescan,
}

fn classify(root: &Path, path: &Path) -> Option<bool> {
    let relative = path.strip_prefix(root).ok()?;
    let mut components = relative.components().map(|component| component.as_os_str());
    let first = components.next()?;
    if first == ".git" {
        return Some(true);
    }
    let ignored = relative
        .components()
        .any(|component| IGNORED.iter().any(|name| component.as_os_str() == *name));
    (!ignored).then_some(false)
}

/// Watch `root` recursively. Returns an error if the watcher cannot be created.
pub fn watch(root: &Path) -> Result<Watcher, String> {
    let (raw_sender, raw_receiver) = mpsc::channel::<Raw>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let message = match result {
            Ok(event) if event.need_rescan() => Raw::Rescan,
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
            Ok(event) => Raw::Paths(event.paths),
            Err(_) => Raw::Rescan,
        };
        let _ = raw_sender.send(message);
    })
    .map_err(|error| format!("Cannot watch the workspace: {error}"))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| format!("Cannot watch {}: {error}", root.display()))?;
    let (sender, batches) = async_channel::unbounded();
    let root = root.to_path_buf();
    thread::spawn(move || coalesce(root, raw_receiver, sender));
    Ok(Watcher {
        _watcher: watcher,
        batches,
    })
}

fn coalesce(root: PathBuf, raw: mpsc::Receiver<Raw>, sender: Sender<Batch>) {
    while let Ok(first) = raw.recv() {
        let mut batch = Batch::default();
        let started = Instant::now();
        let mut next = Some(first);
        while let Some(message) = next.take() {
            match message {
                Raw::Rescan => batch.rescan = true,
                Raw::Paths(paths) => {
                    for path in paths {
                        match classify(&root, &path) {
                            Some(true) => batch.git = true,
                            Some(false) => {
                                batch.paths.insert(path);
                            }
                            None => {}
                        }
                    }
                }
            }
            let remaining = MAX_WAIT.saturating_sub(started.elapsed());
            next = raw.recv_timeout(QUIET.min(remaining)).ok();
        }
        if batch.paths.is_empty() && !batch.rescan && !batch.git {
            continue;
        }
        if sender.send_blocking(batch).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_bursts_and_ignores_generated_folders() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        let watcher = watch(&root).unwrap();
        for ix in 0..20 {
            std::fs::write(root.join(format!("file{ix}.txt")), "x").unwrap();
        }
        std::fs::write(root.join("target/debug/out.o"), "x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = BTreeSet::new();
        let mut batches = 0;
        while seen.len() < 20 && Instant::now() < deadline {
            if let Ok(batch) = watcher.batches.try_recv() {
                batches += 1;
                seen.extend(batch.paths);
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
        assert_eq!(seen.len(), 20, "{seen:?}");
        assert!(batches < 20, "events are coalesced ({batches} batches)");
        assert!(
            seen.iter()
                .all(|path| !path.starts_with(root.join("target")))
        );
        assert_eq!(classify(&root, &root.join(".git/HEAD")), Some(true));
        assert_eq!(classify(&root, &root.join("node_modules/x.js")), None);
    }
}

//! Unsaved-buffer and crash recovery (IDE-015).
//!
//! Modified buffers (file-backed and untitled) are snapshotted to a recovery folder next
//! to the user settings, debounced after edits. Saving, discarding or closing a buffer
//! removes its snapshot; a clean exit clears the folder. On start, snapshots are restored
//! in tab order with their cursor and a "recovered" marker. If the file changed on disk
//! since the snapshot, both versions are kept: the file opens with its disk content and
//! the recovered text opens in a separate tab titled "<name> (recovered)", so they can
//! be compared and reconciled.

use std::{collections::HashSet, time::Duration};

use ide_core::recovery::{self, Entry};

use super::*;

const SNAPSHOT_DELAY: Duration = Duration::from_millis(1000);

#[derive(Default)]
pub(super) struct RecoveryState {
    pub(super) directory: Option<PathBuf>,
    session: String,
    written: HashSet<String>,
    timer: Option<Task<()>>,
}

impl RecoveryState {
    pub(super) fn new(directory: Option<PathBuf>) -> Self {
        let session = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_micros().to_string())
            .unwrap_or_default();
        Self {
            directory,
            session,
            written: HashSet::new(),
            timer: None,
        }
    }

    fn key(&self, id: BufferId) -> String {
        format!("{}-{id}", self.session)
    }
}

impl IdeShell {
    pub(super) fn recovery_directory(cx: &App) -> Option<PathBuf> {
        settings::path(cx).and_then(|path| path.parent().map(|parent| parent.join("recovery")))
    }

    /// Debounce a snapshot after an edit.
    pub(super) fn schedule_recovery_snapshot(&mut self, cx: &mut Context<Self>) {
        if self.recovery.directory.is_none() {
            return;
        }
        self.recovery.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SNAPSHOT_DELAY).await;
            let _ = this.update(cx, |this, cx| this.write_recovery_snapshots(cx));
        }));
    }

    /// Write snapshots of modified buffers and remove those of saved or closed ones.
    pub(super) fn write_recovery_snapshots(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = self.recovery.directory.clone() else {
            return;
        };
        let entries: Vec<Entry> = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(_, buffer)| buffer.dirty)
            .map(|(order, buffer)| {
                let cursor = buffer.editor.read(cx).cursor_position();
                Entry {
                    key: self.recovery.key(buffer.id),
                    path: buffer.document.path().map(Path::to_path_buf),
                    name: buffer
                        .title_override
                        .clone()
                        .unwrap_or_else(|| buffer.document.name()),
                    text: buffer.document.text().to_string(),
                    disk_hash: buffer.document.disk_hash(),
                    order,
                    cursor: (cursor.line, cursor.character),
                    language: buffer.language.to_string(),
                }
            })
            .collect();
        let current: HashSet<String> = entries.iter().map(|entry| entry.key.clone()).collect();
        let stale: Vec<String> = self
            .recovery
            .written
            .difference(&current)
            .cloned()
            .collect();
        self.recovery.written = current;
        cx.background_executor()
            .spawn(async move {
                for entry in &entries {
                    let _ = recovery::write(&directory, entry);
                }
                for key in stale {
                    recovery::remove(&directory, &key);
                }
            })
            .detach();
    }

    /// Clear all snapshots after a clean shutdown (nothing unsaved remains).
    pub(super) fn clear_recovery(&mut self) {
        if let Some(directory) = &self.recovery.directory
            && self.buffers.iter().all(|buffer| !buffer.dirty)
        {
            recovery::clear(directory);
            self.recovery.written.clear();
        }
    }

    /// Restore snapshots left by a previous run (after the session's files reopened).
    pub(super) fn restore_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.recovery.directory.clone() else {
            return;
        };
        let (entries, problems) = recovery::load(&directory);
        if entries.is_empty() && problems.is_empty() {
            return;
        }
        let mut restored = 0;
        let mut conflicts = Vec::new();
        for entry in &entries {
            let target = match &entry.path {
                Some(path) if path.is_file() => {
                    let index = self.buffer_for_path(path).or_else(|| {
                        Document::open(path).ok().map(|document| {
                            let buffer = self.new_buffer(document, window, cx);
                            self.buffers.push(buffer);
                            self.buffers.len() - 1
                        })
                    });
                    match index {
                        Some(ix) if Document::current_disk_hash(path) == entry.disk_hash => {
                            Some(ix)
                        }
                        _ => {
                            conflicts.push(entry.name.clone());
                            None
                        }
                    }
                }
                Some(_) => {
                    conflicts.push(entry.name.clone());
                    None
                }
                None => None,
            };
            let ix = match target {
                Some(ix) => ix,
                None => {
                    // Untitled, or a conflicting/deleted file: restore into a new tab.
                    let buffer = self.new_buffer(Document::default(), window, cx);
                    self.buffers.push(buffer);
                    let ix = self.buffers.len() - 1;
                    if entry.path.is_some() {
                        self.buffers[ix].title_override =
                            Some(format!("{} (recovered)", entry.name));
                    }
                    if let Some(language) = ide_core::language::by_id(&entry.language) {
                        self.set_buffer_language(ix, language.id, cx);
                    }
                    ix
                }
            };
            let buffer = &mut self.buffers[ix];
            buffer.document.set_text(entry.text.as_str().into());
            buffer.folds.clear();
            buffer.dirty = buffer.document.is_dirty();
            buffer.recovered = true;
            let editor = buffer.editor.clone();
            editor.update(cx, |editor, cx| {
                editor.set_value(entry.text.clone(), window, cx);
                editor.set_cursor_position(
                    Position::new(entry.cursor.0, entry.cursor.1),
                    window,
                    cx,
                );
            });
            self.sync_lsp_buffer(ix, cx);
            restored += 1;
        }
        // Remove the pristine placeholder buffer if recovered tabs were added.
        if self.buffers.len() > 1 && self.buffers[0].is_pristine() {
            self.buffers.remove(0);
            self.active = self.active.saturating_sub(1).min(self.buffers.len() - 1);
        }
        // Old records are replaced by snapshots under this session's keys.
        recovery::clear(&directory);
        self.write_recovery_snapshots(cx);
        let mut status = format!("Recovered {restored} unsaved buffer(s)");
        if !conflicts.is_empty() {
            status.push_str(&format!(
                "; {} changed on disk since the snapshot — the recovered text is in a separate tab",
                conflicts.join(", ")
            ));
        }
        if !problems.is_empty() {
            status.push_str(&format!(
                "; {} snapshot(s) could not be read",
                problems.len()
            ));
        }
        self.status = status;
        let active = self.active;
        self.activate(active, window, cx);
    }
}

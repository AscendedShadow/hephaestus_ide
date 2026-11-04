//! Keeping the explorer, search index and open buffers in sync with disk (IDE-002).
//!
//! Batches from the workspace watcher are queued and processed on the next frame. Each
//! workspace gets a new watcher generation, so batches from a previous workspace are
//! discarded. Clean buffers reload in place (keeping cursor and scroll); dirty buffers
//! keep their text and get the existing "changed on disk" conflict bar; deleted files
//! stay open, marked as deleted, and cannot be saved back silently (saving requires
//! Save As, because the saved path's disk state no longer matches).

use crate::watcher::{self, Batch};

use super::*;

#[derive(Default)]
pub(super) struct FsSync {
    generation: u64,
    watcher: Option<watcher::Watcher>,
    task: Option<Task<()>>,
    pending: Option<(u64, Batch)>,
    pub(super) error: Option<String>,
}

impl IdeShell {
    /// Start watching the current workspace root (replacing any previous watcher).
    pub(super) fn start_watching(&mut self, cx: &mut Context<Self>) {
        self.fs_sync.generation += 1;
        self.fs_sync.watcher = None;
        self.fs_sync.task = None;
        self.fs_sync.pending = None;
        self.fs_sync.error = None;
        let Some(root) = self.workspace.root().map(Path::to_path_buf) else {
            return;
        };
        let generation = self.fs_sync.generation;
        let executor = cx.background_executor().clone();
        // Setting up a recursive watch can take a while on large trees.
        self.fs_sync.task = Some(cx.spawn(async move |this, cx| {
            let watcher = executor.spawn(async move { watcher::watch(&root) }).await;
            let batches = match this.update(cx, |this, cx| {
                if this.fs_sync.generation != generation {
                    return None;
                }
                match watcher {
                    Ok(watcher) => {
                        let batches = watcher.batches.clone();
                        this.fs_sync.watcher = Some(watcher);
                        Some(batches)
                    }
                    Err(error) => {
                        this.fs_sync.error = Some(error.clone());
                        this.status =
                            format!("{error} — use Refresh Explorer after external changes");
                        cx.notify();
                        None
                    }
                }
            }) {
                Ok(Some(batches)) => batches,
                _ => return,
            };
            while let Ok(batch) = batches.recv().await {
                let updated = this.update(cx, |this, cx| {
                    if this.fs_sync.generation != generation {
                        return;
                    }
                    let merged = match this.fs_sync.pending.take() {
                        Some((pending_generation, mut pending))
                            if pending_generation == generation =>
                        {
                            pending.paths.extend(batch.paths);
                            pending.rescan |= batch.rescan;
                            pending.git |= batch.git;
                            pending
                        }
                        _ => batch,
                    };
                    this.fs_sync.pending = Some((generation, merged));
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    #[cfg(test)]
    pub(super) fn fs_sync_ready(&self) -> bool {
        self.fs_sync.watcher.is_some()
    }

    /// Process a queued batch (called from render, where a window is available).
    pub(super) fn schedule_fs_batch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fs_sync.pending.is_some() {
            cx.defer_in(window, |this, window, cx| this.process_fs_batch(window, cx));
        }
    }

    pub(super) fn process_fs_batch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((generation, batch)) = self.fs_sync.pending.take() else {
            return;
        };
        if generation != self.fs_sync.generation || self.busy {
            // Stale (previous workspace), or a save/open is in flight: retry later.
            if generation == self.fs_sync.generation {
                self.fs_sync.pending = Some((generation, batch));
            }
            return;
        }
        if batch.rescan {
            self.after_tree_change(None, cx);
            self.check_disk_changes(window, cx);
            return;
        }
        if batch.git {
            self.refresh_git(cx);
        }
        let mut reload: Vec<PathBuf> = Vec::new();
        let mut structure_changed = false;
        for path in &batch.paths {
            let exists = path.exists();
            let known = self.search_paths.binary_search(path).is_ok();
            if exists != known && !path.is_dir()
                || (path.is_dir() && !self.workspace.is_expanded(path))
            {
                structure_changed = true;
            }
            for directory in [Some(path.as_path()), path.parent()].into_iter().flatten() {
                if self.workspace.is_expanded(directory)
                    && !reload.iter().any(|known| known == directory)
                {
                    reload.push(directory.to_path_buf());
                }
            }
        }
        self.workspace.prune();
        for directory in reload {
            if directory.is_dir() {
                self.load_directory(directory, cx);
            }
        }
        self.tree_rows = self.workspace.rows();
        if structure_changed {
            self.index_workspace(cx);
        }
        if !batch.paths.is_empty() {
            self.refresh_git(cx);
        }
        let ids: Vec<BufferId> = self
            .buffers
            .iter()
            .filter(|buffer| {
                buffer
                    .document
                    .path()
                    .is_some_and(|path| batch.paths.contains(path))
            })
            .map(|buffer| buffer.id)
            .collect();
        for id in ids {
            self.sync_buffer_with_disk(id, window, cx);
        }
        cx.notify();
    }

    /// Reconcile one buffer with its file: reload when clean, flag when dirty or deleted.
    pub(super) fn sync_buffer_with_disk(
        &mut self,
        id: BufferId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let Some(path) = self.buffers[ix].document.path().map(Path::to_path_buf) else {
            return;
        };
        if !self.buffers[ix]
            .document
            .has_external_changes()
            .unwrap_or(false)
        {
            return;
        }
        if !path.exists() {
            self.buffers[ix].external_change = true;
            self.buffers[ix].deleted_on_disk = true;
            self.status = format!(
                "{} was deleted on disk — Save As to keep it",
                path.display()
            );
            return;
        }
        self.buffers[ix].deleted_on_disk = false;
        if self.buffers[ix].dirty {
            self.buffers[ix].external_change = true;
            self.status = format!("{} changed on disk — Save As or reload", path.display());
            return;
        }
        match Document::open(&path) {
            Ok(document) => {
                self.replace_document_preserving_view(ix, document, window, cx);
                self.status = format!("Reloaded {}", path.display());
            }
            Err(error) => {
                self.buffers[ix].external_change = true;
                self.status = format!("{} could not be reloaded: {error}", path.display());
            }
        }
    }

    /// Replace a buffer's document with a freshly read one, keeping the cursor position
    /// and scroll where possible.
    pub(super) fn replace_document_preserving_view(
        &mut self,
        ix: usize,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = &mut self.buffers[ix];
        let editor = buffer.editor.clone();
        let cursor = editor.read(cx).cursor_position();
        let text = document.text().to_string();
        buffer.document = document;
        buffer.folds.clear();
        buffer.dirty = false;
        buffer.external_change = false;
        buffer.deleted_on_disk = false;
        let focused = editor.read(cx).focus_handle(cx).is_focused(window);
        editor.update(cx, |editor, cx| {
            editor.set_value(text, window, cx);
            let line = cursor
                .line
                .min(editor.text().lines_len().saturating_sub(1) as u32);
            editor.set_cursor_position(Position::new(line, cursor.character), window, cx);
        });
        if !focused && ix == self.active {
            // set_cursor_position may move focus; keep it where the user was.
            self.focus_editor(window, cx);
        }
        self.sync_lsp_buffer(ix, cx);
    }

    pub(super) fn refresh_explorer(
        &mut self,
        _: &RefreshExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.after_tree_change(None, cx);
        self.check_disk_changes(window, cx);
        self.status = "Refreshed the explorer".into();
        cx.notify();
    }
}

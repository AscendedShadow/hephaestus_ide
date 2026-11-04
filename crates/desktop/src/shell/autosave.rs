//! Save All and auto-save (IDE-031).
//!
//! Auto-save never opens a Save As prompt and never moves focus: untitled buffers are
//! skipped. Saves are serialized per buffer: while a save is in flight, further requests
//! are coalesced into one follow-up save, and edits made during the save stay dirty (the
//! saved snapshot is accepted, not the live text). A failed auto-save is reported once
//! and not retried until the buffer changes again, so a read-only file or an external
//! change cannot cause a retry storm. External-change and read-only protection come from
//! `Document::save_to`, the same path as a manual save.

use std::{collections::HashSet, time::Duration};

use super::*;
use crate::preferences::{AutoSave, AutoSaveMode};

/// Per-shell auto-save bookkeeping.
#[derive(Default)]
pub(super) struct AutoSaveState {
    timers: HashMap<BufferId, Task<()>>,
    in_flight: HashSet<BufferId>,
    again: HashSet<BufferId>,
    /// Buffers whose last auto-save failed, with the text revision that failed.
    failed: HashMap<BufferId, String>,
}

impl IdeShell {
    pub(super) fn auto_save_settings(cx: &App) -> AutoSave {
        settings::current(cx).auto_save
    }

    pub(super) fn save_all_action(
        &mut self,
        _: &SaveAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let untitled: Vec<String> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.dirty && buffer.document.path().is_none())
            .map(|buffer| buffer.document.name())
            .collect();
        let named = self
            .buffers
            .iter()
            .any(|buffer| buffer.dirty && buffer.document.path().is_some());
        let note = (!untitled.is_empty()).then(|| {
            format!(
                "{} untitled buffer{} not saved — use Save As",
                untitled.len(),
                if untitled.len() == 1 { "" } else { "s" }
            )
        });
        if named {
            self.save_all_note = note;
            self.save_all(None, window, cx);
        } else {
            self.status = note.unwrap_or_else(|| "No unsaved files".into());
        }
        cx.notify();
    }

    /// Called after every edit to a buffer.
    pub(super) fn schedule_auto_save(&mut self, id: BufferId, cx: &mut Context<Self>) {
        let settings = Self::auto_save_settings(cx);
        if settings.mode != AutoSaveMode::AfterDelay {
            return;
        }
        self.auto_save.failed.remove(&id);
        let delay = Duration::from_millis(settings.delay_ms.max(50));
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| this.auto_save_buffer(id, cx));
        });
        self.auto_save.timers.insert(id, timer);
    }

    /// Save all modified file-backed buffers when focus leaves the window or the tab.
    pub(super) fn auto_save_on_focus_change(
        &mut self,
        only: Option<BufferId>,
        cx: &mut Context<Self>,
    ) {
        if Self::auto_save_settings(cx).mode != AutoSaveMode::OnFocusChange {
            return;
        }
        let ids: Vec<BufferId> = self
            .buffers
            .iter()
            .filter(|buffer| only.is_none_or(|id| id == buffer.id))
            .map(|buffer| buffer.id)
            .collect();
        for id in ids {
            self.auto_save_buffer(id, cx);
        }
    }

    /// Save one buffer in the background without prompting or moving focus.
    pub(super) fn auto_save_buffer(&mut self, id: BufferId, cx: &mut Context<Self>) {
        self.auto_save.timers.remove(&id);
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let buffer = &self.buffers[ix];
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            return;
        };
        if !buffer.dirty || buffer.external_change {
            return;
        }
        let text = buffer.document.text().to_string();
        if self.auto_save.failed.get(&id) == Some(&text) {
            return;
        }
        if !self.auto_save.in_flight.insert(id) {
            self.auto_save.again.insert(id);
            return;
        }
        let snapshot = buffer.document.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = executor
                .spawn({
                    let path = path.clone();
                    async move { snapshot.save_to(&path) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.auto_save.in_flight.remove(&id);
                let Some(ix) = this.buffer_index(id) else {
                    return;
                };
                match result {
                    Ok(saved) => {
                        let saved_text = saved.text().to_string();
                        let buffer = &mut this.buffers[ix];
                        buffer.document.accept_saved(saved);
                        buffer.dirty = buffer.document.is_dirty();
                        this.servers
                            .update(cx, |servers, _| servers.saved(&path, &saved_text));
                        this.refresh_git(cx);
                        if settings::is_settings_file(&path, cx) {
                            let root = this.workspace.root().map(Path::to_path_buf);
                            settings::reload(root.as_deref(), None, cx);
                        }
                    }
                    Err(error) => {
                        if error.kind() == std::io::ErrorKind::AlreadyExists {
                            this.buffers[ix].external_change = true;
                        }
                        this.auto_save.failed.insert(id, text);
                        this.status = format!("Auto-save failed: {}: {error}", path.display());
                    }
                }
                if this.auto_save.again.remove(&id) && this.buffers[ix].dirty {
                    this.auto_save_buffer(id, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

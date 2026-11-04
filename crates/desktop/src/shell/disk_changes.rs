//! Reacting to files changed or deleted on disk while open.

use super::*;

impl IdeShell {
    pub(super) fn check_disk_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let documents: Vec<_> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.document.path().is_some())
            .map(|buffer| (buffer.id, buffer.document.clone()))
            .collect();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let changed = executor
                .spawn(async move {
                    documents
                        .into_iter()
                        .filter_map(|(id, document)| match document.has_external_changes() {
                            Ok(true) => Some((id, document.path().unwrap().to_path_buf())),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                for (id, path) in changed {
                    let Some(ix) = this.buffer_index(id) else {
                        continue;
                    };
                    if this.buffers[ix].document.path() != Some(path.as_path()) {
                        continue;
                    }
                    if !this.buffers[ix]
                        .document
                        .has_external_changes()
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if this.buffers[ix].dirty {
                        this.buffers[ix].external_change = true;
                        this.status =
                            format!("{} changed on disk — Save As or reload", path.display());
                    } else if let Ok(document) = Document::open(&path) {
                        this.replace_document_preserving_view(ix, document, window, cx);
                        this.status = format!("Reloaded {}", path.display());
                    } else {
                        this.buffers[ix].external_change = true;
                        this.status = format!(
                            "{} changed or was deleted — Save As to keep this buffer",
                            path.display()
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn reload_file(
        &mut self,
        _: &ReloadFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            return;
        };
        match Document::open(&path) {
            Ok(document) => {
                let text = document.text().to_string();
                let buffer = &mut self.buffers[self.active];
                buffer.document = document;
                buffer.folds.clear();
                buffer
                    .editor
                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
                buffer.dirty = false;
                buffer.external_change = false;
                self.status = format!("Reloaded {}", path.display());
            }
            Err(error) => self.status = format!("Reload failed: {error}"),
        }
        cx.notify();
    }
}

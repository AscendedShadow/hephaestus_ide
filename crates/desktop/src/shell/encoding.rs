//! Encoding and line-ending conversion (IDE-029).

use ide_core::document::{Encoding, LineEnding};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;

impl IdeShell {
    fn encoding_items(&self, reopen: bool) -> Vec<PickerItem> {
        let current = self.document().encoding();
        Encoding::all()
            .into_iter()
            .map(|encoding| PickerItem {
                label: format!(
                    "{} {}",
                    if reopen { "Reopen with" } else { "Save with" },
                    encoding.label()
                ),
                detail: if encoding == current {
                    "current".into()
                } else {
                    String::new()
                },
                filter: encoding.label().to_string(),
                depth: 0,
                action: if reopen {
                    PickerAction::ReopenWithEncoding(encoding)
                } else {
                    PickerAction::SaveWithEncoding(encoding)
                },
            })
            .collect()
    }

    pub(super) fn reopen_with_encoding(
        &mut self,
        _: &ReopenWithEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.document().path().is_none() || self.show_diff {
            self.status = "Only files on disk can be reopened".into();
            cx.notify();
            return;
        }
        let items = self.encoding_items(true);
        self.show_picker(
            PickerKind::Locations,
            "Reopen with Encoding".into(),
            items,
            window,
            cx,
        );
    }

    pub(super) fn save_with_encoding(
        &mut self,
        _: &SaveWithEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff {
            return;
        }
        let items = self.encoding_items(false);
        self.show_picker(
            PickerKind::Locations,
            "Save with Encoding".into(),
            items,
            window,
            cx,
        );
    }

    pub(super) fn change_line_ending(
        &mut self,
        _: &ChangeLineEnding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff {
            return;
        }
        let current = self.document().line_ending();
        let items = [LineEnding::Lf, LineEnding::CrLf]
            .into_iter()
            .map(|ending| PickerItem {
                label: format!("Use {}", ending.label()),
                detail: if ending == current {
                    "current".into()
                } else {
                    String::new()
                },
                filter: ending.label().into(),
                depth: 0,
                action: PickerAction::SetLineEnding(ending),
            })
            .collect();
        self.show_picker(
            PickerKind::Locations,
            "Line endings".into(),
            items,
            window,
            cx,
        );
    }

    /// Convert line endings: a pending change, saved with the next save.
    pub(super) fn set_line_ending(&mut self, ending: LineEnding, cx: &mut Context<Self>) {
        let buffer = &mut self.buffers[self.active];
        buffer.document.set_line_ending(ending);
        buffer.dirty = buffer.document.is_dirty();
        self.status = format!("Line endings set to {} — save to apply", ending.label());
        cx.notify();
    }

    /// Save with another encoding, refusing (without changes) if the text cannot be
    /// represented in it.
    pub(super) fn apply_save_encoding(
        &mut self,
        encoding: Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.document().text().to_string();
        if let Err(error) = encoding.encode(&text) {
            self.status = format!("Not converted: {error}");
            cx.notify();
            return;
        }
        let buffer = &mut self.buffers[self.active];
        buffer.document.set_encoding(encoding);
        buffer.dirty = buffer.document.is_dirty();
        if buffer.document.path().is_some() {
            self.save(SaveTarget::Current, None, window, cx);
        } else {
            self.status = format!("Encoding set to {} — save to apply", encoding.label());
        }
        cx.notify();
    }

    /// Reopen the file with another encoding; unsaved changes are discarded only after an
    /// explicit confirmation. A failed decode keeps the buffer as it is.
    pub(super) fn apply_reopen_encoding(
        &mut self,
        encoding: Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.buffer().id;
        if !self.buffer().dirty {
            return self.reopen_buffer(id, encoding, window, cx);
        }
        if window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        let name = self.document().name();
        window.open_dialog(cx, move |dialog, _, _| {
            let shell = shell.clone();
            dialog
                .title("Reopen with Encoding")
                .child(format!(
                    "{name} has unsaved changes. Reopening as {} discards them.",
                    encoding.label()
                ))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Discard and Reopen"))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.reopen_buffer(id, encoding, window, cx)
                    });
                    true
                })
        });
    }

    fn reopen_buffer(
        &mut self,
        id: BufferId,
        encoding: Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let Some(path) = self.buffers[ix].document.path().map(Path::to_path_buf) else {
            return;
        };
        match Document::open_with(&path, Some(encoding)) {
            Ok(document) => {
                self.replace_document_preserving_view(ix, document, window, cx);
                self.status = format!("Reopened {} as {}", path.display(), encoding.label());
            }
            Err(error) => {
                self.status = format!("Could not reopen as {}: {error}", encoding.label())
            }
        }
        cx.notify();
    }

    /// Offer an explicit encoding when a file is not valid UTF-8/UTF-16.
    pub(super) fn offer_encodings_for(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = Encoding::all()
            .into_iter()
            .filter(|encoding| {
                matches!(
                    encoding,
                    Encoding::Legacy(_) | Encoding::Utf16Le | Encoding::Utf16Be
                )
            })
            .map(|encoding| PickerItem {
                label: format!("Open with {}", encoding.label()),
                detail: String::new(),
                filter: encoding.label().to_string(),
                depth: 0,
                action: PickerAction::OpenWithEncoding(path.clone(), encoding),
            })
            .collect();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.show_picker(
            PickerKind::Locations,
            format!("{name} is not UTF-8 — choose its encoding"),
            items,
            window,
            cx,
        );
    }

    pub(super) fn open_with_encoding(
        &mut self,
        path: PathBuf,
        encoding: Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match Document::open_with(&path, Some(encoding)) {
            Ok(document) => {
                self.open_document(document, window, cx);
                self.status = format!("Opened {} as {}", path.display(), encoding.label());
            }
            Err(error) => self.status = format!("Could not open as {}: {error}", encoding.label()),
        }
        cx.notify();
    }
}

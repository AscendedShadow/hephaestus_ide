//! Applying source-space edits to an open buffer as one undoable editor operation.
//!
//! Buffers may display a folded projection of their source (see `folding`). Programmatic
//! edits (line commands, rename, formatting, code actions, replace, completion) are
//! expressed in source offsets; this helper unfolds any fold inside the edited span, maps
//! the edits to display offsets and applies them through the editor so that undo/redo,
//! dirty tracking and LSP synchronization follow the normal typing path.

use std::ops::Range;

use super::*;

impl IdeShell {
    /// The selection of buffer `ix` in source offsets, and whether it is reversed.
    pub(super) fn source_selection(&self, ix: usize, cx: &App) -> (Range<usize>, bool) {
        let buffer = &self.buffers[ix];
        let editor = buffer.editor.read(cx);
        let display = editor.selected_byte_range();
        let reversed = editor.is_selection_reversed();
        let start = folding::display_to_source(&buffer.folds, display.start, false);
        let end = folding::display_to_source(&buffer.folds, display.end, true);
        (start..end.max(start), reversed)
    }

    /// Apply `edits` (source offsets of buffer `ix`, non-overlapping) as a single undo
    /// step, then select `selection` (source offsets in the edited text) when given.
    /// Returns `false` when the edits are invalid for the buffer's current text.
    pub(super) fn apply_source_edits(
        &mut self,
        ix: usize,
        edits: &[(Range<usize>, String)],
        selection: Option<(Range<usize>, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(buffer) = self.buffers.get_mut(ix) else {
            return false;
        };
        let source = buffer.document.text().to_string();
        if let Err(error) = ide_core::text_edit::validate(&source, edits) {
            self.status = format!("Edit rejected: {error}");
            cx.notify();
            return false;
        }
        let editor = buffer.editor.clone();
        if !edits.is_empty() {
            let span = edits
                .iter()
                .map(|(range, _)| range.start)
                .min()
                .unwrap_or(0)
                ..edits.iter().map(|(range, _)| range.end).max().unwrap_or(0);
            if let Some(toggle) = folding::unfold_range(&source, &mut buffer.folds, span.clone()) {
                editor.update(cx, |editor, cx| {
                    let text = editor.text();
                    let range = text.byte_to_utf16_idx(toggle.edit.start)
                        ..text.byte_to_utf16_idx(toggle.edit.end);
                    editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
                });
            }
            let folds = &buffer.folds;
            let display_edits: Vec<_> = edits
                .iter()
                .map(|(range, text)| {
                    (
                        folding::source_to_display(folds, range.start)
                            ..folding::source_to_display(folds, range.end),
                        text.clone(),
                    )
                })
                .collect();
            // Folds after the edited span move by the net length change.
            let delta: isize = edits
                .iter()
                .map(|(range, text)| text.len() as isize - range.len() as isize)
                .sum();
            let shifted: Vec<_> = folds
                .iter()
                .map(|fold| {
                    let mut fold = fold.clone();
                    if fold.range.start > span.end {
                        fold.range.start = fold.range.start.saturating_add_signed(delta);
                        fold.range.end = fold.range.end.saturating_add_signed(delta);
                    }
                    fold
                })
                .collect();
            let display_selection = selection.map(|(range, reversed)| {
                (
                    folding::source_to_display(&shifted, range.start)
                        ..folding::source_to_display(&shifted, range.end),
                    reversed,
                )
            });
            editor.update(cx, |editor, cx| {
                editor.apply_edits(&display_edits, display_selection, window, cx)
            });
            // Update the document and folds now (the editor's change notification arrives
            // later and then finds them already in sync), so callers such as save see the
            // edited text immediately.
            if let Ok(updated) = ide_core::text_edit::apply(&source, edits) {
                buffer.document.set_text(updated.as_str().into());
                buffer.folds = shifted;
                buffer.dirty = buffer.document.is_dirty();
                self.sync_lsp_buffer(ix, cx);
            }
        } else if let Some((range, reversed)) = selection {
            let folds = buffer.folds.clone();
            editor.update(cx, |editor, cx| {
                editor.select_byte_range(
                    folding::source_to_display(&folds, range.start)
                        ..folding::source_to_display(&folds, range.end),
                    reversed,
                    window,
                    cx,
                )
            });
        }
        if ix == self.active && !self.show_diff {
            self.focus_editor(window, cx);
        }
        cx.notify();
        true
    }
}

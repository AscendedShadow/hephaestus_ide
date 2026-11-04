//! Folding brace blocks.

use super::*;

impl IdeShell {
    pub(super) fn toggle_fold(
        &mut self,
        _: &ToggleFold,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_fold(folding::toggle, window, cx);
    }

    pub(super) fn fold_clicked_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor().read(cx);
        if editor.focus_handle(cx).is_focused(window) && editor.cursor_position().character == 0 {
            self.apply_fold(folding::toggle_line, window, cx);
        }
    }

    pub(super) fn apply_fold(
        &mut self,
        toggle: fn(&str, &mut Vec<Fold>, usize) -> Option<Toggle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || self.show_diff {
            return;
        }
        self.clear_secondary_cursors(cx);
        let editor = self.editor().clone();
        let cursor = editor.read(cx).cursor();
        let buffer = &mut self.buffers[self.active];
        let source = buffer.document.text().to_string();
        let Some(toggle) = toggle(&source, &mut buffer.folds, cursor) else {
            self.status = "No brace block here".into();
            cx.notify();
            return;
        };
        let position = position_at(&folding::projected(&source, &buffer.folds), toggle.cursor);
        editor.update(cx, |editor, cx| {
            let text = editor.text();
            let range =
                text.byte_to_utf16_idx(toggle.edit.start)..text.byte_to_utf16_idx(toggle.edit.end);
            editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
            editor.set_cursor_position(position, window, cx);
        });
        self.status = if toggle.folded {
            "Folded brace block"
        } else {
            "Expanded brace block"
        }
        .into();
        cx.notify();
    }
}

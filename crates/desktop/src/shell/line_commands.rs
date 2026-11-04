//! Comment and line-editing commands (IDE-032).

use ide_core::{editing, language};

use super::*;

impl IdeShell {
    fn editing_target(&mut self, cx: &mut Context<Self>) -> Option<(usize, String, Range<usize>)> {
        if self.blocked() || self.show_diff {
            return None;
        }
        let ix = self.active;
        let (selection, _) = self.source_selection(ix, cx);
        let text = self.buffers[ix].document.text().to_string();
        Some((ix, text, selection))
    }

    fn run_transform(
        &mut self,
        transform: impl FnOnce(&str, Range<usize>) -> Option<editing::Transform>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((ix, text, selection)) = self.editing_target(cx) else {
            return;
        };
        let Some(transform) = transform(&text, selection) else {
            return;
        };
        self.apply_source_edits(
            ix,
            &transform.edits,
            Some((transform.selection, transform.reversed)),
            window,
            cx,
        );
    }

    pub(super) fn toggle_line_comment(
        &mut self,
        _: &ToggleLineComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let syntax = language::get(self.buffer().language);
        match (syntax.line_comment, syntax.block_comment) {
            (Some(token), _) => self.run_transform(
                |text, selection| Some(editing::toggle_line_comment(text, selection, token)),
                window,
                cx,
            ),
            // Languages without line comments (HTML, CSS, Markdown) wrap lines in blocks.
            (None, Some(delimiters)) => self.run_transform(
                |text, selection| Some(editing::toggle_block_comment(text, selection, delimiters)),
                window,
                cx,
            ),
            (None, None) => {
                self.status = format!("{} has no comment syntax", syntax.name);
                cx.notify();
            }
        }
    }

    pub(super) fn toggle_block_comment(
        &mut self,
        _: &ToggleBlockComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let syntax = language::get(self.buffer().language);
        let Some(delimiters) = syntax.block_comment else {
            self.status = format!("{} has no block comments", syntax.name);
            cx.notify();
            return;
        };
        self.run_transform(
            |text, selection| Some(editing::toggle_block_comment(text, selection, delimiters)),
            window,
            cx,
        );
    }

    pub(super) fn move_lines_up(
        &mut self,
        _: &MoveLinesUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| editing::move_lines(text, selection, true),
            window,
            cx,
        );
    }

    pub(super) fn move_lines_down(
        &mut self,
        _: &MoveLinesDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| editing::move_lines(text, selection, false),
            window,
            cx,
        );
    }

    pub(super) fn duplicate_lines_up(
        &mut self,
        _: &DuplicateLinesUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| Some(editing::duplicate_lines(text, selection, true)),
            window,
            cx,
        );
    }

    pub(super) fn duplicate_lines_down(
        &mut self,
        _: &DuplicateLinesDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| Some(editing::duplicate_lines(text, selection, false)),
            window,
            cx,
        );
    }

    pub(super) fn delete_lines(
        &mut self,
        _: &DeleteLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| Some(editing::delete_lines(text, selection)),
            window,
            cx,
        );
    }

    pub(super) fn insert_line_above(
        &mut self,
        _: &InsertLineAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| Some(editing::insert_line(text, selection, true)),
            window,
            cx,
        );
    }

    pub(super) fn insert_line_below(
        &mut self,
        _: &InsertLineBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_transform(
            |text, selection| Some(editing::insert_line(text, selection, false)),
            window,
            cx,
        );
    }
}

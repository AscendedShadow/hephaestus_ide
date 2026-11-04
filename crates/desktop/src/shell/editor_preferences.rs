//! Applying editor preferences and EditorConfig to buffers (IDE-014).

use gpui_component::input::TabSize;
use ide_core::editorconfig;

use super::*;
use crate::preferences::{self, EditorPreferences};

impl IdeShell {
    /// Resolve the effective preferences for a document in a language mode.
    pub(super) fn resolve_preferences(
        document: &Document,
        language: &str,
        cx: &App,
    ) -> EditorPreferences {
        let editorconfig = document.path().map(editorconfig::resolve);
        preferences::resolve(
            &settings::current(cx).editor,
            language,
            editorconfig.as_ref(),
        )
    }

    /// Re-resolve and apply preferences to one buffer (or all buffers).
    pub(super) fn refresh_preferences(&mut self, only: Option<usize>, cx: &mut Context<Self>) {
        let indices: Vec<usize> = match only {
            Some(ix) => vec![ix],
            None => (0..self.buffers.len()).collect(),
        };
        for ix in indices {
            let Some(buffer) = self.buffers.get_mut(ix) else {
                continue;
            };
            let preferences = Self::resolve_preferences(&buffer.document, buffer.language, cx);
            buffer.preferences = preferences;
            self.sync_view_settings(ix, cx);
        }
        if let Some(vim) = &mut self.vim {
            vim.set_indent_width(self.buffers[self.active].preferences.tab_width);
        }
        cx.notify();
    }

    pub(super) fn apply_preferences(
        editor: &Entity<InputState>,
        preferences: &EditorPreferences,
        cx: &mut App,
    ) {
        editor.update(cx, |editor, cx| {
            editor.set_tab_size(
                TabSize {
                    tab_size: preferences.tab_width,
                    hard_tabs: !preferences.insert_spaces,
                },
                cx,
            );
            editor.set_soft_wrap_mode(preferences.word_wrap, cx);
            editor.set_line_number_mode(preferences.line_numbers, cx);
        });
    }

    /// Apply the save-time policies (trailing whitespace, final newline, line ending for
    /// new files) to buffer `ix` before it is written.
    pub(super) fn apply_save_policies(
        &mut self,
        ix: usize,
        new_file: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(buffer) = self.buffers.get_mut(ix) else {
            return;
        };
        let preferences = buffer.preferences.clone();
        if new_file && let Some(line_ending) = preferences.line_ending {
            buffer.document.set_line_ending(line_ending);
        }
        let text = buffer.document.text().to_string();
        let edits = preferences::save_edits(&text, &preferences);
        if !edits.is_empty() {
            let (selection, reversed) = self.source_selection(ix, cx);
            let map = |offset| ide_core::text_edit::map_offset(&edits, offset);
            self.apply_source_edits(
                ix,
                &edits,
                Some((map(selection.start)..map(selection.end), reversed)),
                window,
                cx,
            );
        }
    }

    pub(super) fn indentation_label(&self) -> String {
        let preferences = &self.buffer().preferences;
        if preferences.insert_spaces {
            format!("Spaces: {}", preferences.tab_width)
        } else {
            format!("Tab Size: {}", preferences.tab_width)
        }
    }

    pub(super) fn editor_text_size(&self) -> Pixels {
        px(self.buffer().preferences.font_size)
    }

    pub(super) fn editor_font_family(&self) -> gpui::SharedString {
        match &self.buffer().preferences.font_family {
            Some(family) => family.clone().into(),
            None => theme::mono_family().into(),
        }
    }
}

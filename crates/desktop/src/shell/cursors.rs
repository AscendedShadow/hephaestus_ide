//! Multiple cursors and rectangular (column) selections (IDE-013).
//!
//! The editor widget owns the primary selection; secondary selections live here as byte
//! ranges of the editor text. Folds are expanded whenever secondary cursors are created,
//! so editor offsets equal document offsets while they exist.
//!
//! Behaviour:
//! - Typing, Backspace/Delete, completions, Tab/Shift-Tab indentation and paste made at
//!   the primary selection are repeated at every secondary selection, joined into the
//!   primary edit's undo step. Edits that cannot be repeated (e.g. a format) return to a
//!   single caret.
//! - Arrow keys, Home and End move every caret; other navigation, Escape, a plain click,
//!   Undo/Redo, folding and IME composition return to the primary caret.
//! - Copy/Cut join the selections with newlines; Paste distributes clipboard lines when
//!   their count equals the number of carets, otherwise pastes the whole text at each.
//! - Vim mode keeps a single caret.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::MouseUpEvent;
use gpui_component::input::{Copy, Cut, Paste, Redo, Undo};
use ide_core::multicursor;

use super::*;

#[derive(Default)]
pub(super) struct CursorState {
    active: Option<MultiCursor>,
    /// Shared with the keystroke interceptor so it only touches the shell while
    /// secondary cursors exist (it may run while the shell is being updated).
    live: Rc<Cell<bool>>,
    /// Anchor (editor offset) of an in-progress column-selection drag.
    column_drag: Option<usize>,
}

struct MultiCursor {
    buffer: BufferId,
    /// Secondary selections (editor offsets), sorted and non-overlapping.
    extras: Vec<Range<usize>>,
    /// The primary selection when the last key went down, to interpret the next edit.
    pending: Option<Range<usize>>,
    /// The editor text after edits the shell applied to every cursor itself.
    applied: Option<String>,
    /// Visual column kept by Add Cursor Above/Below.
    column_goal: Option<usize>,
    /// A navigation key went down; collapse unless a cursor command handles it.
    collapse_after_key: bool,
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end.max(b.start + 1) && b.start < a.end.max(a.start + 1)
}

impl IdeShell {
    fn set_cursor_state(&mut self, state: Option<MultiCursor>) {
        self.cursors.live.set(state.is_some());
        self.cursors.active = state;
    }

    /// A flag that is set while secondary cursors exist.
    pub(super) fn cursors_live(&self) -> Rc<Cell<bool>> {
        self.cursors.live.clone()
    }

    /// Secondary selections of the active buffer (editor offsets).
    pub(super) fn secondary_cursors(&self) -> &[Range<usize>] {
        match &self.cursors.active {
            Some(state) if state.buffer == self.buffer().id => &state.extras,
            _ => &[],
        }
    }

    /// The command target view of a buffer changed (editor groups).
    pub(super) fn cursors_view_changed(&mut self, cx: &mut Context<Self>) {
        self.clear_secondary_cursors(cx);
    }

    pub(super) fn clear_secondary_cursors(&mut self, cx: &mut Context<Self>) {
        if self.cursors.active.is_some() {
            self.set_cursor_state(None);
            cx.notify();
        }
    }

    fn cursors_unavailable(&mut self, cx: &mut Context<Self>) -> bool {
        if self.blocked() || self.show_diff {
            return true;
        }
        if self.vim.is_some() {
            self.status = "Multiple cursors are not available in Vim mode".into();
            cx.notify();
            return true;
        }
        false
    }

    /// Expand every fold of the active buffer, keeping the primary selection.
    fn expand_folds_for_cursors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        if self.buffers[ix].folds.is_empty() {
            return;
        }
        let (selection, reversed) = self.source_selection(ix, cx);
        let buffer = &mut self.buffers[ix];
        let source = buffer.document.text().to_string();
        let Some(toggle) = folding::unfold_range(&source, &mut buffer.folds, 0..source.len())
        else {
            return;
        };
        buffer.folds.clear();
        buffer.editor.clone().update(cx, |editor, cx| {
            let text = editor.text();
            let range =
                text.byte_to_utf16_idx(toggle.edit.start)..text.byte_to_utf16_idx(toggle.edit.end);
            editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
            editor.select_byte_range(selection, reversed, window, cx);
        });
    }

    /// Make `primary` the editor selection and `extras` the secondary ones (merged, with
    /// anything overlapping the primary dropped).
    fn set_cursors(
        &mut self,
        primary: Range<usize>,
        reversed: bool,
        extras: Vec<Range<usize>>,
        column_goal: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let extras: Vec<_> = multicursor::merge(extras)
            .into_iter()
            .filter(|range| !overlaps(range, &primary))
            .collect();
        self.editor().clone().update(cx, |editor, cx| {
            editor.select_byte_range(primary, reversed, window, cx)
        });
        let id = self.buffer().id;
        let state = MultiCursor {
            buffer: id,
            extras,
            pending: None,
            applied: None,
            column_goal,
            collapse_after_key: false,
        };
        self.set_cursor_state((!state.extras.is_empty()).then_some(state));
        let count = self.secondary_cursors().len() + 1;
        if count > 1 {
            self.status = format!("{count} cursors");
        }
        cx.notify();
    }

    pub(super) fn add_cursor_above(
        &mut self,
        _: &AddCursorAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertically(-1, window, cx);
    }

    pub(super) fn add_cursor_below(
        &mut self,
        _: &AddCursorBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertically(1, window, cx);
    }

    fn add_cursor_vertically(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursors_unavailable(cx) {
            return;
        }
        self.expand_folds_for_cursors(window, cx);
        let tab_width = self.buffer().preferences.tab_width;
        let editor = self.editor().read(cx);
        let text = editor.text().to_string();
        let primary = editor.selected_byte_range();
        let cursor = editor.cursor();
        let carets = self
            .secondary_cursors()
            .iter()
            .map(|range| range.end)
            .chain([cursor]);
        let edge = if delta < 0 {
            carets.min()
        } else {
            carets.max()
        }
        .unwrap_or(cursor);
        let goal = self
            .cursors
            .active
            .as_ref()
            .and_then(|state| state.column_goal)
            .unwrap_or_else(|| multicursor::visual_column(&text, cursor, tab_width));
        let target = multicursor::vertical(&text, edge, delta, tab_width)
            .and_then(|line| multicursor::offset_at_column(&text, line, goal, tab_width, true));
        let Some(target) = target else {
            self.status = format!("No line {}", if delta < 0 { "above" } else { "below" });
            cx.notify();
            return;
        };
        let mut extras = self.secondary_cursors().to_vec();
        extras.push(primary);
        self.set_cursors(target..target, false, extras, Some(goal), window, cx);
    }

    pub(super) fn select_next_occurrence(
        &mut self,
        _: &SelectNextOccurrence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors_unavailable(cx) {
            return;
        }
        self.expand_folds_for_cursors(window, cx);
        let editor = self.editor().read(cx);
        let text = editor.text().to_string();
        let primary = editor.selected_byte_range();
        let mut extras = self.secondary_cursors().to_vec();
        if primary.is_empty() {
            match multicursor::word_at(&text, primary.start) {
                Some(word) => self.set_cursors(word, false, extras, None, window, cx),
                None => {
                    self.status = "No word at the cursor".into();
                    cx.notify();
                }
            }
            return;
        }
        let mut taken = extras.clone();
        taken.push(primary.clone());
        match multicursor::next_occurrence(&text, &text[primary.clone()], primary.end, &taken) {
            Some(next) => {
                extras.push(primary);
                self.set_cursors(next, false, extras, None, window, cx);
            }
            None => {
                self.status = "No more occurrences".into();
                cx.notify();
            }
        }
    }

    pub(super) fn select_all_occurrences(
        &mut self,
        _: &SelectAllOccurrences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors_unavailable(cx) {
            return;
        }
        self.expand_folds_for_cursors(window, cx);
        let editor = self.editor().read(cx);
        let text = editor.text().to_string();
        let primary = editor.selected_byte_range();
        let Some(needle) = (if primary.is_empty() {
            multicursor::word_at(&text, primary.start)
        } else {
            Some(primary.clone())
        }) else {
            self.status = "No word at the cursor".into();
            cx.notify();
            return;
        };
        let mut all = multicursor::all_occurrences(&text, &text[needle.clone()]);
        let ix = all
            .iter()
            .position(|range| range.start <= needle.start && needle.start < range.end)
            .unwrap_or(0);
        let main = all.remove(ix);
        self.set_cursors(main, false, all, None, window, cx);
    }

    /// Keystroke hook (before key bindings and the editor handle the key).
    pub(super) fn cursors_keystroke(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors.active.is_none() || !self.editor().focus_handle(cx).is_focused(window) {
            return;
        }
        let id = self.buffer().id;
        let Some(state) = self.cursors.active.as_mut() else {
            return;
        };
        if state.buffer != id {
            self.set_cursor_state(None);
            return;
        }
        let modifiers = &keystroke.modifiers;
        let plain = !(modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.shift
            || modifiers.function);
        let key = keystroke.key.as_str();
        let navigation = matches!(
            key,
            "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown"
        );
        if key == "escape" && plain {
            self.set_cursor_state(None);
            self.status = "Single cursor".into();
            cx.notify();
        } else if navigation && plain && !matches!(key, "pageup" | "pagedown") {
            self.move_secondary_cursors(key, cx);
        } else if navigation {
            // Other navigation returns to a single caret, unless the key ran a cursor
            // command (Add Cursor Above/Below), which replaces the state.
            state.collapse_after_key = true;
            let shell = cx.entity();
            cx.defer(move |cx| {
                shell.update(cx, |this, cx| {
                    if this
                        .cursors
                        .active
                        .as_ref()
                        .is_some_and(|state| state.collapse_after_key)
                    {
                        this.clear_secondary_cursors(cx);
                    }
                })
            });
        } else {
            state.pending = Some(
                self.buffers[self.active]
                    .editor
                    .read(cx)
                    .selected_byte_range(),
            );
        }
    }

    fn move_secondary_cursors(&mut self, key: &str, cx: &mut Context<Self>) {
        let tab_width = self.buffer().preferences.tab_width;
        let text = self.editor().read(cx).text().to_string();
        let Some(state) = self.cursors.active.as_mut() else {
            return;
        };
        let previous = |offset: usize| {
            text[..offset]
                .char_indices()
                .next_back()
                .map_or(0, |(ix, _)| ix)
        };
        let next = |offset: usize| {
            text[offset..]
                .chars()
                .next()
                .map_or(offset, |character| offset + character.len_utf8())
        };
        let line_start = |offset: usize| text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
        let line_end = |offset: usize| {
            text[offset..]
                .find('\n')
                .map_or(text.len(), |ix| offset + ix)
        };
        let moved = state
            .extras
            .iter()
            .map(|range| {
                let caret = match key {
                    "left" if !range.is_empty() => range.start,
                    "right" if !range.is_empty() => range.end,
                    "left" => previous(range.start),
                    "right" => next(range.end),
                    "up" => multicursor::vertical(&text, range.start, -1, tab_width).unwrap_or(0),
                    "down" => {
                        multicursor::vertical(&text, range.end, 1, tab_width).unwrap_or(text.len())
                    }
                    "home" => line_start(range.start),
                    _ => line_end(range.end),
                };
                caret..caret
            })
            .collect();
        state.extras = multicursor::merge(moved);
        state.column_goal = None;
        cx.notify();
    }

    /// Change hook, run before the document is synchronised with the editor text.
    pub(super) fn cursors_after_change(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = &self.buffers[ix];
        let Some(state) = self.cursors.active.as_mut() else {
            return;
        };
        if state.buffer != buffer.id {
            return;
        }
        let editor = buffer.editor.clone();
        let displayed = editor.read(cx).text().to_string();
        let old = buffer.document.text().to_string();
        if displayed == old {
            return;
        }
        if state.applied.take().as_deref() == Some(displayed.as_str()) {
            return;
        }
        let collapse = |this: &mut Self, reason: &str, cx: &mut Context<Self>| {
            this.set_cursor_state(None);
            this.status = format!("Single cursor: {reason}");
            cx.notify();
        };
        if !buffer.folds.is_empty() {
            return collapse(self, "folding changed the text", cx);
        }
        if editor.read(cx).is_composing() || editor.read(cx).last_change_committed_composition() {
            return collapse(self, "IME composition edits the primary caret only", cx);
        }
        let Some(primary) = state.pending.take() else {
            return collapse(self, "the edit did not come from typing", cx);
        };
        let extras: Vec<_> = state
            .extras
            .iter()
            .filter(|range| !overlaps(range, &primary))
            .cloned()
            .collect();
        let (selection, reversed) = {
            let editor = editor.read(cx);
            (editor.selected_byte_range(), editor.is_selection_reversed())
        };
        let Some((edits, carets)) =
            multicursor::replicate(&old, &displayed, primary, editor.read(cx).cursor(), &extras)
        else {
            return collapse(self, "this edit cannot be repeated at every cursor", cx);
        };
        let shift = |offset: usize| {
            let delta: isize = edits
                .iter()
                .filter(|(range, _)| range.end <= offset)
                .map(|(range, text)| text.len() as isize - range.len() as isize)
                .sum();
            offset.saturating_add_signed(delta)
        };
        let selection = shift(selection.start)..shift(selection.end);
        state.extras = multicursor::merge(carets);
        editor.update(cx, |editor, cx| {
            editor.apply_edits_with(&edits, Some((selection, reversed)), true, window, cx)
        });
    }

    /// All selections of the active buffer (primary included) in document order, with
    /// the index of the primary.
    fn all_selections(&self, cx: &App) -> (Vec<Range<usize>>, usize) {
        let primary = self.editor().read(cx).selected_byte_range();
        let mut all = self.secondary_cursors().to_vec();
        all.push(primary.clone());
        all.sort_by_key(|range| range.start);
        let ix = all.iter().position(|range| *range == primary).unwrap_or(0);
        (all, ix)
    }

    /// Replace every selection with its text as one undo step; carets land after each.
    fn edit_all_selections(
        &mut self,
        texts: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (all, primary) = self.all_selections(cx);
        let editor = self.editor().clone();
        let text = editor.read(cx).text().to_string();
        let edits: Vec<_> = all.into_iter().zip(texts).collect();
        let Ok(after) = ide_core::text_edit::apply(&text, &edits) else {
            return;
        };
        let mut shift = 0isize;
        let carets: Vec<usize> = edits
            .iter()
            .map(|(range, text)| {
                let caret = (range.start as isize + shift) as usize + text.len();
                shift += text.len() as isize - range.len() as isize;
                caret
            })
            .collect();
        let main = carets[primary];
        if let Some(state) = self.cursors.active.as_mut() {
            state.extras = carets
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != primary)
                .map(|(_, caret)| *caret..*caret)
                .collect();
            state.pending = None;
            state.applied = Some(after);
        }
        editor.update(cx, |editor, cx| {
            editor.apply_edits(&edits, Some((main..main, false)), window, cx)
        });
        cx.notify();
    }

    fn cursors_copy(&mut self, cut: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.secondary_cursors().is_empty() {
            return false;
        }
        let (all, _) = self.all_selections(cx);
        if all.iter().all(Range::is_empty) {
            return false;
        }
        let text = self.editor().read(cx).text().to_string();
        let joined = all
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(joined));
        if cut {
            self.edit_all_selections(vec![String::new(); all.len()], window, cx);
        }
        true
    }

    fn cursors_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.secondary_cursors().is_empty() {
            return false;
        }
        let Some(clipboard) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let lines: Vec<String> = clipboard
            .strip_suffix('\n')
            .unwrap_or(&clipboard)
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
            .collect();
        if lines.len() != self.secondary_cursors().len() + 1 {
            // Pasted whole at the primary caret and repeated at the others.
            return false;
        }
        self.edit_all_selections(lines, window, cx);
        true
    }

    /// Mouse-down hook; returns whether the event was consumed.
    pub(super) fn cursors_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left {
            return false;
        }
        let modifier = settings::current(cx).multi_cursor_modifier;
        if !modifier.pressed(&event.modifiers) {
            if self.cursors.active.is_some() {
                self.clear_secondary_cursors(cx);
            }
            return false;
        }
        let editor = self.editor().read(cx);
        if !editor
            .text_geometry()
            .is_some_and(|geometry| geometry.viewport.contains(&event.position))
        {
            return false;
        }
        let display = editor.index_for_mouse_position(event.position);
        cx.stop_propagation();
        if self.cursors_unavailable(cx) {
            return true;
        }
        let offset = folding::display_to_source(&self.buffer().folds, display, false);
        self.expand_folds_for_cursors(window, cx);
        if event.modifiers.shift {
            let anchor = self.editor().read(cx).cursor();
            self.cursors.column_drag = Some(anchor);
            self.select_column(anchor, offset, window, cx);
        } else {
            self.toggle_cursor(offset, window, cx);
        }
        true
    }

    pub(super) fn cursors_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(anchor) = self.cursors.column_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.cursors.column_drag = None;
            return;
        }
        let head = self
            .editor()
            .read(cx)
            .index_for_mouse_position(event.position);
        self.select_column(anchor, head, window, cx);
    }

    pub(super) fn cursors_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.cursors.column_drag = None;
    }

    /// Add a caret at `offset`, or remove the secondary selection there.
    pub(super) fn toggle_cursor(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors_unavailable(cx) {
            return;
        }
        let primary = self.editor().read(cx).selected_byte_range();
        let mut extras = self.secondary_cursors().to_vec();
        let hit = |range: &Range<usize>| range.start <= offset && offset <= range.end;
        if let Some(ix) = extras.iter().position(hit) {
            extras.remove(ix);
            self.set_cursors(primary, false, extras, None, window, cx);
        } else if hit(&primary) {
            // Removing the primary promotes the most recent secondary caret.
            if let Some(next) = extras.pop() {
                self.set_cursors(next, false, extras, None, window, cx);
            }
        } else {
            extras.push(primary);
            self.set_cursors(offset..offset, false, extras, None, window, cx);
        }
    }

    /// Rectangular selection between `anchor` and `head` (editor offsets), by visual
    /// column over logical lines.
    pub(super) fn select_column(
        &mut self,
        anchor: usize,
        head: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors_unavailable(cx) {
            return;
        }
        let tab_width = self.buffer().preferences.tab_width;
        let text = self.editor().read(cx).text().to_string();
        let mut selections = multicursor::rectangle(&text, anchor, head, tab_width);
        let reversed = multicursor::visual_column(&text, head, tab_width)
            < multicursor::visual_column(&text, anchor, tab_width);
        let main = if selections.is_empty() {
            head..head
        } else if head >= anchor {
            selections.pop().unwrap_or(head..head)
        } else {
            selections.remove(0)
        };
        self.set_cursors(main, reversed, selections, None, window, cx);
    }

    /// Element hooks for the editor container. Action captures only reach the shell while
    /// secondary cursors exist (Vim mode dispatches editor actions mid-update).
    pub(super) fn cursor_listeners(&self, element: Div, cx: &mut Context<Self>) -> Div {
        fn when_live<A: 'static>(
            live: Rc<Cell<bool>>,
            listener: impl Fn(&A, &mut Window, &mut App) + 'static,
        ) -> impl Fn(&A, &mut Window, &mut App) + 'static {
            move |action, window, cx| {
                if live.get() {
                    listener(action, window, cx)
                }
            }
        }
        let live = self.cursors_live();
        element
            .on_mouse_move(cx.listener(Self::cursors_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::cursors_mouse_up))
            .capture_action(when_live(
                live.clone(),
                cx.listener(|this, _: &Undo, _, cx| this.clear_secondary_cursors(cx)),
            ))
            .capture_action(when_live(
                live.clone(),
                cx.listener(|this, _: &Redo, _, cx| this.clear_secondary_cursors(cx)),
            ))
            .capture_action(when_live(
                live.clone(),
                cx.listener(|this, _: &Copy, window, cx| {
                    if this.cursors_copy(false, window, cx) {
                        cx.stop_propagation();
                    }
                }),
            ))
            .capture_action(when_live(
                live.clone(),
                cx.listener(|this, _: &Cut, window, cx| {
                    if this.cursors_copy(true, window, cx) {
                        cx.stop_propagation();
                    }
                }),
            ))
            .capture_action(when_live(
                live,
                cx.listener(|this, _: &Paste, window, cx| {
                    if this.cursors_paste(window, cx) {
                        cx.stop_propagation();
                    }
                }),
            ))
    }

    /// Paint secondary carets and selections over the editor.
    pub(super) fn render_secondary_cursors(&self, cx: &App) -> Option<impl IntoElement> {
        let extras = self.secondary_cursors().to_vec();
        if extras.is_empty() {
            return None;
        }
        let editor = self.editor().clone();
        let text = editor.read(cx).text().to_string();
        let selection: Hsla = Hsla::from(theme::accent()).opacity(0.3);
        let caret: Hsla = theme::text().into();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, cx| {
                    let editor = editor.read(cx);
                    let Some(geometry) = editor.text_geometry() else {
                        return;
                    };
                    let height = geometry.line_height;
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: geometry.viewport,
                        }),
                        |window| {
                            for range in &extras {
                                let mut start = range.start;
                                while start < range.end {
                                    let end = text[start..]
                                        .find('\n')
                                        .map_or(text.len(), |ix| start + ix)
                                        .min(range.end);
                                    if let (Some(from), Some(to)) = (
                                        editor.offset_position_in_window(start),
                                        editor.offset_position_in_window(end),
                                    ) {
                                        let width = if to.y == from.y {
                                            (to.x - from.x).max(px(4.))
                                        } else {
                                            px(4.)
                                        };
                                        window.paint_quad(fill(
                                            Bounds::new(from, size(width, height)),
                                            selection,
                                        ));
                                    }
                                    start = end + 1;
                                }
                                if let Some(position) = editor.offset_position_in_window(range.end)
                                {
                                    window.paint_quad(fill(
                                        Bounds::new(position, size(px(2.), height)),
                                        caret,
                                    ));
                                }
                            }
                        },
                    );
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

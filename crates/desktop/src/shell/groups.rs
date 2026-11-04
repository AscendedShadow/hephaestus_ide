//! Split editor groups (IDE-012).
//!
//! Groups are laid out as a chain: each group after the first is split off the previous
//! one to the right or below. A document shown in several groups has one view (editor
//! widget) per group; views share the document, its folds, dirty state and one undo
//! history, while cursor, selection and scroll are per view. Edits made in one view are
//! mirrored into the others without being recorded twice. Language-server lifecycles are
//! per document, so extra views never reopen the document on the server.
//!
//! The focused group renders the full editor (gutter, overlays, diff view); the others
//! render their active view, and focusing one of them makes it the focused group.

use gpui::{AnyElement, Axis};
use gpui_component::resizable::ResizableState;

use super::*;
use crate::preferences::EditorPreferences;

pub(super) type GroupId = usize;

pub(super) struct BufferView {
    pub(super) group: GroupId,
    pub(super) editor: Entity<InputState>,
    /// Language mode the view's highlighter was set up for.
    language: &'static str,
    _subscription: Subscription,
}

pub(super) struct EditorGroup {
    pub(super) id: GroupId,
    /// How this group was split off the previous one.
    pub(super) axis: Axis,
    /// Sizes of the split between the previous group and this one.
    split: Option<Entity<ResizableState>>,
    /// The group's active document while another group has focus.
    pub(super) active: Option<BufferId>,
}

pub(super) struct Groups {
    pub(super) list: Vec<EditorGroup>,
    /// Index into `list` of the focused group.
    pub(super) focused: usize,
    next_id: GroupId,
}

impl Default for Groups {
    fn default() -> Self {
        Self {
            list: vec![EditorGroup {
                id: 0,
                axis: Axis::Horizontal,
                split: None,
                active: None,
            }],
            focused: 0,
            next_id: 1,
        }
    }
}

impl Groups {
    pub(super) fn focused_id(&self) -> GroupId {
        self.list[self.focused].id
    }

    pub(super) fn index(&self, id: GroupId) -> Option<usize> {
        self.list.iter().position(|group| group.id == id)
    }
}

/// The changed span between two texts: (range in `old`, replacement from `new`).
fn difference<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix = old[prefix..]
        .bytes()
        .rev()
        .zip(new[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !(old.is_char_boundary(old.len() - suffix) && new.is_char_boundary(new.len() - suffix)) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

impl IdeShell {
    /// Create an editor view of buffer `id` for `group`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_view(
        &mut self,
        id: BufferId,
        text: String,
        language: &'static str,
        preferences: &EditorPreferences,
        path: Option<PathBuf>,
        group: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> BufferView {
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language)
                .line_number(preferences.line_numbers)
                .soft_wrap(preferences.word_wrap)
                .tab_size(TabSize {
                    tab_size: preferences.tab_width,
                    hard_tabs: !preferences.insert_spaces,
                })
                .default_value(text)
        });
        language_server::attach(&editor, &self.servers, path, language, cx);
        self.attach_completion(id, &editor, cx);
        let subscription = cx.subscribe_in(
            &editor,
            window,
            move |this, editor, event, window, cx| match event {
                InputEvent::Change => this.view_changed(id, editor, window, cx),
                InputEvent::Focus => this.view_focused(id, editor, window, cx),
                _ => {}
            },
        );
        BufferView {
            group,
            editor,
            language,
            _subscription: subscription,
        }
    }

    fn view_changed(
        &mut self,
        id: BufferId,
        editor: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        {
            // Echo of an edit mirrored from another view: nothing new.
            let buffer = &self.buffers[ix];
            if buffer.views.len() > 1 && buffer.editor != *editor {
                let displayed = editor.read(cx).text();
                let echo = if buffer.folds.is_empty() {
                    displayed == buffer.document.text()
                } else {
                    *displayed
                        == folding::projected(&buffer.document.text().to_string(), &buffer.folds)
                };
                if echo {
                    return;
                }
            }
        }
        if self.buffers[ix].editor == *editor {
            self.cursors_after_change(ix, window, cx);
        }
        // Ropes clone in O(1) (shared); the common unfolded case never copies the text.
        let displayed = editor.read(cx).text().clone();
        let buffer = &mut self.buffers[ix];
        let old = buffer.document.text().clone();
        let fast = if buffer.folds.is_empty() {
            folding::apply_unfolded_edit(&old, &displayed)
        } else {
            Some(None)
        };
        let changed = match fast {
            None => false,
            Some(Some(text)) => {
                buffer.document.set_text(text);
                true
            }
            Some(None) => {
                let source = old.to_string();
                match folding::apply_edit(&source, &mut buffer.folds, &displayed.to_string()) {
                    Some(text) => {
                        buffer.document.set_text(text.into());
                        true
                    }
                    None => false,
                }
            }
        };
        if changed {
            self.shift_breakpoints_for_edit(ix, &old, cx);
        }
        if self.buffers[ix].views.len() > 1 {
            self.mirror_to_other_views(ix, editor, &displayed.to_string(), window, cx);
        }
        let buffer = &mut self.buffers[ix];
        buffer.dirty = buffer.document.is_dirty();
        self.sync_lsp_buffer(ix, cx);
        if self.buffers[ix].dirty {
            self.schedule_auto_save(id, cx);
        }
        self.track_snippet(ix);
        self.schedule_recovery_snapshot(cx);
        if ix == self.active {
            self.schedule_outline_refresh(window, cx);
            self.signature_help_after_edit(ix, window, cx);
        }
        cx.notify();
    }

    fn mirror_to_other_views(
        &mut self,
        ix: usize,
        source: &Entity<InputState>,
        displayed: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let others: Vec<_> = self.buffers[ix]
            .views
            .iter()
            .map(|view| view.editor.clone())
            .filter(|editor| editor != source)
            .collect();
        for other in others {
            let text = other.read(cx).text().to_string();
            if text != displayed {
                let (range, replacement) = difference(&text, displayed);
                other.update(cx, |other, cx| {
                    other.apply_external_edit(range, replacement, window, cx)
                });
            }
        }
    }

    fn view_focused(
        &mut self,
        id: BufferId,
        editor: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let Some(group) = self.buffers[ix]
            .views
            .iter()
            .find(|view| view.editor == *editor)
            .map(|view| view.group)
        else {
            return;
        };
        if group == self.groups.focused_id()
            && self.active == ix
            && self.buffers[ix].editor == *editor
        {
            return;
        }
        self.focus_group_buffer(group, ix, window, cx);
    }

    /// The view of buffer `ix` in `group`, if any.
    pub(super) fn view_in(&self, ix: usize, group: GroupId) -> Option<&Entity<InputState>> {
        self.buffers[ix]
            .views
            .iter()
            .find(|view| view.group == group)
            .map(|view| &view.editor)
    }

    /// Indices of the buffers shown in `group`, in tab order.
    pub(super) fn group_tabs(&self, group: GroupId) -> Vec<usize> {
        (0..self.buffers.len())
            .filter(|ix| self.view_in(*ix, group).is_some())
            .collect()
    }

    /// Make sure buffer `ix` has a view in the focused group and make it the buffer's
    /// command target (called by `activate`).
    pub(super) fn ensure_focused_view(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group = self.groups.focused_id();
        if self.view_in(ix, group).is_none() {
            self.add_view(ix, group, window, cx);
        }
        if let Some(editor) = self.view_in(ix, group).cloned() {
            let buffer = &mut self.buffers[ix];
            if buffer.editor != editor {
                buffer.editor = editor;
                self.cursors_view_changed(cx);
            }
        }
        self.groups.list[self.groups.focused].active = Some(self.buffers[ix].id);
        self.sync_view_settings(ix, cx);
    }

    /// Add a view of buffer `ix` to `group`, sharing text, folds and undo history, with
    /// the selection of the buffer's current view.
    fn add_view(&mut self, ix: usize, group: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = &self.buffers[ix];
        let owner = buffer.editor.clone();
        let (text, selection, reversed) = {
            let owner = owner.read(cx);
            (
                owner.text().to_string(),
                owner.selected_byte_range(),
                owner.is_selection_reversed(),
            )
        };
        let (id, language, preferences, path) = (
            buffer.id,
            buffer.language,
            buffer.preferences.clone(),
            buffer.document.path().map(Path::to_path_buf),
        );
        let view = self.create_view(id, text, language, &preferences, path, group, window, cx);
        view.editor.update(cx, |editor, cx| {
            editor.share_history_with(owner.read(cx));
            editor.select_byte_range(selection, reversed, window, cx);
        });
        self.buffers[ix].views.push(view);
    }

    /// Bring the views' highlighter and preferences in line with their buffer.
    pub(super) fn sync_view_settings(&mut self, ix: usize, cx: &mut Context<Self>) {
        let buffer = &mut self.buffers[ix];
        let language = buffer.language;
        let path = buffer.document.path().map(Path::to_path_buf);
        let mut attach = Vec::new();
        for view in &mut buffer.views {
            if view.language != language {
                view.language = language;
                view.editor
                    .update(cx, |editor, cx| editor.set_highlighter(language, cx));
                attach.push(view.editor.clone());
            }
        }
        let preferences = buffer.preferences.clone();
        let views: Vec<_> = buffer
            .views
            .iter()
            .map(|view| view.editor.clone())
            .collect();
        for editor in attach {
            language_server::attach(&editor, &self.servers, path.clone(), language, cx);
        }
        for editor in views {
            Self::apply_preferences(&editor, &preferences, cx);
        }
    }

    /// Focus `group` showing buffer `ix`.
    pub(super) fn focus_group_buffer(
        &mut self,
        group: GroupId,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group_ix) = self.groups.index(group) else {
            return;
        };
        if let Some(current) = self.buffers.get(self.active) {
            self.groups.list[self.groups.focused].active = Some(current.id);
        }
        if self.groups.focused != group_ix {
            self.groups.focused = group_ix;
            self.clear_secondary_cursors(cx);
        }
        self.activate(ix, window, cx);
    }

    fn focus_group(&mut self, group_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let group = &self.groups.list[group_ix];
        let id = group.id;
        let tabs = self.group_tabs(id);
        let ix = group
            .active
            .and_then(|active| self.buffer_index(active))
            .filter(|ix| tabs.contains(ix))
            .or_else(|| tabs.first().copied());
        if let Some(ix) = ix {
            self.focus_group_buffer(id, ix, window, cx);
        }
    }

    /// Split the focused group, showing the active document in the new group.
    pub(super) fn split_group(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let ix = self.active;
        let id = self.groups.next_id;
        self.groups.next_id += 1;
        self.groups.list[self.groups.focused].active = Some(self.buffers[ix].id);
        self.groups.list.insert(
            self.groups.focused + 1,
            EditorGroup {
                id,
                axis,
                split: Some(cx.new(|_| ResizableState::default())),
                active: Some(self.buffers[ix].id),
            },
        );
        self.add_view(ix, id, window, cx);
        self.show_diff = false;
        self.focus_group_buffer(id, ix, window, cx);
        self.status = format!(
            "Split editor {}",
            if axis == Axis::Horizontal {
                "right"
            } else {
                "down"
            }
        );
    }

    pub(super) fn split_editor_right(
        &mut self,
        _: &SplitEditorRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_group(Axis::Horizontal, window, cx);
    }

    pub(super) fn split_editor_down(
        &mut self,
        _: &SplitEditorDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_group(Axis::Vertical, window, cx);
    }

    pub(super) fn focus_next_group(
        &mut self,
        _: &FocusNextGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = (self.groups.focused + 1) % self.groups.list.len();
        self.focus_group(next, window, cx);
    }

    pub(super) fn focus_previous_group(
        &mut self,
        _: &FocusPreviousGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.groups.list.len();
        let previous = (self.groups.focused + len - 1) % len;
        self.focus_group(previous, window, cx);
    }

    /// Remove buffer `ix`'s view in `group`; the buffer must have another view.
    fn drop_view(&mut self, ix: usize, group: GroupId, cx: &mut Context<Self>) {
        let buffer = &mut self.buffers[ix];
        let Some(position) = buffer.views.iter().position(|view| view.group == group) else {
            return;
        };
        if buffer.views.len() < 2 {
            return;
        }
        let view = buffer.views.remove(position);
        if buffer.editor == view.editor {
            buffer.editor = buffer.views[0].editor.clone();
            self.cursors_view_changed(cx);
        }
    }

    /// Close the tab of buffer `id` in `group`: only the view when the document is
    /// visible in another group, otherwise the document (with the usual unsaved-changes
    /// prompt).
    pub(super) fn close_tab_in(
        &mut self,
        group: GroupId,
        id: BufferId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        if self.buffers[ix].views.len() < 2 {
            if group != self.groups.focused_id() || ix != self.active {
                self.focus_group_buffer(group, ix, window, cx);
            }
            self.request(PendingAction::CloseBuffer(id), window, cx);
            return;
        }
        let focused = group == self.groups.focused_id();
        self.drop_view(ix, group, cx);
        self.after_tab_left_group(group, ix, focused, window, cx);
    }

    /// After buffer `ix` left `group`: close the group when it is empty, otherwise pick
    /// its next active tab.
    fn after_tab_left_group(
        &mut self,
        group: GroupId,
        ix: usize,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = self.group_tabs(group);
        if tabs.is_empty() {
            self.remove_group(group, window, cx);
            return;
        }
        let next = tabs
            .iter()
            .rev()
            .find(|tab| **tab < ix)
            .or(tabs.first())
            .copied()
            .unwrap_or(0);
        if focused {
            self.focus_group_buffer(group, next, window, cx);
        } else if let Some(group_ix) = self.groups.index(group) {
            self.groups.list[group_ix].active = Some(self.buffers[next].id);
            cx.notify();
        }
    }

    /// Remove an empty group (or close a group: documents only shown there move to the
    /// neighbouring group, so nothing unsaved is discarded).
    pub(super) fn remove_group(
        &mut self,
        group: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.groups.list.len() < 2 {
            return;
        }
        let Some(group_ix) = self.groups.index(group) else {
            return;
        };
        let neighbour_ix = if group_ix == 0 { 1 } else { group_ix - 1 };
        let neighbour = self.groups.list[neighbour_ix].id;
        for ix in self.group_tabs(group) {
            if self.buffers[ix].views.len() > 1 {
                self.drop_view(ix, group, cx);
            } else if let Some(view) = self.buffers[ix].views.first_mut() {
                view.group = neighbour;
            }
        }
        let removed = self.groups.list.remove(group_ix);
        if group_ix == 0
            && let Some(first) = self.groups.list.first_mut()
        {
            // The new first group has no split before it.
            first.split = None;
            first.axis = removed.axis;
        }
        let was_focused = self.groups.focused == group_ix;
        if self.groups.focused > group_ix || self.groups.focused >= self.groups.list.len() {
            self.groups.focused = self.groups.focused.saturating_sub(1);
        }
        if was_focused {
            let target = self.groups.index(neighbour).unwrap_or(0);
            self.groups.focused = target;
            self.focus_group(target, window, cx);
        }
        cx.notify();
    }

    pub(super) fn close_editor_group(
        &mut self,
        _: &CloseEditorGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        if self.groups.list.len() < 2 {
            self.status = "Only one editor group is open".into();
            cx.notify();
            return;
        }
        self.remove_group(self.groups.focused_id(), window, cx);
    }

    /// Move the active tab to the next group (splitting right when there is none).
    pub(super) fn move_tab_to_next_group(
        &mut self,
        _: &MoveTabToNextGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || self.show_diff {
            return;
        }
        let ix = self.active;
        let source = self.groups.focused_id();
        if self.groups.list.len() < 2 {
            if self.group_tabs(source).len() < 2 {
                self.status = "Nothing to move: the group has a single tab".into();
                cx.notify();
                return;
            }
            self.split_group(Axis::Horizontal, window, cx);
            self.drop_view(ix, source, cx);
            self.after_tab_left_group(source, ix, false, window, cx);
            return;
        }
        let target_ix = (self.groups.focused + 1) % self.groups.list.len();
        let target = self.groups.list[target_ix].id;
        self.move_view(ix, source, target, window, cx);
    }

    /// Move buffer `ix`'s view from `source` to `target` and focus it there.
    pub(super) fn move_view(
        &mut self,
        ix: usize,
        source: GroupId,
        target: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if source == target {
            return;
        }
        if self.view_in(ix, target).is_some() {
            self.drop_view(ix, source, cx);
        } else if let Some(view) = self.buffers[ix]
            .views
            .iter_mut()
            .find(|view| view.group == source)
        {
            view.group = target;
        }
        self.focus_group_buffer(target, ix, window, cx);
        if self.group_tabs(source).is_empty() {
            self.remove_group(source, window, cx);
        } else if let Some(source_ix) = self.groups.index(source) {
            let tabs = self.group_tabs(source);
            let next = tabs
                .iter()
                .rev()
                .find(|tab| **tab < ix)
                .or(tabs.first())
                .copied();
            self.groups.list[source_ix].active = next.map(|next| self.buffers[next].id);
        }
    }

    /// Show buffer `ix` where it is already visible (focusing that group), or open it in
    /// the focused group.
    pub(super) fn reveal_buffer(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.groups.focused_id();
        match self.buffers[ix]
            .views
            .iter()
            .map(|view| view.group)
            .find(|group| *group != focused)
        {
            Some(group) if self.view_in(ix, focused).is_none() => {
                self.focus_group_buffer(group, ix, window, cx)
            }
            _ => self.activate(ix, window, cx),
        }
    }

    /// The group layout for the session file (empty when not split).
    pub(super) fn session_groups(&self, cx: &App) -> Vec<crate::session::SessionGroup> {
        if self.groups.list.len() < 2 {
            return Vec::new();
        }
        // Session files are the buffers with a path, in buffer order.
        let file_index = |ix: usize| {
            self.buffers[ix].document.path()?;
            Some(
                self.buffers[..ix]
                    .iter()
                    .filter(|buffer| buffer.document.path().is_some())
                    .count(),
            )
        };
        self.groups
            .list
            .iter()
            .enumerate()
            .map(|(group_ix, group)| {
                let tabs: Vec<usize> = self
                    .group_tabs(group.id)
                    .into_iter()
                    .filter(|ix| file_index(*ix).is_some())
                    .collect();
                let active = if group_ix == self.groups.focused {
                    Some(self.active)
                } else {
                    group.active.and_then(|id| self.buffer_index(id))
                };
                crate::session::SessionGroup {
                    down: group.axis == Axis::Vertical,
                    files: tabs.iter().filter_map(|ix| file_index(*ix)).collect(),
                    cursors: tabs
                        .iter()
                        .filter_map(|ix| self.view_in(*ix, group.id))
                        .map(|editor| {
                            let cursor = editor.read(cx).cursor_position();
                            (cursor.line, cursor.character)
                        })
                        .collect(),
                    active: active.and_then(file_index),
                }
            })
            .collect()
    }

    /// Rebuild the saved group layout after the session's files were opened.
    pub(super) fn restore_groups(
        &mut self,
        files: &[PathBuf],
        saved: &[crate::session::SessionGroup],
        focused: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer_of =
            |this: &Self, file: usize| files.get(file).and_then(|path| this.buffer_for_path(path));
        for group in &saved[1..] {
            let id = self.groups.next_id;
            self.groups.next_id += 1;
            self.groups.list.push(EditorGroup {
                id,
                axis: if group.down {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                },
                split: Some(cx.new(|_| ResizableState::default())),
                active: None,
            });
        }
        for (group_ix, group) in saved.iter().enumerate() {
            let id = self.groups.list[group_ix].id;
            for (position, file) in group.files.iter().enumerate() {
                let Some(ix) = buffer_of(self, *file) else {
                    continue;
                };
                if self.view_in(ix, id).is_none() {
                    self.add_view(ix, id, window, cx);
                }
                if let (Some(editor), Some((line, column))) = (
                    self.view_in(ix, id).cloned(),
                    group.cursors.get(position).copied(),
                ) {
                    editor.update(cx, |editor, cx| {
                        editor.set_cursor_position(Position::new(line, column), window, cx)
                    });
                }
            }
            self.groups.list[group_ix].active = group
                .active
                .and_then(|file| buffer_of(self, file))
                .map(|ix| self.buffers[ix].id);
        }
        // Documents restored into the first group that belong elsewhere leave it.
        let first = self.groups.list[0].id;
        let listed: Vec<usize> = saved[0]
            .files
            .iter()
            .filter_map(|file| buffer_of(self, *file))
            .collect();
        for ix in self.group_tabs(first) {
            if !listed.contains(&ix) && self.buffers[ix].views.len() > 1 {
                self.drop_view(ix, first, cx);
            }
        }
        let focused = focused.min(self.groups.list.len() - 1);
        if self.group_tabs(first).is_empty() {
            self.groups.focused = 0;
            self.remove_group(first, window, cx);
        } else {
            self.focus_group(focused, window, cx);
        }
    }

    /// Tab indices shown in the focused group.
    pub(super) fn focused_tabs(&self) -> Vec<usize> {
        self.group_tabs(self.groups.focused_id())
    }

    /// Render all groups; `focused` is the full editor of the focused group.
    pub(super) fn render_groups(&self, focused: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let mut focused = Some(focused);
        let mut elements: Vec<AnyElement> = Vec::new();
        for (group_ix, group) in self.groups.list.iter().enumerate() {
            if group_ix == self.groups.focused {
                elements.push(focused.take().unwrap_or_else(|| div().into_any_element()));
            } else {
                elements.push(self.render_unfocused_group(group, cx).into_any_element());
            }
        }
        // Fold from the last group backwards: group i | (group i+1 ...).
        let mut rest = elements.pop().unwrap_or_else(|| div().into_any_element());
        for (group_ix, element) in elements.into_iter().enumerate().rev() {
            let next = &self.groups.list[group_ix + 1];
            let Some(state) = next.split.clone() else {
                continue;
            };
            let id = ("editor-group-split", next.id);
            let split = match next.axis {
                Axis::Horizontal => h_resizable(id),
                Axis::Vertical => v_resizable(id),
            };
            rest = split
                .with_state(&state)
                .child(resizable_panel().child(element))
                .child(resizable_panel().child(rest))
                .into_any_element();
        }
        rest
    }

    fn render_unfocused_group(
        &self,
        group: &EditorGroup,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tabs = self.group_tabs(group.id);
        let active = group
            .active
            .and_then(|id| self.buffer_index(id))
            .filter(|ix| tabs.contains(ix))
            .or_else(|| tabs.first().copied());
        let editor = active.and_then(|ix| self.view_in(ix, group.id)).cloned();
        div()
            .id(("editor-group", group.id))
            .debug_selector({
                let id = group.id;
                move || format!("editor-group-{id}")
            })
            .key_context(EDITOR_CONTEXT)
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(active.map(|ix| self.buffers[ix].id), |element, buffer| {
                let group = group.id;
                // Clicking into an unfocused group focuses it (before the editor places
                // the caret).
                element.capture_any_mouse_down(cx.listener(move |this, _, window, cx| {
                    if !this.blocked()
                        && let Some(ix) = this.buffer_index(buffer)
                    {
                        this.focus_group_buffer(group, ix, window, cx);
                    }
                }))
            })
            .child(self.render_group_tabs(group.id, active, cx))
            .children(editor.map(|editor| {
                div().relative().flex_1().min_h_0().overflow_hidden().child(
                    Input::new(&editor)
                        .h_full()
                        .w_full()
                        .pl(EDITOR_PADDING_LEFT)
                        .bordered(false)
                        .focus_bordered(false)
                        .appearance(false)
                        .font_family(self.editor_font_family())
                        .text_size(self.editor_text_size())
                        .bg(theme::background())
                        .text_color(theme::text()),
                )
            }))
    }
}

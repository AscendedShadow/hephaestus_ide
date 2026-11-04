//! Editor tabs: titles, switching, closing and rendering.

use super::*;

impl IdeShell {
    pub(super) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_diff {
            self.request(PendingAction::LeaveDiff, window, cx);
        } else {
            self.close_tab_in(self.groups.focused_id(), self.buffer().id, window, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let tabs = self.focused_tabs();
        let files = tabs.len();
        let has_diff = self.diff_view.read(cx).target().is_some();
        let current = if self.show_diff {
            files
        } else {
            tabs.iter().position(|tab| *tab == self.active).unwrap_or(0)
        };
        let len = (files + usize::from(has_diff)) as isize;
        if len == 0 {
            return;
        }
        let ix = (current as isize + step).rem_euclid(len) as usize;
        if ix == files {
            self.focus_diff(window, cx);
        } else {
            self.activate(tabs[ix], window, cx);
        }
    }

    pub(super) fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    pub(super) fn previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(-1, window, cx);
    }

    pub(super) fn tab_title(&self, buffer: &Buffer) -> String {
        if let Some(title) = &buffer.title_override {
            return title.clone();
        }
        let title = self.plain_tab_title(buffer);
        if buffer.recovered && buffer.dirty {
            format!("{title} (recovered)")
        } else {
            title
        }
    }

    pub(super) fn plain_tab_title(&self, buffer: &Buffer) -> String {
        let name = buffer.document.name();
        let shared = self
            .buffers
            .iter()
            .any(|other| other.id != buffer.id && other.document.name() == name);
        match buffer
            .document
            .path()
            .and_then(Path::parent)
            .and_then(Path::file_name)
        {
            Some(folder) if shared => format!("{name} — {}", folder.to_string_lossy()),
            _ => name,
        }
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = (!self.show_diff).then_some(self.active);
        self.render_group_tabs(self.groups.focused_id(), active, cx)
            .when_some(self.diff_view.read(cx).title(), |tabs, title| {
                tabs.child(self.render_diff_tab(title, cx))
            })
    }

    pub(super) fn render_group_tabs(
        &self,
        group: groups::GroupId,
        active: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("tabs", group))
            .debug_selector(move || format!("tabs-{group}"))
            .h(px(HEADER_HEIGHT + 2.))
            .flex_shrink_0()
            .flex()
            .overflow_x_scroll()
            .bg(theme::panel())
            .children(
                self.group_tabs(group)
                    .into_iter()
                    .map(|ix| self.render_tab(group, active == Some(ix), &self.buffers[ix], cx)),
            )
    }

    pub(super) fn render_tab(
        &self,
        group: groups::GroupId,
        active: bool,
        buffer: &Buffer,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = buffer.id;
        let dirty = buffer.dirty;
        let close_hidden = dirty || !active;
        editor_tab(
            ("tab", id * 1000 + group as u64),
            active,
            Icon::new(FileKind::of(buffer.document.path().unwrap_or(Path::new(""))).icon()),
            self.tab_title(buffer),
        )
        .child(
            div()
                .relative()
                .size(px(18.))
                .flex_shrink_0()
                .when(dirty, |slot| {
                    slot.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .group_hover(TAB_GROUP, |style| style.invisible())
                            .child(div().size(px(8.)).rounded_full().bg(theme::accent())),
                    )
                })
                .child(
                    tab_close_button(("close-tab", id * 1000 + group as u64), close_hidden)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab_in(group, id, window, cx);
                        })),
                ),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !this.blocked()
                && let Some(ix) = this.buffer_index(id)
            {
                this.focus_group_buffer(group, ix, window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, window, cx| this.close_tab_in(group, id, window, cx)),
        )
    }

    pub(super) fn render_diff_tab(&self, title: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.show_diff;
        editor_tab("diff-tab", active, Icon::new(AppIcon::GitBranch), title)
            .debug_selector(|| "diff-tab".into())
            .child(div().size(px(18.)).flex_shrink_0().child(
                tab_close_button("close-diff-tab", !active).on_click(cx.listener(
                    |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request(PendingAction::LeaveDiff, window, cx);
                    },
                )),
            ))
            .on_click(cx.listener(|this, _, window, cx| {
                if !this.blocked() {
                    this.focus_diff(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, _, window, cx| {
                    this.request(PendingAction::LeaveDiff, window, cx)
                }),
            )
    }

    pub(super) fn active_tab(&self, cx: &App) -> (String, bool) {
        match self.diff_view.read(cx).title() {
            Some(title) if self.show_diff => (title, false),
            _ => (self.document().name(), self.buffer().dirty),
        }
    }
}

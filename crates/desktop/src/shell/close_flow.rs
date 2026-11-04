//! Unsaved-changes prompts, closing buffers or the window, and the diff tab.

use super::*;

impl IdeShell {
    pub(super) fn focus_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_diff = true;
        window.focus(&self.diff_view.focus_handle(cx));
        cx.notify();
    }

    pub(super) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_diff = false;
        self.git.update(cx, |git, cx| git.clear_selection(cx));
        self.focus_editor(window, cx);
        cx.notify();
    }

    pub(super) fn leave_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.diff_view.update(cx, |view, cx| view.finish_switch(cx)) {
            self.focus_diff(window, cx);
        } else {
            self.close_diff(window, cx);
        }
    }

    pub(super) fn cancel_switch(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.diff_view.update(cx, |view, _| view.cancel_switch()) {
            self.git.update(cx, |git, cx| git.select(target, cx));
        }
    }

    pub(super) fn cancel_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.take() == Some(PendingAction::LeaveDiff) {
            self.cancel_switch(cx);
            self.focus_diff(window, cx);
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    pub(super) fn save_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending != Some(PendingAction::LeaveDiff) {
            self.save(SaveTarget::Current, None, window, cx);
            return;
        }
        let Some(task) = self.diff_view.update(cx, |view, cx| view.save_merge(cx)) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            if task.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(action) = this.pending.take() {
                        this.perform(action, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn can_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if self.buffers.iter().any(|buffer| buffer.dirty) {
            self.request(PendingAction::CloseWindow, window, cx);
            false
        } else {
            self.save_session(cx);
            self.clear_recovery();
            true
        }
    }

    pub(super) fn request(
        &mut self,
        action: PendingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.blocked() {
            self.perform(action, window, cx);
        }
    }

    pub(super) fn perform(
        &mut self,
        action: PendingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let unsaved = match action {
            PendingAction::CloseBuffer(id) => {
                self.buffer_index(id).filter(|&ix| self.buffers[ix].dirty)
            }
            PendingAction::LeaveDiff => None,
            PendingAction::CloseWindow => self.buffers.iter().position(|buffer| buffer.dirty),
        };
        if let Some(ix) = unsaved {
            self.reveal_buffer(ix, window, cx);
            self.pending = Some(action);
            cx.notify();
            return;
        }
        if action == PendingAction::LeaveDiff && self.diff_view.read(cx).has_unsaved_merge(cx) {
            self.focus_diff(window, cx);
            self.pending = Some(action);
            cx.notify();
            return;
        }
        match action {
            PendingAction::CloseBuffer(id) => {
                if let Some(ix) = self.buffer_index(id) {
                    self.remove_buffer(ix, window, cx);
                }
            }
            PendingAction::LeaveDiff => self.leave_diff(window, cx),
            PendingAction::CloseWindow => {
                self.save_session(cx);
                self.clear_recovery();
                window.remove_window();
            }
        }
    }

    pub(super) fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.pending.take() else {
            return;
        };
        if action == PendingAction::LeaveDiff {
            self.leave_diff(window, cx);
            return;
        }
        self.remove_buffer(self.active, window, cx);
        if action == PendingAction::CloseWindow {
            self.perform(action, window, cx);
        }
        cx.notify();
    }

    pub(super) fn close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request(PendingAction::CloseWindow, window, cx);
    }

    pub(super) fn render_save_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let diff_view = self.diff_view.read(cx);
        let (message, busy) = if self.pending == Some(PendingAction::LeaveDiff) {
            let name = diff_view
                .target()
                .map(|target| target.relative.clone())
                .unwrap_or_default();
            let leaving = if diff_view.switching() {
                "switching files"
            } else {
                "closing"
            };
            (
                format!("Save the merge result for {name} before {leaving}?"),
                self.busy || diff_view.merge_busy(cx),
            )
        } else {
            (
                format!("Save changes to {} before closing?", self.document().name()),
                self.busy,
            )
        };
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .bg(theme::accent_wash())
            .border_b_1()
            .border_color(theme::border())
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(px(16.))
                    .text_color(theme::accent()),
            )
            .child(div().flex_1().min_w_0().truncate().child(message))
            .child(
                Button::new("confirm-save")
                    .primary()
                    .small()
                    .label("Save")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save_pending(window, cx))),
            )
            .child(
                Button::new("discard")
                    .small()
                    .label("Discard")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
            )
            .child(
                Button::new("cancel")
                    .ghost()
                    .small()
                    .label("Cancel")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_pending(window, cx))),
            )
    }
}

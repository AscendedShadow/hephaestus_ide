//! Sidebar and tool panel visibility, and the project/Git context they show.

use super::*;

impl IdeShell {
    /// The folder for root-scoped features (Git, run/debug, tasks): the active root.
    pub(super) fn project_directory(&self) -> Option<&Path> {
        self.active_root()
            .or_else(|| self.document().path().and_then(Path::parent))
    }

    pub(super) fn sync_git(&mut self, cx: &mut Context<Self>) -> bool {
        let directory = self.project_directory().map(Path::to_path_buf);
        self.git
            .update(cx, |git, cx| git.set_directory(directory, cx))
    }

    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if !self.sync_git(cx) {
            self.git.update(cx, |git, cx| git.refresh(cx));
        }
    }

    pub(super) fn show_folder_panel(
        &mut self,
        _: &ShowFolderPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_sidebar = SidebarPanel::Folder;
        self.show_sidebar = true;
        cx.notify();
    }

    pub(super) fn show_git_panel(
        &mut self,
        _: &ShowGitPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_sidebar = SidebarPanel::Git;
        self.show_sidebar = true;
        self.refresh_git(cx);
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.show_sidebar && self.active_sidebar == panel {
            self.show_sidebar = false;
        } else {
            self.active_sidebar = panel;
            self.show_sidebar = true;
            if panel == SidebarPanel::Git {
                self.refresh_git(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn forward_to_git<A: Action>(
        handler: fn(&mut GitPanel, &A, &mut Window, &mut Context<GitPanel>),
    ) -> impl Fn(&mut Self, &A, &mut Window, &mut Context<Self>) {
        move |this, action, window, cx| {
            this.git
                .update(cx, |git, cx| handler(git, action, window, cx))
        }
    }

    pub(super) fn show_debug_panel(
        &mut self,
        _: &ShowDebugPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_panel = ToolPanel::Debug;
        cx.notify();
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "sidebar".into())
            .size_full()
            .overflow_hidden()
            .bg(theme::panel())
            .child(match self.active_sidebar {
                SidebarPanel::Folder => self.render_project_sidebar(cx).into_any_element(),
                SidebarPanel::Git => self.git.clone().into_any_element(),
                SidebarPanel::Search => self.search_panel.clone().into_any_element(),
                SidebarPanel::Outline => self.render_outline(cx).into_any_element(),
                SidebarPanel::Testing => self.render_testing(cx).into_any_element(),
            })
    }

    pub(super) fn toggle_terminal(
        &mut self,
        _: &ToggleTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.focus_handle(cx).is_focused(window) {
            self.focus_editor(window, cx);
        } else {
            self.focus_terminal(window, cx);
        }
    }

    pub(super) fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Terminal;
        let profile = settings::current(cx)
            .default_terminal_profile
            .and_then(|name| {
                Self::terminal_profiles(cx)
                    .into_iter()
                    .find(|profile| profile.name == name)
            });
        let options = self.profile_options(profile.as_ref(), None);
        let result = self
            .terminal
            .update(cx, |terminal, cx| terminal.start_with(options, cx));
        if let Err(error) = result {
            self.status = error;
        }
        window.focus(&self.terminal.focus_handle(cx));
        cx.notify();
    }

    pub(super) fn render_tool_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "tool-panel".into())
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(HEADER_HEIGHT))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::border())
                    .children(ToolPanel::ALL.into_iter().map(|panel| {
                        let active = self.active_panel == panel;
                        div()
                            .id(panel.label())
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .map(|tab| {
                                if active {
                                    tab.bg(theme::hover()).text_color(theme::text())
                                } else {
                                    tab.text_color(theme::muted())
                                        .hover(|style| style.text_color(theme::text()))
                                }
                            })
                            .on_click(cx.listener(move |this, _, window, cx| match panel {
                                ToolPanel::Terminal => this.focus_terminal(window, cx),
                                ToolPanel::Run => {
                                    this.active_panel = ToolPanel::Run;
                                    cx.notify();
                                }
                                ToolPanel::Debug => {
                                    this.show_debug_panel(&ShowDebugPanel, window, cx)
                                }
                                ToolPanel::Problems => {
                                    this.show_problems(&ShowProblems, window, cx)
                                }
                            }))
                            .child(panel.icon().size(px(14.)))
                            .child(panel.label())
                    })),
            )
            .child(match self.active_panel {
                ToolPanel::Terminal => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_terminal_tabs(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .bg(theme::background())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| this.focus_terminal(window, cx)),
                            )
                            .child(self.terminal.clone()),
                    )
                    .into_any_element(),
                ToolPanel::Run => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_task_tabs(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .bg(theme::background())
                            .child(self.selected_task_view()),
                    )
                    .into_any_element(),
                ToolPanel::Problems => self.render_problems(cx).into_any_element(),
                ToolPanel::Debug => div()
                    .flex_1()
                    .min_h_0()
                    .child(self.debugger.clone())
                    .into_any_element(),
            })
    }
}

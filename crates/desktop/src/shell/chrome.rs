//! Window chrome: the title bar (with its menu) and the status bar.

use super::*;

impl IdeShell {
    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (name, dirty) = self.active_tab(cx);
        TitleBar::new()
            .text_color(theme::text())
            .child(div().flex_1().min_w_0().flex().items_center().gap_2().when(
                !cfg!(target_os = "macos"),
                |row| {
                    row.child(
                        Button::new("app-menu")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(AppIcon::Menu))
                            .tooltip("Menu")
                            .dropdown_menu_with_anchor(Corner::TopLeft, |menu, window, cx| {
                                cx.get_menus().unwrap_or_default().into_iter().fold(
                                    menu.min_w(px(200.)),
                                    |menu, submenu| {
                                        let items = submenu.items;
                                        menu.submenu(
                                            submenu.name,
                                            window,
                                            cx,
                                            move |menu, window, cx| {
                                                app_menu_items(menu, items.clone(), window, cx)
                                            },
                                        )
                                    },
                                )
                            }),
                    )
                },
            ))
            .child(
                div()
                    .flex_shrink()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(div().font_weight(FontWeight::MEDIUM).child(name))
                    .when(dirty, |title| {
                        title.child(div().size(px(6.)).rounded_full().bg(theme::accent()))
                    })
                    .when(self.workspace.root().is_some(), |title| {
                        title
                            .child(div().text_color(theme::subtle()).child("—"))
                            .child(
                                div()
                                    .text_color(theme::muted())
                                    .child(self.workspace.display_name().to_string()),
                            )
                    }),
            )
            .child(div().flex_1())
            .on_close_window(cx.listener(|this, _, window, cx| {
                this.request(PendingAction::CloseWindow, window, cx)
            }))
    }

    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.buffer().dirty;
        let status = self
            .vim
            .as_ref()
            .and_then(VimInput::prompt)
            .unwrap_or_else(|| self.status.clone());
        let vim_mode = self.vim.as_ref().map(|vim| {
            let pending = vim.pending();
            if pending.is_empty() {
                vim.mode().label().to_string()
            } else {
                format!("{} {pending}", vim.mode().label())
            }
        });
        let divider = || div().w_px().h(px(12.)).flex_shrink_0().bg(theme::border());
        let server = (!self.show_diff)
            .then(|| self.servers.read(cx).state(self.buffer().language))
            .flatten();
        div()
            .h(px(26.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(theme::chrome())
            .border_t_1()
            .border_color(theme::border())
            .text_xs()
            .text_color(theme::muted())
            .child(
                status_item(
                    "quick-folder",
                    self.show_sidebar && self.active_sidebar == SidebarPanel::Folder,
                )
                .child(Icon::new(AppIcon::Files).size(px(13.)))
                .child("Files")
                .on_click(
                    cx.listener(|this, _, _, cx| this.toggle_sidebar(SidebarPanel::Folder, cx)),
                ),
            )
            .child(
                status_item(
                    "quick-git",
                    self.show_sidebar && self.active_sidebar == SidebarPanel::Git,
                )
                .child(Icon::new(AppIcon::GitBranch).size(px(13.)))
                .child(
                    self.git
                        .read(cx)
                        .branch_label()
                        .unwrap_or_else(|| "Git".into()),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(SidebarPanel::Git, cx))),
            )
            .when_some(
                self.active_root()
                    .filter(|_| self.workspace.roots().len() > 1)
                    .map(|root| self.workspace.label(root)),
                |bar, label| {
                    bar.child(
                        status_item("active-root", false)
                            .child(Icon::new(AppIcon::Files).size(px(13.)))
                            .child(format!("Root: {label}"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_active_root(&SelectActiveRoot, window, cx)
                            })),
                    )
                },
            )
            .child(
                status_item(
                    "quick-search",
                    self.show_sidebar && self.active_sidebar == SidebarPanel::Search,
                )
                .child(Icon::new(IconName::Search).size(px(13.)))
                .child("Search")
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.show_sidebar && this.active_sidebar == SidebarPanel::Search {
                        this.toggle_sidebar(SidebarPanel::Search, cx);
                    } else {
                        this.search_project(&SearchProject, window, cx);
                    }
                })),
            )
            .child(divider())
            .child(div().flex_1().min_w_0().px_1p5().truncate().child(status))
            .when_some(Self::auto_save_settings(cx).label(), |bar, label| {
                bar.child(div().flex_shrink_0().px_1p5().child(label))
            })
            .when_some(vim_mode, |bar, mode| {
                bar.child(
                    div()
                        .flex_shrink_0()
                        .px_1p5()
                        .py_px()
                        .rounded_sm()
                        .bg(theme::accent_wash())
                        .text_color(theme::accent())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(mode),
                )
            })
            .child(
                status_item("quick-terminal", self.active_panel == ToolPanel::Terminal)
                    .child(ToolPanel::Terminal.icon().size(px(13.)))
                    .child(ToolPanel::Terminal.label())
                    .on_click(cx.listener(|this, _, window, cx| this.focus_terminal(window, cx))),
            )
            .child(
                status_item("quick-debug", self.active_panel == ToolPanel::Debug)
                    .child(ToolPanel::Debug.icon().size(px(13.)))
                    .child(match self.debugger.read(cx).state() {
                        DebugState::Idle => ToolPanel::Debug.label().to_string(),
                        state => state.label(),
                    })
                    .when(self.debugger.read(cx).is_paused(), |item| {
                        item.text_color(theme::execution_line())
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_debug_panel(&ShowDebugPanel, window, cx)
                    })),
            )
            .when_some(server, |bar, (name, state)| {
                let failed = matches!(state, language_server::State::Failed(_));
                bar.child(
                    status_item("quick-language-server", false)
                        .when(failed, |item| {
                            item.text_color(theme::git_change(Change::Deleted))
                        })
                        .child(div().size(px(6.)).rounded_full().bg(match state {
                            language_server::State::Ready => theme::git_change(Change::Added),
                            language_server::State::Starting => theme::accent(),
                            language_server::State::Failed(_) => theme::git_change(Change::Deleted),
                        }))
                        .child(match &state {
                            language_server::State::Ready => name,
                            other => format!("{name} · {}", other.label()),
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.restart_servers(cx))),
                )
            })
            .when(!self.show_diff, |bar| {
                bar.child(divider())
                    .child(
                        status_item("quick-language-mode", false)
                            .child(ide_core::language::get(self.buffer().language).name)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_language_mode(&ChangeLanguageMode, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1p5()
                            .child(self.indentation_label()),
                    )
                    .child(
                        status_item("quick-encoding", false)
                            .child(self.document().encoding().label())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.document().path().is_some() {
                                    this.reopen_with_encoding(&ReopenWithEncoding, window, cx)
                                } else {
                                    this.save_with_encoding(&SaveWithEncoding, window, cx)
                                }
                            })),
                    )
                    .child(
                        status_item("quick-line-ending", false)
                            .child(self.document().line_ending().label())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_line_ending(&ChangeLineEnding, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1p5()
                            .px_1p5()
                            .child(div().size(px(6.)).rounded_full().bg(if dirty {
                                theme::accent()
                            } else {
                                theme::git_change(Change::Added)
                            }))
                            .child(if dirty { "Modified" } else { "Saved" }),
                    )
            })
    }
}

fn app_menu_items(
    mut menu: PopupMenu,
    items: Vec<OwnedMenuItem>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        menu = match item {
            OwnedMenuItem::Action { name, action, .. } => menu.menu(name, action),
            OwnedMenuItem::Separator => menu.separator(),
            OwnedMenuItem::Submenu(submenu) => {
                let items = submenu.items;
                menu.submenu(submenu.name, window, cx, move |menu, window, cx| {
                    app_menu_items(menu, items.clone(), window, cx)
                })
            }
            OwnedMenuItem::SystemMenu(_) => menu,
        };
    }
    menu
}

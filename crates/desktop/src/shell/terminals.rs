//! Multiple terminal sessions and shell profiles (IDE-024).
//!
//! Lifecycle policy: closing a session ends its process; other sessions keep running.
//! Switching workspaces leaves interactive terminals running in their current folders
//! (the task terminal and debugger are stopped as before); new terminals start in the
//! new workspace.

use crate::settings::{TerminalCwd, TerminalProfile};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;

pub(super) struct TerminalSession {
    pub id: u64,
    pub name: String,
    pub view: Entity<TerminalView>,
}

impl IdeShell {
    pub(super) fn terminal_profiles(cx: &App) -> Vec<TerminalProfile> {
        settings::current(cx).terminal_profiles
    }

    fn profile_named(name: Option<&str>, cx: &App) -> Option<TerminalProfile> {
        let name = name?;
        Self::terminal_profiles(cx)
            .into_iter()
            .find(|profile| profile.name == name)
    }

    pub(super) fn profile_options(
        &self,
        profile: Option<&TerminalProfile>,
        directory: Option<PathBuf>,
    ) -> terminal::Options {
        let workspace_directory = self.project_directory().map(Path::to_path_buf);
        let working_directory = directory.or_else(|| match profile.map(|profile| &profile.cwd) {
            Some(TerminalCwd::Home) => std::env::home_dir(),
            Some(TerminalCwd::Path(path)) if path.is_absolute() => Some(path.clone()),
            Some(TerminalCwd::Path(path)) => {
                workspace_directory.as_ref().map(|base| base.join(path))
            }
            Some(TerminalCwd::Workspace) | None => workspace_directory.clone(),
        });
        terminal::Options {
            shell: profile
                .filter(|profile| !profile.program.trim().is_empty())
                .map(|profile| (profile.program.clone(), profile.args.clone())),
            working_directory: working_directory.or_else(std::env::home_dir),
            env: profile
                .map(|profile| profile.env.clone())
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Create a terminal with `profile` (or the default profile) in `directory`.
    pub(super) fn create_terminal(
        &mut self,
        profile: Option<TerminalProfile>,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = profile.or_else(|| {
            Self::profile_named(
                settings::current(cx).default_terminal_profile.as_deref(),
                cx,
            )
        });
        let options = self.profile_options(profile.as_ref(), directory);
        let view = cx.new(TerminalView::new);
        let result = view.update(cx, |view, cx| view.start_with(options, cx));
        self.next_terminal_id += 1;
        let id = self.next_terminal_id;
        let base = profile
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "Terminal".into());
        let name = if self
            .terminal_sessions
            .iter()
            .any(|session| session.name == base)
        {
            format!("{base} {id}")
        } else {
            base
        };
        self.terminal_sessions.push(TerminalSession {
            id,
            name: name.clone(),
            view: view.clone(),
        });
        self.select_terminal(self.terminal_sessions.len() - 1, window, cx);
        self.status = match result {
            Ok(()) => format!("Started {name}"),
            Err(error) => match &profile {
                Some(profile) => format!("Terminal profile \"{}\": {error}", profile.name),
                None => error,
            },
        };
        cx.notify();
    }

    pub(super) fn new_terminal_in(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_terminal(None, directory, window, cx);
    }

    pub(super) fn new_terminal(
        &mut self,
        _: &NewTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_terminal(None, None, window, cx);
    }

    pub(super) fn select_terminal(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.terminal_sessions.get(ix) else {
            return;
        };
        self.active_terminal = ix;
        self.terminal = session.view.clone();
        self.active_panel = ToolPanel::Terminal;
        window.focus(&self.terminal.focus_handle(cx));
        cx.notify();
    }

    pub(super) fn next_terminal(
        &mut self,
        _: &NextTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_sessions.len() > 1 {
            let ix = (self.active_terminal + 1) % self.terminal_sessions.len();
            self.select_terminal(ix, window, cx);
        }
    }

    pub(super) fn close_terminal_at(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix >= self.terminal_sessions.len() {
            return;
        }
        let session = self.terminal_sessions.remove(ix);
        session.view.update(cx, |view, cx| view.terminate(cx));
        if self.terminal_sessions.is_empty() {
            // Keep one (unstarted) terminal so the panel always has a session.
            self.next_terminal_id += 1;
            self.terminal_sessions.push(TerminalSession {
                id: self.next_terminal_id,
                name: "Terminal".into(),
                view: cx.new(TerminalView::new),
            });
        }
        let ix = ix.min(self.terminal_sessions.len() - 1);
        self.active_terminal = ix;
        self.terminal = self.terminal_sessions[ix].view.clone();
        self.status = format!("Closed {}", session.name);
        let _ = window;
        cx.notify();
    }

    pub(super) fn close_terminal(
        &mut self,
        _: &CloseTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_terminal_at(self.active_terminal, window, cx);
    }

    pub(super) fn rename_terminal(
        &mut self,
        _: &RenameTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(session) = self.terminal_sessions.get(self.active_terminal) else {
            return;
        };
        let id = session.id;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(session.name.clone())
                .validate(|text, _| !text.contains('\n'))
        });
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let (shell, input) = (shell.clone(), input.clone());
                dialog
                    .title("Rename Terminal")
                    .w(px(360.))
                    .child(Input::new(&input))
                    .confirm()
                    .on_ok(move |_, _, cx| {
                        let name = input.read(cx).value().trim().to_string();
                        if name.is_empty() {
                            return false;
                        }
                        let _ = shell.update(cx, |shell, cx| {
                            if let Some(session) = shell
                                .terminal_sessions
                                .iter_mut()
                                .find(|session| session.id == id)
                            {
                                session.name = name.clone();
                            }
                            cx.notify();
                        });
                        true
                    })
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn select_terminal_profile(
        &mut self,
        _: &SelectTerminalProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profiles = Self::terminal_profiles(cx);
        let default = settings::current(cx).default_terminal_profile;
        let mut items = vec![PickerItem {
            label: "System shell".into(),
            detail: String::new(),
            filter: "system shell".into(),
            depth: 0,
            action: PickerAction::TerminalProfile(None),
        }];
        items.extend(profiles.iter().map(|profile| PickerItem {
            label: profile.name.clone(),
            detail: format!(
                "{}{}{}",
                if profile.program.is_empty() {
                    "default shell"
                } else {
                    &profile.program
                },
                if profile.args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", profile.args.join(" "))
                },
                if default.as_deref() == Some(profile.name.as_str()) {
                    " · default"
                } else {
                    ""
                }
            ),
            filter: profile.name.clone(),
            depth: 0,
            action: PickerAction::TerminalProfile(Some(profile.name.clone())),
        }));
        self.show_picker(
            PickerKind::Locations,
            "New terminal with profile".into(),
            items,
            window,
            cx,
        );
    }

    pub(super) fn start_profile(
        &mut self,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = match name {
            Some(name) => match Self::profile_named(Some(&name), cx) {
                Some(profile) => Some(profile),
                None => {
                    self.status = format!("Terminal profile \"{name}\" is not configured");
                    cx.notify();
                    return;
                }
            },
            None => Some(TerminalProfile {
                name: "Terminal".into(),
                ..Default::default()
            }),
        };
        self.create_terminal(profile, None, window, cx);
    }

    pub(super) fn render_terminal_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(26.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .text_xs()
            .children(
                self.terminal_sessions
                    .iter()
                    .enumerate()
                    .map(|(ix, session)| {
                        let active = ix == self.active_terminal;
                        let state = match session.view.read(cx).process_state() {
                            None if session.view.read(cx).error().is_some() => " · failed",
                            None => "",
                            Some(true) => "",
                            Some(false) => " · exited",
                        };
                        div()
                            .id(("terminal-tab", session.id))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .map(|tab| {
                                if active {
                                    tab.bg(theme::hover()).text_color(theme::text())
                                } else {
                                    tab.text_color(theme::muted())
                                }
                            })
                            .child(format!("{}{state}", session.name))
                            .child(
                                div()
                                    .id(("terminal-close", session.id))
                                    .text_color(theme::subtle())
                                    .hover(|style| style.text_color(theme::text()))
                                    .child(Icon::new(IconName::Close).size(px(10.)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_terminal_at(ix, window, cx)
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_terminal(ix, window, cx)
                            }))
                    }),
            )
            .child(
                Button::new("terminal-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New Terminal")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_terminal(&NewTerminal, window, cx)
                    })),
            )
            .child(
                Button::new("terminal-profiles")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip("New Terminal with Profile…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.select_terminal_profile(&SelectTerminalProfile, window, cx)
                    })),
            )
    }
}

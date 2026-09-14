use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use gpui::{
    Context, Div, Entity, Focusable, IntoElement, MouseButton, PathPromptOptions, Pixels, Render,
    Stateful, Subscription, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Root, ThemeMode, TitleBar, WindowExt as _,
    button::Button,
    input::{Input, InputEvent, InputState},
    menu::AppMenuBar,
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    switch::Switch,
};
use ide_core::{
    document::Document,
    workspace::{self, TreeRow, Workspace},
};

use crate::{commands::*, terminal_view::TerminalView, theme};

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ToolPanel {
    #[default]
    Terminal,
    Git,
    Debug,
}

impl ToolPanel {
    const ALL: [Self; 3] = [Self::Terminal, Self::Git, Self::Debug];

    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Git => "Git",
            Self::Debug => "Debug",
        }
    }

    /// Shown instead of panels that are not implemented yet.
    fn placeholder(self) -> Option<&'static str> {
        match self {
            Self::Terminal => None,
            Self::Git => Some("Repository status and diffs are not implemented yet."),
            Self::Debug => Some("Debug adapter integration is not implemented yet."),
        }
    }
}

#[derive(Clone)]
enum PendingAction {
    New,
    Open,
    /// Open a file chosen in the project tree.
    OpenPath(PathBuf),
    Close,
}

pub struct IdeShell {
    document: Document,
    editor: Entity<InputState>,
    editor_subscription: Option<Subscription>,
    workspace: Workspace,
    /// `workspace.rows()`, rebuilt whenever the tree changes rather than per frame.
    tree_rows: Vec<TreeRow>,
    tree_scroll: UniformListScrollHandle,
    /// In-window menu bar; `None` on macOS, where the menus live in the system menu bar.
    menu_bar: Option<Entity<AppMenuBar>>,
    /// Drag state for the sidebar | editor split.
    sidebar_split: Entity<ResizableState>,
    /// Drag state for the editor / tool panel split.
    tool_panel_split: Entity<ResizableState>,
    active_panel: ToolPanel,
    terminal: Entity<TerminalView>,
    pending: Option<PendingAction>,
    busy: bool,
    dirty: bool,
    status: String,
}

impl IdeShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = Self::make_editor(&Document::default(), window, cx);
        let mut shell = Self {
            document: Document::default(),
            editor,
            editor_subscription: None,
            workspace: Workspace::default(),
            tree_rows: Vec::new(),
            tree_scroll: UniformListScrollHandle::new(),
            menu_bar: (!cfg!(target_os = "macos")).then(|| AppMenuBar::new(window, cx)),
            sidebar_split: cx.new(|_| ResizableState::default()),
            tool_panel_split: cx.new(|_| ResizableState::default()),
            active_panel: ToolPanel::default(),
            terminal: cx.new(TerminalView::new),
            pending: None,
            busy: false,
            dirty: false,
            status: "Ready — open a file or start typing".into(),
        };
        shell.subscribe_editor(window, cx);
        shell
    }

    fn make_editor(
        document: &Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            let editor = InputState::new(window, cx)
                .code_editor("plain_text")
                .line_number(true)
                .soft_wrap(false)
                .default_value(document.text().to_string());
            editor.focus(window, cx);
            editor
        })
    }

    fn subscribe_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_subscription =
            Some(
                cx.subscribe_in(&self.editor, window, |this, editor, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.document.set_text(editor.read(cx).text().clone());
                        this.dirty = this.document.is_dirty();
                        cx.notify();
                    }
                }),
            );
    }

    fn install_document(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A fresh input entity gives each document its own undo history and selection.
        self.editor = Self::make_editor(&document, window, cx);
        self.document = document;
        self.dirty = false;
        self.subscribe_editor(window, cx);
        cx.notify();
    }

    pub fn can_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if self.dirty {
            self.request(PendingAction::Close, window, cx);
            false
        } else {
            true
        }
    }

    fn request(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending.is_some() {
            return;
        }
        if self.dirty {
            self.pending = Some(action);
            cx.notify();
        } else {
            self.perform(action, window, cx);
        }
    }

    fn perform(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            PendingAction::New => {
                self.install_document(Document::default(), window, cx);
                self.status = "New document".into();
            }
            PendingAction::Open => self.open(None, window, cx),
            PendingAction::OpenPath(path) => self.open(Some(path), window, cx),
            PendingAction::Close => window.remove_window(),
        }
    }

    /// Open `path`, or ask for a file when it is `None`.
    fn open(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        let picker = if path.is_none() {
            self.status = "Choose a UTF-8 text file…".into();
            Some(cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open file".into()),
            }))
        } else {
            self.status = "Opening…".into();
            None
        };
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let path = if let Some(picker) = picker {
                    picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                        .and_then(|paths| paths.into_iter().next())
                } else {
                    path
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        Document::open(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(document)) => {
                        this.status = format!("Opened {}", document.name());
                        this.install_document(document, window, cx);
                    }
                    Ok(None) => this.status = "Open cancelled".into(),
                    Err(error) => this.status = format!("Open failed: {error}"),
                }
                this.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
        let existing_path = if save_as {
            None
        } else {
            self.document.path().map(PathBuf::from)
        };
        let picker = if existing_path.is_none() {
            let directory = self
                .document
                .path()
                .and_then(|path| path.parent())
                .or(self.workspace.root())
                .unwrap_or(Path::new("."));
            Some(cx.prompt_for_new_path(directory, Some(&self.document.name())))
        } else {
            None
        };
        let snapshot = self.document.clone();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let path = if let Some(picker) = picker {
                    picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                } else {
                    existing_path
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        snapshot
                            .save_to(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(saved)) => {
                        this.document.accept_saved(saved);
                        this.dirty = this.document.is_dirty();
                        this.status = format!("Saved {}", this.document.name());
                        // Show a newly created file in the project tree.
                        if let Some(parent) = this.document.path().and_then(Path::parent)
                            && this.workspace.is_expanded(parent)
                        {
                            this.load_directory(parent.to_path_buf(), cx);
                        }
                        if !this.dirty
                            && let Some(action) = this.pending.take()
                        {
                            this.perform(action, window, cx);
                        }
                    }
                    Ok(None) => this.status = "Save cancelled".into(),
                    Err(error) => this.status = format!("Save failed: {error}"),
                }
                this.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending.is_some() {
            return;
        }
        self.busy = true;
        self.status = "Choose a folder…".into();
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open folder".into()),
        });
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Workspace>, String> = async {
                let selected = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = selected.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        Workspace::open(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(workspace)) => this.set_workspace(workspace, cx),
                    Ok(None) => this.status = "Open folder cancelled".into(),
                    Err(error) => this.status = format!("Open folder failed: {error}"),
                }
                this.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn set_workspace(&mut self, workspace: Workspace, cx: &mut Context<Self>) {
        self.status = format!("Opened folder {}", workspace.display_name());
        self.workspace = workspace;
        self.tree_rows = self.workspace.rows();
        self.tree_scroll = UniformListScrollHandle::new();
        cx.notify();
    }

    /// Read `directory` in the background, then expand it in the project tree.
    fn load_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let listing = executor
                .spawn({
                    let directory = directory.clone();
                    async move { workspace::read_dir(&directory) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match listing {
                    Ok(listing) => {
                        if this.workspace.expand(directory, listing) {
                            this.tree_rows = this.workspace.rows();
                        }
                    }
                    Err(error) => {
                        this.status = format!("Could not read {}: {error}", directory.display())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn activate_tree_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.tree_rows.get(ix) else {
            return;
        };
        let path = row.entry.path.clone();
        if !row.entry.is_dir {
            if self.document.path() != Some(path.as_path()) {
                self.request(PendingAction::OpenPath(path), window, cx);
            }
        } else if row.expanded {
            self.workspace.collapse(&path);
            self.tree_rows = self.workspace.rows();
            cx.notify();
        } else {
            self.load_directory(path, cx);
        }
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::New, window, cx);
    }

    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::Open, window, cx);
    }

    fn save_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        self.save(false, window, cx);
    }

    fn save_file_as(&mut self, _: &SaveFileAs, window: &mut Window, cx: &mut Context<Self>) {
        self.save(true, window, cx);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::Close, window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("Settings").w(px(420.)).child(
                div().debug_selector(|| "light-mode".into()).child(
                    Switch::new("light-mode")
                        .label("Light mode")
                        .checked(!theme::mode().is_dark())
                        .on_click(|light, window, cx| {
                            let mode = if *light {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            theme::set_mode(mode, Some(window), cx);
                        }),
                ),
            )
        });
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_root = self.workspace.root().is_some();
        div()
            .debug_selector(|| "sidebar".into())
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                div()
                    .h(px(34.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px_4()
                    .text_color(theme::muted())
                    .child(if has_root {
                        self.workspace.display_name().to_string()
                    } else {
                        "PROJECT".into()
                    }),
            )
            .map(|sidebar| {
                if has_root {
                    sidebar.child(
                        div().flex_1().min_h_0().child(
                            uniform_list(
                                "project-tree",
                                self.tree_rows.len(),
                                cx.processor(|this, range: Range<usize>, _, cx| {
                                    range
                                        .map(|ix| this.render_tree_row(ix, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .size_full()
                            .track_scroll(self.tree_scroll.clone()),
                        ),
                    )
                } else {
                    sidebar.child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap_3()
                            .px_4()
                            .child(div().text_color(theme::muted()).child("No folder open"))
                            .child(
                                Button::new("open-folder")
                                    .label("Open Folder…")
                                    .disabled(self.busy || self.pending.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_folder(&OpenFolder, window, cx)
                                    })),
                            ),
                    )
                }
            })
    }

    fn render_tree_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = &self.tree_rows[ix];
        let is_open = !row.entry.is_dir && self.document.path() == Some(row.entry.path.as_path());
        let (chevron, icon) = match (row.entry.is_dir, row.expanded) {
            (true, true) => (Some(IconName::ChevronDown), IconName::FolderOpen),
            (true, false) => (Some(IconName::ChevronRight), IconName::Folder),
            (false, _) => (None, IconName::File),
        };
        div()
            .id(ix)
            .w_full()
            .h(px(24.))
            .flex()
            .items_center()
            .gap_1()
            .pl(px(8. + 12. * row.depth as f32))
            .pr_2()
            .cursor_pointer()
            .hover(|style| style.bg(theme::border()))
            .when(is_open, |row| {
                row.bg(theme::border()).text_color(theme::accent())
            })
            .child(
                div()
                    .size(px(14.))
                    .flex_shrink_0()
                    .when_some(chevron, |slot, chevron| {
                        slot.child(Icon::new(chevron).size_full().text_color(theme::muted()))
                    }),
            )
            .child(
                Icon::new(icon)
                    .size(px(14.))
                    .flex_shrink_0()
                    .text_color(theme::muted()),
            )
            .child(div().min_w_0().truncate().child(row.entry.name.clone()))
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate_tree_row(ix, window, cx)),
            )
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.focus_handle(cx).is_focused(window) {
            self.editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        } else {
            self.focus_terminal(window, cx);
        }
    }

    /// Show and focus the terminal, starting a shell in the project folder if none is running.
    fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Terminal;
        let directory = self
            .workspace
            .root()
            .or_else(|| self.document.path().and_then(Path::parent))
            .map(Path::to_path_buf);
        self.terminal
            .update(cx, |terminal, cx| terminal.start(directory, cx));
        window.focus(&self.terminal.focus_handle(cx));
        cx.notify();
    }

    fn render_tool_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "tool-panel".into())
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .bg(theme::panel())
                    .children(ToolPanel::ALL.into_iter().map(|panel| {
                        div()
                            .id(panel.label())
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .text_color(if self.active_panel == panel {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if panel == ToolPanel::Terminal {
                                    this.focus_terminal(window, cx);
                                } else {
                                    this.active_panel = panel;
                                    cx.notify();
                                }
                            }))
                            .child(panel.label())
                    })),
            )
            .child(match self.active_panel.placeholder() {
                Some(placeholder) => div()
                    .p_3()
                    .text_color(theme::muted())
                    .child(placeholder)
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus_terminal(window, cx)),
                    )
                    .child(self.terminal.clone())
                    .into_any_element(),
            })
    }
}

impl Render for IdeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = format!(
            "{}{}",
            self.document.name(),
            if self.dirty { " •" } else { "" }
        );
        window.set_window_title(&match self.workspace.root() {
            Some(_) => format!("{name} — {} — Hephaestus", self.workspace.display_name()),
            None => format!("{name} — Hephaestus"),
        });
        let blocked = self.busy || self.pending.is_some();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_color(theme::text())
            .text_sm()
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::save_file))
            .on_action(cx.listener(Self::save_file_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_terminal))
            .child(
                TitleBar::new()
                    .bg(theme::panel())
                    .text_color(theme::text())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .when_some(self.menu_bar.clone(), |row, menu_bar| {
                                row.child(div().flex_shrink_0().child(menu_bar))
                            })
                            .child(div().text_color(theme::muted()).child(name.clone())),
                    )
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        this.request(PendingAction::Close, window, cx)
                    })),
            )
            .when(self.pending.is_some(), |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .bg(theme::panel())
                        .border_b_1()
                        .border_color(theme::accent())
                        .child(div().flex_1().child("Save changes before continuing?"))
                        .child(
                            Button::new("confirm-save")
                                .label("Save")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                                ),
                        )
                        .child(
                            Button::new("discard")
                                .label("Discard")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(action) = this.pending.take() {
                                        this.perform(action, window, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pending = None;
                                    this.editor
                                        .update(cx, |editor, cx| editor.focus(window, cx));
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("sidebar-split")
                        .with_state(&self.sidebar_split)
                        .child(
                            resizable_panel()
                                .size(px(220.))
                                .size_range(px(140.)..px(480.))
                                .child(self.render_sidebar(cx)),
                        )
                        .child(
                            v_resizable("tool-panel-split")
                                .with_state(&self.tool_panel_split)
                                .child(
                                    resizable_panel().child(
                                        div()
                                            .size_full()
                                            .min_w_0()
                                            .min_h_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .h(px(34.))
                                                    .flex_shrink_0()
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px_4()
                                                    .border_b_1()
                                                    .border_color(theme::border())
                                                    .child(div().flex_shrink_0().child(name))
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .truncate()
                                                            .text_color(theme::muted())
                                                            .child(
                                                                self.document
                                                                    .path()
                                                                    .map(|path| {
                                                                        path.display().to_string()
                                                                    })
                                                                    .unwrap_or_else(|| {
                                                                        "Not saved to disk".into()
                                                                    }),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div().flex_1().min_h_0().overflow_hidden().child(
                                                    Input::new(&self.editor)
                                                        .h_full()
                                                        .w_full()
                                                        .bordered(false)
                                                        .focus_bordered(false)
                                                        .appearance(false)
                                                        .disabled(blocked)
                                                        .font_family(
                                                            if cfg!(target_os = "windows") {
                                                                "Consolas"
                                                            } else {
                                                                "monospace"
                                                            },
                                                        )
                                                        .text_size(px(14.))
                                                        .bg(theme::background())
                                                        .text_color(theme::text()),
                                                ),
                                            ),
                                    ),
                                )
                                .child(
                                    resizable_panel()
                                        .size(px(180.))
                                        .size_range(px(72.)..Pixels::MAX)
                                        .child(self.render_tool_panel(cx)),
                                ),
                        ),
                ),
            )
            .child(
                div()
                    .min_h(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .bg(theme::panel())
                    .border_t_1()
                    .border_color(theme::border())
                    .text_color(theme::muted())
                    .child(div().flex_1().min_w_0().child(self.status.clone()))
                    .child(format!(
                        "{} | UTF-8 | {}",
                        if self.dirty { "Modified" } else { "Saved" },
                        self.document.line_ending().label()
                    )),
            )
            .children(Root::render_dialog_layer(window, cx))
    }
}

use std::{
    cell::Cell,
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::{
    Action, App, ClickEvent, Context, Div, ElementId, Entity, EntityInputHandler as _, Focusable,
    FontWeight, IntoElement, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent,
    PathPromptOptions, Pixels, Render, Stateful, Subscription, Task, TextRun,
    UniformListScrollHandle, Window, canvas, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Root, RopeExt as _, Sizable as _, ThemeMode, TitleBar,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, Position, TabSize},
    menu::AppMenuBar,
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    switch::Switch,
    tooltip::Tooltip,
};
use ide_core::{
    document::Document,
    git::{self, Change, Repository},
    vim::Command as VimCommand,
    workspace::{self, TreeRow, Workspace},
};

use crate::{
    assets::AppIcon,
    brace_guide::BraceGuide,
    commands::*,
    diff_view::DiffView,
    folding::{self, Fold, Toggle},
    git_panel::{GitPanel, GitPanelEvent},
    navigation::{self, Hit, Lookup, Search},
    settings::{self, Settings},
    syntax,
    terminal_view::TerminalView,
    theme,
    ui::{self, HEADER_HEIGHT},
    vim::VimInput,
};

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

pub const EDITOR_CONTEXT: &str = "Editor";
const INDENT_WIDTH: usize = 2;
const EDITOR_TEXT_SIZE: Pixels = px(14.);
const EDITOR_PADDING_LEFT: Pixels = px(12.);
const LINE_NUMBER_PADDING: Pixels = px(6.);

fn position_at(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |ix| ix + 1);
    let character = before[line_start..].encode_utf16().count() as u32;
    Position::new(line, character)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SidebarPanel {
    #[default]
    Folder,
    Git,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ToolPanel {
    #[default]
    Terminal,
    Debug,
}

impl ToolPanel {
    const ALL: [Self; 2] = [Self::Terminal, Self::Debug];

    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Debug => "Debug",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Terminal => Icon::new(IconName::SquareTerminal),
            Self::Debug => Icon::new(AppIcon::Bug),
        }
    }
}

const ACTIVITY_BAR_WIDTH: f32 = 44.;
const TAB_GROUP: &str = "tab";

fn activity_item(
    id: &'static str,
    icon: Icon,
    active: bool,
    tooltip: &'static str,
    action: Box<dyn Action>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .size(px(34.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(theme::muted())
        .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
        .when(active, |item| {
            item.text_color(theme::text()).child(
                div()
                    .absolute()
                    .left(px(-5.))
                    .top(px(8.))
                    .bottom(px(8.))
                    .w(px(2.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
        })
        .child(icon.size(px(18.)))
        .tooltip(move |window, cx| {
            Tooltip::new(tooltip)
                .action(action.as_ref(), None)
                .build(window, cx)
        })
        .on_click(on_click)
}

fn status_item(id: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(18.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .px_1p5()
        .rounded_sm()
        .cursor_pointer()
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
}

fn editor_tab(id: impl Into<ElementId>, active: bool, icon: Icon, title: String) -> Stateful<Div> {
    div()
        .id(id)
        .group(TAB_GROUP)
        .relative()
        .h_full()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .pl_3()
        .pr_1p5()
        .border_r_1()
        .border_color(theme::border())
        .cursor_pointer()
        .map(|tab| {
            if active {
                tab.bg(theme::background()).text_color(theme::text()).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(theme::accent()),
                )
            } else {
                tab.text_color(theme::muted())
                    .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            }
        })
        .child(icon.size(px(14.)).flex_shrink_0().text_color(if active {
            theme::accent()
        } else {
            theme::subtle()
        }))
        .child(title)
}

fn tab_close_button(id: impl Into<ElementId>, hidden: bool) -> Stateful<Div> {
    div()
        .id(id)
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_color(theme::muted())
        .when(hidden, |close| {
            close
                .invisible()
                .group_hover(TAB_GROUP, |style| style.visible())
        })
        .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
        .child(Icon::new(IconName::Close).size(px(12.)))
}

type BufferId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAction {
    CloseBuffer(BufferId),
    CloseWindow,
}

struct Buffer {
    id: BufferId,
    document: Document,
    editor: Entity<InputState>,
    dirty: bool,
    folds: Vec<Fold>,
    _subscription: Subscription,
}

impl Buffer {
    fn is_pristine(&self) -> bool {
        self.document.path().is_none() && !self.dirty && self.document.text().len() == 0
    }
}

enum SaveTarget {
    Current,
    Ask,
    Path(PathBuf),
}

pub struct IdeShell {
    buffers: Vec<Buffer>,
    active: usize,
    next_buffer_id: BufferId,
    workspace: Workspace,
    tree_rows: Vec<TreeRow>,
    tree_scroll: UniformListScrollHandle,
    menu_bar: Option<Entity<AppMenuBar>>,
    sidebar_split: Entity<ResizableState>,
    tool_panel_split: Entity<ResizableState>,
    active_sidebar: SidebarPanel,
    active_panel: ToolPanel,
    terminal: Entity<TerminalView>,
    git: Entity<GitPanel>,
    diff_view: Entity<DiffView>,
    show_diff: bool,
    pending: Option<PendingAction>,
    busy: bool,
    cloning: bool,
    vim: Option<VimInput>,
    status: String,
    declaration_search: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl IdeShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let git = cx.new(|cx| GitPanel::new(window, cx));
        let diff_view = git.read(cx).diff_view().clone();
        let mut shell = Self {
            buffers: Vec::new(),
            active: 0,
            next_buffer_id: 0,
            workspace: Workspace::default(),
            tree_rows: Vec::new(),
            tree_scroll: UniformListScrollHandle::new(),
            menu_bar: (!cfg!(target_os = "macos")).then(|| AppMenuBar::new(window, cx)),
            sidebar_split: cx.new(|_| ResizableState::default()),
            tool_panel_split: cx.new(|_| ResizableState::default()),
            active_sidebar: SidebarPanel::default(),
            active_panel: ToolPanel::default(),
            terminal: cx.new(TerminalView::new),
            git,
            diff_view,
            show_diff: false,
            pending: None,
            busy: false,
            cloning: false,
            vim: None,
            status: "Ready — open a file or start typing".into(),
            declaration_search: Task::ready(()),
            _subscriptions: Vec::new(),
        };
        shell._subscriptions = vec![
            cx.subscribe_in(
                &shell.git,
                window,
                |this, _, event, window, cx| match event {
                    GitPanelEvent::OpenFile(path) => {
                        if !this.blocked() {
                            this.open(Some(path.clone()), window, cx);
                        }
                    }
                    GitPanelEvent::ShowDiff => {
                        if !this.blocked() {
                            this.focus_diff(window, cx);
                        }
                    }
                    GitPanelEvent::Status(status) => {
                        this.status = status.clone();
                        cx.notify();
                    }
                },
            ),
            cx.observe(&shell.git, |_, _, cx| cx.notify()),
            cx.observe_in(&shell.diff_view, window, |this, diff_view, window, cx| {
                if this.show_diff && diff_view.read(cx).target().is_none() {
                    this.show_diff = false;
                    this.focus_editor(window, cx);
                }
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_git(cx);
                }
            }),
        ];
        let buffer = shell.new_buffer(Document::default(), window, cx);
        shell.buffers.push(buffer);
        shell.activate(0, window, cx);
        shell
    }

    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    fn buffer(&self) -> &Buffer {
        &self.buffers[self.active]
    }

    fn document(&self) -> &Document {
        &self.buffer().document
    }

    fn editor(&self) -> &Entity<InputState> {
        &self.buffer().editor
    }

    fn buffer_index(&self, id: BufferId) -> Option<usize> {
        self.buffers.iter().position(|buffer| buffer.id == id)
    }

    fn buffer_for_path(&self, path: &Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|buffer| buffer.document.path() == Some(path))
    }

    fn blocked(&self) -> bool {
        self.busy || self.pending.is_some()
    }

    fn new_buffer(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Buffer {
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(syntax::language(document.path()))
                .line_number(true)
                .soft_wrap(false)
                .tab_size(TabSize {
                    tab_size: INDENT_WIDTH,
                    hard_tabs: false,
                })
                .default_value(document.text().to_string())
        });
        let subscription = cx.subscribe_in(&editor, window, move |this, editor, event, _, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(ix) = this.buffer_index(id)
            {
                let buffer = &mut this.buffers[ix];
                let displayed = editor.read(cx).text().to_string();
                let source = buffer.document.text().to_string();
                if let Some(text) = folding::apply_edit(&source, &mut buffer.folds, &displayed) {
                    buffer.document.set_text(text.into());
                }
                buffer.dirty = buffer.document.is_dirty();
                cx.notify();
            }
        });
        Buffer {
            id,
            document,
            editor,
            dirty: false,
            folds: Vec::new(),
            _subscription: subscription,
        }
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        self.show_diff = false;
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        if let Some(vim) = &mut self.vim {
            vim.reset(&editor, cx);
        }
        self.sync_git(cx);
        cx.notify();
    }

    fn project_directory(&self) -> Option<&Path> {
        self.workspace
            .root()
            .or_else(|| self.document().path().and_then(Path::parent))
    }

    fn sync_git(&mut self, cx: &mut Context<Self>) -> bool {
        let directory = self.project_directory().map(Path::to_path_buf);
        self.git
            .update(cx, |git, cx| git.set_directory(directory, cx))
    }

    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if !self.sync_git(cx) {
            self.git.update(cx, |git, cx| git.refresh(cx));
        }
    }

    fn show_folder_panel(&mut self, _: &ShowFolderPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.active_sidebar = SidebarPanel::Folder;
        cx.notify();
    }

    fn show_git_panel(&mut self, _: &ShowGitPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.active_sidebar = SidebarPanel::Git;
        self.refresh_git(cx);
        cx.notify();
    }

    fn forward_to_git<A: Action>(
        handler: fn(&mut GitPanel, &A, &mut Window, &mut Context<GitPanel>),
    ) -> impl Fn(&mut Self, &A, &mut Window, &mut Context<Self>) {
        move |this, action, window, cx| {
            this.git
                .update(cx, |git, cx| handler(git, action, window, cx))
        }
    }

    fn show_debug_panel(&mut self, _: &ShowDebugPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Debug;
        cx.notify();
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor()
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    fn open_document(&mut self, document: Document, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = document.path().and_then(|path| self.buffer_for_path(path)) {
            self.activate(ix, window, cx);
            return;
        }
        let buffer = self.new_buffer(document, window, cx);
        if self.buffer().is_pristine() {
            self.buffers[self.active] = buffer;
        } else {
            self.active += 1;
            self.buffers.insert(self.active, buffer);
        }
        self.activate(self.active, window, cx);
    }

    fn remove_buffer(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.buffers.remove(ix);
        if self.buffers.is_empty() {
            let buffer = self.new_buffer(Document::default(), window, cx);
            self.buffers.push(buffer);
        }
        let active = if self.active > ix {
            self.active - 1
        } else {
            self.active.min(self.buffers.len() - 1)
        };
        let show_diff = self.show_diff;
        self.activate(active, window, cx);
        if show_diff {
            self.focus_diff(window, cx);
        }
    }

    fn focus_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_diff = true;
        window.focus(&self.diff_view.focus_handle(cx));
        cx.notify();
    }

    fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_diff = false;
        self.git.update(cx, |git, cx| git.clear_selection(cx));
        self.focus_editor(window, cx);
        cx.notify();
    }

    pub fn can_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if self.buffers.iter().any(|buffer| buffer.dirty) {
            self.request(PendingAction::CloseWindow, window, cx);
            false
        } else {
            true
        }
    }

    fn request(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        if !self.blocked() {
            self.perform(action, window, cx);
        }
    }

    fn perform(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        let unsaved = match action {
            PendingAction::CloseBuffer(id) => {
                self.buffer_index(id).filter(|&ix| self.buffers[ix].dirty)
            }
            PendingAction::CloseWindow => self.buffers.iter().position(|buffer| buffer.dirty),
        };
        if let Some(ix) = unsaved {
            self.activate(ix, window, cx);
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
            PendingAction::CloseWindow => window.remove_window(),
        }
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.pending.take() else {
            return;
        };
        self.remove_buffer(self.active, window, cx);
        if action == PendingAction::CloseWindow {
            self.perform(action, window, cx);
        }
        cx.notify();
    }

    fn open(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = path.as_deref().and_then(|path| self.buffer_for_path(path)) {
            self.activate(ix, window, cx);
            return;
        }
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
                        this.open_document(document, window, cx);
                    }
                    Ok(None) => this.status = "Open cancelled".into(),
                    Err(error) => this.status = format!("Open failed: {error}"),
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn save(
        &mut self,
        target: SaveTarget,
        then: Option<PendingAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let buffer = self.buffer();
        let id = buffer.id;
        let existing_path = match target {
            SaveTarget::Current => buffer.document.path().map(PathBuf::from),
            SaveTarget::Ask => None,
            SaveTarget::Path(path) => Some(path),
        };
        let picker = if existing_path.is_none() {
            let directory = buffer
                .document
                .path()
                .and_then(|path| path.parent())
                .or(self.workspace.root())
                .unwrap_or(Path::new("."));
            Some(cx.prompt_for_new_path(directory, Some(&buffer.document.name())))
        } else {
            None
        };
        let snapshot = buffer.document.clone();
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
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
                        let saved_path = saved.path().map(Path::to_path_buf);
                        if let Some(ix) = this.buffer_index(id) {
                            let buffer = &mut this.buffers[ix];
                            let language = syntax::language(buffer.document.path());
                            buffer.document.accept_saved(saved);
                            buffer.dirty = buffer.document.is_dirty();
                            this.status = format!("Saved {}", buffer.document.name());
                            let saved_language = syntax::language(buffer.document.path());
                            if saved_language != language {
                                buffer.editor.update(cx, |editor, cx| {
                                    editor.set_highlighter(saved_language, cx)
                                });
                            }
                        }
                        this.refresh_parent(saved_path.as_deref(), cx);
                        this.refresh_git(cx);
                        if saved_path
                            .as_deref()
                            .is_some_and(|path| settings::is_settings_file(path, cx))
                        {
                            this.status = this.apply_settings(window, cx);
                        }
                        let clean = this
                            .buffer_index(id)
                            .is_some_and(|ix| !this.buffers[ix].dirty);
                        if clean && let Some(action) = then.or_else(|| this.pending.take()) {
                            this.perform(action, window, cx);
                        }
                    }
                    Ok(None) => this.status = "Save cancelled".into(),
                    Err(error) => this.status = format!("Save failed: {error}"),
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn save_all(
        &mut self,
        then: Option<PendingAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let snapshots: Vec<_> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.dirty)
            .filter_map(|buffer| {
                let path = buffer.document.path()?.to_path_buf();
                Some((buffer.id, buffer.document.clone(), path))
            })
            .collect();
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let results = executor
                .spawn(async move {
                    snapshots
                        .into_iter()
                        .map(|(id, snapshot, path)| {
                            let result = snapshot
                                .save_to(&path)
                                .map_err(|error| format!("{}: {error}", path.display()));
                            (id, result)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                let mut failures = Vec::new();
                let mut settings_saved = false;
                let saved = results.iter().filter(|(_, result)| result.is_ok()).count();
                for (id, result) in results {
                    match result {
                        Ok(saved) => {
                            settings_saved |= saved
                                .path()
                                .is_some_and(|path| settings::is_settings_file(path, cx));
                            if let Some(ix) = this.buffer_index(id) {
                                let buffer = &mut this.buffers[ix];
                                buffer.document.accept_saved(saved);
                                buffer.dirty = buffer.document.is_dirty();
                            }
                        }
                        Err(error) => failures.push(error),
                    }
                }
                if saved > 0 {
                    this.refresh_git(cx);
                }
                let settings_status = settings_saved.then(|| this.apply_settings(window, cx));
                match failures.first() {
                    Some(error) => this.status = format!("Save failed: {error}"),
                    None => {
                        let plural = if saved == 1 { "" } else { "s" };
                        this.status = settings_status
                            .unwrap_or_else(|| format!("Saved {saved} file{plural}"));
                        if let Some(action) = then {
                            this.perform(action, window, cx);
                        }
                    }
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_parent(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        if let Some(parent) = path.and_then(Path::parent)
            && self.workspace.is_expanded(parent)
        {
            self.load_directory(parent.to_path_buf(), cx);
        }
    }

    fn open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
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
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn clone_repository(
        &mut self,
        _: &CloneRepository,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || self.cloning || window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        let url = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://github.com/owner/repository.git")
        });
        window.open_dialog(cx, {
            let url = url.clone();
            move |dialog, _, _| {
                let (shell, url) = (shell.clone(), url.clone());
                dialog
                    .title("Clone Repository")
                    .w(px(440.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .pb_1()
                            .child(ui::caption("REPOSITORY URL"))
                            .child(Input::new(&url))
                            .child(div().text_xs().text_color(theme::muted()).child(
                                "Next, choose the folder to clone into. \
                                 The repository opens once it has been cloned.",
                            )),
                    )
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Clone"))
                    .on_ok(move |_, window, cx| {
                        let url = url.read(cx).value().trim().to_string();
                        if git::clone_folder_name(&url).is_none() {
                            return false;
                        }
                        let _ = shell
                            .update(cx, |shell, cx| shell.choose_clone_location(url, window, cx));
                        true
                    })
            }
        });
        url.update(cx, |url, cx| url.focus(window, cx));
    }

    fn choose_clone_location(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() || self.cloning {
            return;
        }
        self.busy = true;
        self.status = "Choose a folder to clone into…".into();
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Clone here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let parent = picker
                .await
                .map_err(|e| e.to_string())
                .and_then(|selected| selected.map_err(|e| e.to_string()))
                .map(|selected| selected.and_then(|paths| paths.into_iter().next()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match parent {
                    Ok(Some(parent)) => this.clone_into(url, parent, cx),
                    Ok(None) => this.status = "Clone cancelled".into(),
                    Err(error) => this.status = format!("Clone failed: {error}"),
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn clone_into(&mut self, url: String, parent: PathBuf, cx: &mut Context<Self>) {
        self.cloning = true;
        self.status = format!("Cloning {url}…");
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = executor
                .spawn(async move {
                    Repository::clone_remote(&url, &parent)
                        .and_then(|repository| Workspace::open(repository.root()))
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloning = false;
                match result {
                    Ok(workspace) => {
                        this.set_workspace(workspace, cx);
                        this.active_sidebar = SidebarPanel::Folder;
                        this.status = format!("Cloned {}", this.workspace.display_name());
                    }
                    Err(error) => this.status = format!("Clone failed: {error}"),
                }
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
        self.sync_git(cx);
        cx.notify();
    }

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
            if !self.blocked() {
                self.open(Some(path), window, cx);
            }
        } else if row.expanded {
            self.workspace.collapse(&path);
            self.tree_rows = self.workspace.rows();
            cx.notify();
        } else {
            self.load_directory(path, cx);
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        match self.project_directory() {
            Some(base) if path.is_relative() => base.join(path),
            _ => path.to_path_buf(),
        }
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let buffer = self.new_buffer(Document::default(), window, cx);
        self.active += 1;
        self.buffers.insert(self.active, buffer);
        self.activate(self.active, window, cx);
        self.status = "New file".into();
    }

    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        if !self.blocked() {
            self.open(None, window, cx);
        }
    }

    fn save_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        self.save(SaveTarget::Current, None, window, cx);
    }

    fn save_file_as(&mut self, _: &SaveFileAs, window: &mut Window, cx: &mut Context<Self>) {
        self.save(SaveTarget::Ask, None, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_diff {
            if !self.blocked() {
                self.close_diff(window, cx);
            }
        } else {
            self.request(PendingAction::CloseBuffer(self.buffer().id), window, cx);
        }
    }

    fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let files = self.buffers.len();
        let has_diff = self.diff_view.read(cx).target().is_some();
        let current = if self.show_diff { files } else { self.active };
        let len = (files + usize::from(has_diff)) as isize;
        let ix = (current as isize + step).rem_euclid(len) as usize;
        if ix == files {
            self.focus_diff(window, cx);
        } else {
            self.activate(ix, window, cx);
        }
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::CloseWindow, window, cx);
    }

    fn toggle_vim_mode(&mut self, _: &ToggleVimMode, _: &mut Window, cx: &mut Context<Self>) {
        self.set_vim_mode(self.vim.is_none(), cx);
    }

    fn toggle_fold(&mut self, _: &ToggleFold, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_fold(folding::toggle, window, cx);
    }

    fn fold_clicked_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor().read(cx);
        if editor.focus_handle(cx).is_focused(window) && editor.cursor_position().character == 0 {
            self.apply_fold(folding::toggle_line, window, cx);
        }
    }

    fn apply_fold(
        &mut self,
        toggle: fn(&str, &mut Vec<Fold>, usize) -> Option<Toggle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || self.show_diff {
            return;
        }
        let editor = self.editor().clone();
        let cursor = editor.read(cx).cursor();
        let buffer = &mut self.buffers[self.active];
        let source = buffer.document.text().to_string();
        let Some(toggle) = toggle(&source, &mut buffer.folds, cursor) else {
            self.status = "No brace block here".into();
            cx.notify();
            return;
        };
        let position = position_at(&folding::projected(&source, &buffer.folds), toggle.cursor);
        editor.update(cx, |editor, cx| {
            let text = editor.text();
            let range =
                text.byte_to_utf16_idx(toggle.edit.start)..text.byte_to_utf16_idx(toggle.edit.end);
            editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
            editor.set_cursor_position(position, window, cx);
        });
        self.status = if toggle.folded {
            "Folded brace block"
        } else {
            "Expanded brace block"
        }
        .into();
        cx.notify();
    }

    fn editor_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left
            || event.modifiers != Modifiers::shift()
            || self.blocked()
        {
            return;
        }
        let editor = self.editor().read(cx);
        if !editor
            .text_geometry()
            .is_some_and(|geometry| geometry.viewport.contains(&event.position))
        {
            return;
        }
        let offset = editor.index_for_mouse_position(event.position);
        if self.go_to_declaration(offset, window, cx) {
            cx.stop_propagation();
        }
    }

    fn go_to_declaration(
        &mut self,
        display_offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let buffer = self.buffer();
        let source = buffer.document.text().to_string();
        let offset = folding::display_to_source(&buffer.folds, display_offset, false);
        let language = syntax::language(buffer.document.path());
        let Some(lookup) = navigation::lookup(language, &source, offset) else {
            return false;
        };
        self.declaration_search = Task::ready(());
        match lookup {
            Lookup::Found(name) => {
                self.reveal_declaration(self.active, name, window, cx);
                true
            }
            Lookup::Search(search) => self.search_declaration(search, language, window, cx),
        }
    }

    fn search_declaration(
        &mut self,
        search: Search,
        language: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(root) = self.project_directory().map(Path::to_path_buf) else {
            let Some(fallback) = search.fallback else {
                return false;
            };
            self.reveal_declaration(self.active, fallback, window, cx);
            return true;
        };
        let origin = self.buffer().id;
        let path = self.document().path().map(Path::to_path_buf);
        let open: HashMap<PathBuf, String> = self
            .buffers
            .iter()
            .filter_map(|buffer| {
                let path = buffer.document.path()?.to_path_buf();
                Some((path, buffer.document.text().to_string()))
            })
            .collect();
        self.status = format!("Looking for the declaration of {}…", search.name);
        cx.notify();
        let executor = cx.background_executor().clone();
        self.declaration_search = cx.spawn_in(window, async move |this, cx| {
            let hit = executor
                .spawn({
                    let search = search.clone();
                    async move {
                        navigation::search_workspace(
                            &root,
                            path.as_deref(),
                            language,
                            &search,
                            &open,
                        )
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_declaration(origin, search, hit, window, cx)
            });
        });
        true
    }

    fn open_declaration(
        &mut self,
        origin: BufferId,
        search: Search,
        hit: Option<Hit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Search { name, fallback, .. } = search;
        let target = hit
            .and_then(|hit| {
                match self.buffer_for_path(&hit.path) {
                    Some(ix) => self.activate(ix, window, cx),
                    None => self.open_document(hit.document?, window, cx),
                }
                Some((self.active, hit.name))
            })
            .or_else(|| Some((self.buffer_index(origin)?, fallback?)));
        match target {
            Some((ix, declaration)) => {
                if ix != self.active {
                    self.activate(ix, window, cx);
                }
                self.reveal_declaration(ix, declaration, window, cx);
            }
            None => {
                self.status = format!("No declaration found for {name}");
                cx.notify();
            }
        }
    }

    fn reveal_declaration(
        &mut self,
        ix: usize,
        declaration: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = &mut self.buffers[ix];
        let source = buffer.document.text().to_string();
        let offset = declaration.start.min(source.len());
        let editor = buffer.editor.clone();
        let display = match folding::reveal(&source, &mut buffer.folds, offset) {
            Some(toggle) => {
                editor.update(cx, |editor, cx| {
                    let text = editor.text();
                    let range = text.byte_to_utf16_idx(toggle.edit.start)
                        ..text.byte_to_utf16_idx(toggle.edit.end);
                    editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
                });
                toggle.cursor
            }
            None => folding::source_to_display(&buffer.folds, offset),
        };
        let name = source.get(declaration).unwrap_or_default();
        let line = position_at(&source, offset).line + 1;
        self.status = format!("Declaration of {name} — {}:{line}", buffer.document.name());
        editor.update(cx, |editor, cx| editor.reveal(display, window, cx));
        cx.notify();
    }

    fn set_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.vim.is_some() {
            return;
        }
        self.vim = enabled.then(|| {
            let mut vim = VimInput::new(INDENT_WIDTH);
            vim.reset(self.editor(), cx);
            vim
        });
        self.status = if enabled {
            "Vim keys on"
        } else {
            "Vim keys off"
        }
        .into();
        cx.notify();
    }

    fn editor_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor().clone();
        if self.blocked() || !editor.focus_handle(cx).is_focused(window) {
            return;
        }
        let Some(vim) = &mut self.vim else {
            return;
        };
        let Some(response) = vim.key_down(&event.keystroke, &editor, window, cx) else {
            return;
        };
        cx.stop_propagation();
        if let Some(message) = response.message {
            self.status = message;
        }
        self.run_vim_commands(response.commands, window, cx);
        cx.notify();
    }

    fn run_vim_commands(
        &mut self,
        commands: Vec<VimCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut commands = commands.into_iter().peekable();
        while let Some(command) = commands.next() {
            match command {
                VimCommand::Write | VimCommand::WriteAs(_) => {
                    let then = commands
                        .next_if(|next| matches!(next, VimCommand::Close { .. }))
                        .map(|_| PendingAction::CloseBuffer(self.buffer().id));
                    let target = match command {
                        VimCommand::WriteAs(path) => SaveTarget::Path(self.resolve(&path)),
                        _ => SaveTarget::Current,
                    };
                    self.save(target, then, window, cx);
                }
                VimCommand::WriteAll => {
                    let then = commands
                        .next_if(|next| matches!(next, VimCommand::CloseAll { .. }))
                        .map(|_| PendingAction::CloseWindow);
                    self.save_all(then, window, cx);
                }
                VimCommand::Close { force: true } => self.remove_buffer(self.active, window, cx),
                VimCommand::Close { force: false } => {
                    self.request(PendingAction::CloseBuffer(self.buffer().id), window, cx)
                }
                VimCommand::CloseAll { force: true } => window.remove_window(),
                VimCommand::CloseAll { force: false } => {
                    self.request(PendingAction::CloseWindow, window, cx)
                }
                VimCommand::NextBuffer => self.cycle_tab(1, window, cx),
                VimCommand::PreviousBuffer => self.cycle_tab(-1, window, cx),
                VimCommand::NewBuffer => self.new_file(&NewFile, window, cx),
                VimCommand::Open(path) => {
                    let path = self.resolve(&path);
                    self.open(Some(path), window, cx);
                }
                VimCommand::Undo | VimCommand::Redo => {}
            }
        }
    }

    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> String {
        let Some(path) = settings::path(cx) else {
            return "No settings file is available".into();
        };
        let problems = match Settings::load(&path) {
            Ok(settings) => settings.apply(Some(window), cx),
            Err(error) => vec![error],
        };
        cx.notify();
        settings::summary(&problems).unwrap_or_else(|| "Settings applied".into())
    }

    fn reload_settings(&mut self, _: &ReloadSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.status = self.apply_settings(window, cx);
    }

    fn edit_settings(&mut self, _: &EditSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let Some(path) = settings::path(cx) else {
            self.status = "No settings folder is available on this system".into();
            cx.notify();
            return;
        };
        if let Err(error) = settings::create_if_missing(&path) {
            self.status = format!("Could not create {}: {error}", path.display());
            cx.notify();
            return;
        }
        self.open(Some(path), window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        let vim_enabled = Rc::new(Cell::new(self.vim.is_some()));
        let settings_path = settings::path(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, vim_enabled) = (shell.clone(), vim_enabled.clone());
            let settings_file = {
                let (edit_shell, reload_shell) = (shell.clone(), shell.clone());
                let location = settings_path
                    .as_ref()
                    .map_or("No settings folder is available".into(), |path| {
                        path.display().to_string()
                    });
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(settings::FILE_NAME)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child("Hotkeys, panel colors and syntax colors"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::subtle())
                                    .truncate()
                                    .child(location),
                            ),
                    )
                    .child(
                        Button::new("reload-settings")
                            .ghost()
                            .small()
                            .label("Reload")
                            .disabled(settings_path.is_none())
                            .on_click(move |_, window, cx| {
                                let _ = reload_shell.update(cx, |shell, cx| {
                                    shell.reload_settings(&ReloadSettings, window, cx)
                                });
                            }),
                    )
                    .child(
                        div().debug_selector(|| "edit-settings".into()).child(
                            Button::new("edit-settings")
                                .small()
                                .label("Edit")
                                .disabled(settings_path.is_none())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = edit_shell.update(cx, |shell, cx| {
                                        shell.edit_settings(&EditSettings, window, cx)
                                    });
                                }),
                        ),
                    )
            };
            let section = |title: &'static str, setting: Div| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(ui::caption(title))
                    .child(
                        div()
                            .px_3()
                            .py_2p5()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .child(setting),
                    )
            };
            dialog.title("Settings").w(px(440.)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .pb_1()
                    .child(section(
                        "APPEARANCE",
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
                    ))
                    .child(section(
                        "EDITOR",
                        div().debug_selector(|| "vim-mode".into()).child(
                            Switch::new("vim-mode")
                                .label("Vim keys in the editor")
                                .checked(vim_enabled.get())
                                .on_click(move |enabled, _, cx| {
                                    vim_enabled.set(*enabled);
                                    let _ = shell
                                        .update(cx, |shell, cx| shell.set_vim_mode(*enabled, cx));
                                }),
                        ),
                    ))
                    .child(section("CUSTOMIZATION", settings_file)),
            )
        });
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "sidebar".into())
            .size_full()
            .overflow_hidden()
            .bg(theme::panel())
            .child(match self.active_sidebar {
                SidebarPanel::Folder => self.render_project_sidebar(cx).into_any_element(),
                SidebarPanel::Git => self.git.clone().into_any_element(),
            })
    }

    fn render_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(ACTIVITY_BAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .py_1p5()
            .bg(theme::chrome())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(activity_item(
                        "quick-folder",
                        Icon::new(AppIcon::Files),
                        self.active_sidebar == SidebarPanel::Folder,
                        "Explorer",
                        Box::new(ShowFolderPanel),
                        cx.listener(|this, _, window, cx| {
                            this.show_folder_panel(&ShowFolderPanel, window, cx)
                        }),
                    ))
                    .child(activity_item(
                        "quick-git",
                        Icon::new(AppIcon::GitBranch),
                        self.active_sidebar == SidebarPanel::Git,
                        "Source Control",
                        Box::new(ShowGitPanel),
                        cx.listener(|this, _, window, cx| {
                            this.show_git_panel(&ShowGitPanel, window, cx)
                        }),
                    )),
            )
            .child(activity_item(
                "open-settings",
                Icon::new(IconName::Settings),
                false,
                "Settings",
                Box::new(OpenSettings),
                cx.listener(|this, _, window, cx| this.open_settings(&OpenSettings, window, cx)),
            ))
    }

    fn render_project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_root = self.workspace.root().is_some();
        let caption = if has_root {
            self.workspace.display_name().to_uppercase()
        } else {
            "EXPLORER".into()
        };
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                ui::panel_header(caption)
                    .child(
                        Button::new("sidebar-new-file")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New File")
                            .disabled(self.blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_file(&NewFile, window, cx)
                            })),
                    )
                    .child(
                        Button::new("sidebar-open-folder")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderOpen)
                            .tooltip("Open Folder…")
                            .disabled(self.blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_folder(&OpenFolder, window, cx)
                            })),
                    ),
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
                            .px_1p5()
                            .track_scroll(self.tree_scroll.clone()),
                        ),
                    )
                } else {
                    sidebar.child(
                        ui::empty_state(
                            Icon::new(IconName::FolderOpen).size(px(28.)),
                            "No folder open",
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::subtle())
                                .child("Open a folder to browse and edit its files."),
                        )
                        .child(
                            div()
                                .pt_2()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    Button::new("open-folder")
                                        .primary()
                                        .small()
                                        .label("Open Folder…")
                                        .disabled(self.blocked())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_folder(&OpenFolder, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("clone-repository")
                                        .small()
                                        .label(if self.cloning {
                                            "Cloning…"
                                        } else {
                                            "Clone Repository…"
                                        })
                                        .loading(self.cloning)
                                        .disabled(self.blocked() || self.cloning)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clone_repository(&CloneRepository, window, cx)
                                        })),
                                ),
                        ),
                    )
                }
            })
    }

    fn render_tree_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = &self.tree_rows[ix];
        let is_open = !row.entry.is_dir && self.document().path() == Some(row.entry.path.as_path());
        let change = self.git.read(cx).tree_change(&row.entry.path);
        let letter = change.filter(|_| !row.entry.is_dir).map(Change::letter);
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
            .gap_1p5()
            .pl(px(4. + 14. * row.depth as f32))
            .pr_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()))
            .when(is_open, |row| row.bg(theme::active_row()))
            .when_some(change, |row, change| {
                row.text_color(theme::git_change(change))
            })
            .child(
                div()
                    .size(px(14.))
                    .flex_shrink_0()
                    .when_some(chevron, |slot, chevron| {
                        slot.child(Icon::new(chevron).size_full().text_color(theme::subtle()))
                    }),
            )
            .child(
                Icon::new(icon)
                    .size(px(15.))
                    .flex_shrink_0()
                    .text_color(if row.entry.is_dir {
                        theme::accent()
                    } else {
                        theme::muted()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.entry.name.clone()),
            )
            .when_some(letter, |row, letter| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(letter),
                )
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate_tree_row(ix, window, cx)),
            )
    }

    fn tab_title(&self, buffer: &Buffer) -> String {
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

    fn render_breadcrumbs(&self) -> impl IntoElement {
        let row = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap();
        let Some(path) = self.document().path() else {
            return row
                .child(
                    div()
                        .text_color(theme::text())
                        .child(self.document().name()),
                )
                .child(
                    div()
                        .text_color(theme::subtle())
                        .child("· not saved to disk"),
                );
        };
        let relative = self
            .workspace
            .root()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        let segments: Vec<String> = relative
            .iter()
            .map(|segment| segment.to_string_lossy().into_owned())
            .filter(|segment| !segment.is_empty() && segment != "\\" && segment != "/")
            .collect();
        let last = segments.len().saturating_sub(1);
        row.children(segments.into_iter().enumerate().flat_map(|(ix, segment)| {
            let separator = (ix > 0).then(|| {
                Icon::new(IconName::ChevronRight)
                    .size(px(12.))
                    .flex_shrink_0()
                    .text_color(theme::subtle())
                    .into_any_element()
            });
            let label = div()
                .flex_shrink_0()
                .when(ix == last, |label| label.text_color(theme::text()))
                .child(segment)
                .into_any_element();
            separator.into_iter().chain([label])
        }))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tabs")
            .debug_selector(|| "tabs".into())
            .h(px(HEADER_HEIGHT + 2.))
            .flex_shrink_0()
            .flex()
            .overflow_x_scroll()
            .bg(theme::panel())
            .children(
                self.buffers
                    .iter()
                    .enumerate()
                    .map(|(ix, buffer)| self.render_tab(ix, buffer, cx)),
            )
            .when_some(self.diff_view.read(cx).title(), |tabs, title| {
                tabs.child(self.render_diff_tab(title, cx))
            })
    }

    fn render_tab(&self, ix: usize, buffer: &Buffer, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = buffer.id;
        let active = ix == self.active && !self.show_diff;
        let dirty = buffer.dirty;
        let close_hidden = dirty || !active;
        editor_tab(
            ("tab", id),
            active,
            Icon::new(IconName::File),
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
                    tab_close_button(("close-tab", id), close_hidden).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.request(PendingAction::CloseBuffer(id), window, cx);
                        },
                    )),
                ),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !this.blocked()
                && let Some(ix) = this.buffer_index(id)
            {
                this.activate(ix, window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, window, cx| {
                this.request(PendingAction::CloseBuffer(id), window, cx)
            }),
        )
    }

    fn render_diff_tab(&self, title: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.show_diff;
        editor_tab("diff-tab", active, Icon::new(AppIcon::GitBranch), title)
            .debug_selector(|| "diff-tab".into())
            .child(div().size(px(18.)).flex_shrink_0().child(
                tab_close_button("close-diff-tab", !active).on_click(cx.listener(
                    |this, _, window, cx| {
                        cx.stop_propagation();
                        if !this.blocked() {
                            this.close_diff(window, cx);
                        }
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
                    if !this.blocked() {
                        this.close_diff(window, cx);
                    }
                }),
            )
    }

    fn line_number_width(&self, window: &Window, cx: &App) -> Pixels {
        let digits = match self.editor().read(cx).text().lines_len() {
            0..=9999 => 5,
            10000..=99999 => 6,
            100000..=999999 => 7,
            _ => 8,
        };
        let sample = "+".repeat(digits);
        let run = TextRun {
            len: sample.len(),
            font: theme::monospace_font(),
            color: theme::text().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(sample.into(), EDITOR_TEXT_SIZE, &[run], None);
        line.width + LINE_NUMBER_PADDING
    }

    fn render_brace_guide(&self, cx: &App) -> impl IntoElement {
        let editor = self.editor().clone();
        let state = editor.read(cx);
        let guide = BraceGuide::find(&state.text().to_string(), state.cursor());
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if let Some(guide) = guide
                    && let Some(geometry) = editor.read(cx).text_geometry()
                {
                    guide.paint(geometry, bounds, EDITOR_TEXT_SIZE, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    fn render_editor(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vim_context = self.vim.as_ref().map(VimInput::key_context);
        let area = div()
            .key_context(EDITOR_CONTEXT)
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx));
        if self.show_diff {
            return area.child(div().flex_1().min_h_0().child(self.diff_view.clone()));
        }
        area.child(
            div()
                .h(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .pl_4()
                .pr_2()
                .bg(theme::background())
                .text_xs()
                .text_color(theme::muted())
                .child(self.render_breadcrumbs()),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .capture_any_mouse_down(cx.listener(Self::editor_mouse_down))
                .when_some(vim_context, |editor, context| {
                    editor
                        .key_context(context)
                        .capture_key_down(cx.listener(Self::editor_key_down))
                })
                .child(
                    Input::new(self.editor())
                        .h_full()
                        .w_full()
                        .pl(EDITOR_PADDING_LEFT)
                        .bordered(false)
                        .focus_bordered(false)
                        .appearance(false)
                        .disabled(self.blocked())
                        .font_family(theme::mono_family())
                        .text_size(EDITOR_TEXT_SIZE)
                        .bg(theme::background())
                        .text_color(theme::text()),
                )
                .child(self.render_brace_guide(cx))
                .child(
                    div()
                        .debug_selector(|| "fold-gutter".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(EDITOR_PADDING_LEFT)
                        .w(self.line_number_width(window, cx))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, window, cx| {
                                cx.defer_in(window, Self::fold_clicked_line);
                            }),
                        ),
                ),
        )
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.focus_handle(cx).is_focused(window) {
            self.focus_editor(window, cx);
        } else {
            self.focus_terminal(window, cx);
        }
    }

    fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Terminal;
        let directory = self.project_directory().map(Path::to_path_buf);
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
                                ToolPanel::Debug => {
                                    this.show_debug_panel(&ShowDebugPanel, window, cx)
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
                    .bg(theme::background())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus_terminal(window, cx)),
                    )
                    .child(self.terminal.clone())
                    .into_any_element(),
                ToolPanel::Debug => div()
                    .flex_1()
                    .min_h_0()
                    .bg(theme::background())
                    .child(ui::empty_state(
                        Icon::new(AppIcon::Bug).size(px(24.)),
                        "Debug adapter integration is not implemented yet.",
                    ))
                    .into_any_element(),
            })
    }
}

impl IdeShell {
    fn active_tab(&self, cx: &App) -> (String, bool) {
        match self.diff_view.read(cx).title() {
            Some(title) if self.show_diff => (title, false),
            _ => (self.document().name(), self.buffer().dirty),
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (name, dirty) = self.active_tab(cx);
        TitleBar::new()
            .text_color(theme::text())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.menu_bar.clone(), |row, menu_bar| {
                        row.child(div().flex_shrink_0().child(menu_bar))
                    }),
            )
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

    fn render_save_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "Save changes to {} before closing?",
                self.document().name()
            )))
            .child(
                Button::new("confirm-save")
                    .primary()
                    .small()
                    .label("Save")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save(SaveTarget::Current, None, window, cx)
                    })),
            )
            .child(
                Button::new("discard")
                    .small()
                    .label("Discard")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
            )
            .child(
                Button::new("cancel")
                    .ghost()
                    .small()
                    .label("Cancel")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pending = None;
                        this.focus_editor(window, cx);
                        cx.notify();
                    })),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .when_some(self.git.read(cx).branch(), |bar, branch| {
                bar.child(
                    status_item("status-branch", false)
                        .child(Icon::new(AppIcon::GitBranch).size(px(13.)))
                        .child(branch)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_git_panel(&ShowGitPanel, window, cx)
                        })),
                )
                .child(divider())
            })
            .child(div().flex_1().min_w_0().px_1p5().truncate().child(status))
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
                    .child(ToolPanel::Debug.label())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_debug_panel(&ShowDebugPanel, window, cx)
                    })),
            )
            .when(!self.show_diff, |bar| {
                bar.child(divider())
                    .child(div().flex_shrink_0().px_1p5().child("UTF-8"))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1p5()
                            .child(self.document().line_ending().label()),
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

impl Render for IdeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (name, dirty) = self.active_tab(cx);
        let name = format!("{name}{}", if dirty { " •" } else { "" });
        window.set_window_title(&match self.workspace.root() {
            Some(_) => format!("{name} — {} — Hephaestus", self.workspace.display_name()),
            None => format!("{name} — Hephaestus"),
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(13.))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::clone_repository))
            .on_action(cx.listener(Self::save_file))
            .on_action(cx.listener(Self::save_file_as))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::edit_settings))
            .on_action(cx.listener(Self::reload_settings))
            .on_action(cx.listener(Self::show_folder_panel))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::show_git_panel))
            .on_action(cx.listener(Self::show_debug_panel))
            .on_action(cx.listener(Self::toggle_vim_mode))
            .on_action(cx.listener(Self::toggle_fold))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::stage_all)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::pull)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::push)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::fetch)))
            .child(self.render_title_bar(cx))
            .when(self.pending.is_some(), |view| {
                view.child(self.render_save_prompt(cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_activity_bar(cx))
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("sidebar-split")
                                .with_state(&self.sidebar_split)
                                .child(
                                    resizable_panel()
                                        .size(px(240.))
                                        .size_range(px(160.)..px(480.))
                                        .child(self.render_sidebar(cx)),
                                )
                                .child(
                                    v_resizable("tool-panel-split")
                                        .with_state(&self.tool_panel_split)
                                        .child(
                                            resizable_panel().child(self.render_editor(window, cx)),
                                        )
                                        .child(
                                            resizable_panel()
                                                .size(px(200.))
                                                .size_range(px(72.)..Pixels::MAX)
                                                .child(self.render_tool_panel(cx)),
                                        ),
                                ),
                        ),
                    ),
            )
            .child(self.render_status_bar(cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}

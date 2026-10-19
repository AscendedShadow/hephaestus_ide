use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    rc::Rc,
};

use gpui::{
    Action, App, BorderStyle, Bounds, ContentMask, Context, Corner, Div, ElementId, Entity,
    EntityInputHandler as _, Focusable, FontWeight, Hsla, IntoElement, KeyDownEvent, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, OwnedMenuItem, PathBuilder, PathPromptOptions,
    Pixels, Render, Stateful, Subscription, Task, TextRun, UniformListScrollHandle, Window, canvas,
    div, fill, point, prelude::*, px, quad, size, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Root, RopeExt as _, Sizable as _, ThemeMode, TitleBar,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, Position, TabSize},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    scroll::ScrollableElement as _,
    switch::Switch,
};
use ide_core::{
    document::Document,
    git::{self, Change, Repository},
    search::{self, Match as SearchMatch},
    vim::Command as VimCommand,
    workspace::{self, TreeRow, Workspace},
};

use crate::{
    assets::AppIcon,
    brace_guide::BraceGuide,
    commands::*,
    debugger::{Debugger, DebuggerEvent, State as DebugState, Step},
    diff_view::DiffView,
    file_icons::FileKind,
    folding::{self, Fold, Toggle},
    git_panel::{GitPanel, GitPanelEvent},
    language_server,
    navigation::{self, Hit, Lookup, Search},
    session::{self, Session},
    settings::{self, Settings},
    syntax,
    terminal_view::{TerminalEvent, TerminalView},
    theme,
    ui::{self, HEADER_HEIGHT},
    vim::VimInput,
};

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shell_feature_tests.rs"]
mod feature_tests;

pub const EDITOR_CONTEXT: &str = "Editor";
const INDENT_WIDTH: usize = 2;
const EDITOR_TEXT_SIZE: Pixels = px(14.);
const EDITOR_PADDING_LEFT: Pixels = px(20.);
const BREAKPOINT_RADIUS: f32 = 4.5;
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
    Run,
    Debug,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    Files,
    Commands,
    Project,
    File,
    Tasks,
    Diagnostics,
    Completion,
    Line,
}

impl ToolPanel {
    const ALL: [Self; 3] = [Self::Terminal, Self::Run, Self::Debug];

    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Run => "Run",
            Self::Debug => "Debug",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Terminal => Icon::new(IconName::SquareTerminal),
            Self::Run => Icon::new(AppIcon::Play),
            Self::Debug => Icon::new(AppIcon::Bug),
        }
    }
}

const TAB_GROUP: &str = "tab";

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
    external_change: bool,
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
    sidebar_split: Entity<ResizableState>,
    tool_panel_split: Entity<ResizableState>,
    active_sidebar: SidebarPanel,
    show_sidebar: bool,
    active_panel: ToolPanel,
    terminal: Entity<TerminalView>,
    task_terminal: Entity<TerminalView>,
    debugger: Entity<Debugger>,
    breakpoint_hover: Option<usize>,
    git: Entity<GitPanel>,
    diff_view: Entity<DiffView>,
    show_diff: bool,
    pending: Option<PendingAction>,
    busy: bool,
    cloning: bool,
    vim: Option<VimInput>,
    status: String,
    declaration_search: Task<()>,
    search_mode: Option<SearchMode>,
    search_input: Entity<InputState>,
    search_paths: Vec<PathBuf>,
    recent_paths: Vec<PathBuf>,
    locations: Vec<SearchMatch>,
    location_index: usize,
    search_hits: Vec<SearchMatch>,
    search_task: Task<()>,
    search_selected: usize,
    task_commands: Vec<settings::RunConfig>,
    servers: Entity<language_server::Servers>,
    diagnostics: HashMap<PathBuf, Vec<SearchMatch>>,
    run_hits: Vec<SearchMatch>,
    command_directory: Option<PathBuf>,
    lsp_task: Task<()>,
    completions: Vec<(String, String)>,
    pending_hit: Option<SearchMatch>,
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
            sidebar_split: cx.new(|_| ResizableState::default()),
            tool_panel_split: cx.new(|_| ResizableState::default()),
            active_sidebar: SidebarPanel::default(),
            show_sidebar: settings::path(cx)
                .and_then(|path| Settings::load(&path).ok())
                .is_some_and(|settings| settings.show_sidebar),
            active_panel: ToolPanel::default(),
            terminal: cx.new(TerminalView::new),
            task_terminal: cx.new(TerminalView::new),
            debugger: cx.new(|cx| Debugger::new(window, cx)),
            breakpoint_hover: None,
            git,
            diff_view,
            show_diff: false,
            pending: None,
            busy: false,
            cloning: false,
            vim: None,
            status: "Ready — open a file or start typing".into(),
            declaration_search: Task::ready(()),
            search_mode: None,
            search_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Search…")
                    .multi_line(false)
            }),
            search_paths: Vec::new(),
            recent_paths: Vec::new(),
            locations: Vec::new(),
            location_index: 0,
            search_hits: Vec::new(),
            search_task: Task::ready(()),
            search_selected: 0,
            task_commands: Vec::new(),
            servers: cx.new(|_| language_server::Servers::default()),
            diagnostics: HashMap::new(),
            run_hits: Vec::new(),
            command_directory: None,
            lsp_task: Task::ready(()),
            completions: Vec::new(),
            pending_hit: None,
            _subscriptions: Vec::new(),
        };
        shell._subscriptions = vec![
            cx.subscribe_in(&shell.servers.clone(), window, |this, _, event, _, cx| {
                this.handle_server_event(event, cx)
            }),
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
            cx.observe(&shell.debugger, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &shell.debugger,
                window,
                |this, _, event, window, cx| match event {
                    DebuggerEvent::Reveal { path, line } => {
                        if !this.blocked() {
                            let hit = SearchMatch {
                                path: path.clone(),
                                line: *line,
                                column: 1,
                                preview: String::new(),
                            };
                            this.open_at(hit, window, cx);
                        }
                    }
                    DebuggerEvent::Status(status) => {
                        this.status = status.clone();
                        cx.notify();
                    }
                },
            ),
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
                    this.check_disk_changes(window, cx);
                }
            }),
            cx.subscribe_in(
                &shell.search_input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).text().to_string();
                        if !text.ends_with('\n') {
                            this.update_search(text, cx);
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        this.choose_search_result(this.search_selected, window, cx)
                    }
                    _ => {}
                },
            ),
            cx.subscribe(&shell.task_terminal, |this, _, event, cx| {
                let TerminalEvent::Exited(lines) = event;
                let directory = this.command_directory.as_deref().unwrap_or(Path::new("."));
                this.run_hits = ide_core::output::locations(lines, directory);
                if !this.run_hits.is_empty() {
                    this.status = format!(
                        "{} build location(s) — open Diagnostics to navigate",
                        this.run_hits.len()
                    );
                }
                cx.notify();
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

    pub fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = settings::path(cx).map(|path| session::path(&path)) else {
            return;
        };
        let snapshot = session::load(&path);
        if let Some(folder) = snapshot.folder
            && let Ok(workspace) = Workspace::open(&folder)
        {
            self.set_workspace(workspace, cx);
        }
        for (ix, file) in snapshot.files.into_iter().take(12).enumerate() {
            if let Ok(document) = Document::open(&file) {
                self.open_document(document, window, cx);
                if let Some((line, column)) = snapshot.cursors.get(ix).copied() {
                    self.editor().update(cx, |editor, cx| {
                        editor.set_cursor_position(Position::new(line, column), window, cx)
                    });
                }
            }
        }
        if snapshot.active < self.buffers.len() {
            self.activate(snapshot.active, window, cx);
        }
        self.recent_paths = snapshot
            .recent
            .into_iter()
            .filter(|path| path.is_file())
            .take(100)
            .collect();
    }

    fn save_session(&self, cx: &App) {
        let Some(path) = settings::path(cx).map(|path| session::path(&path)) else {
            return;
        };
        let snapshot = Session {
            folder: self.workspace.root().map(Path::to_path_buf),
            files: self
                .buffers
                .iter()
                .filter_map(|buffer| buffer.document.path().map(Path::to_path_buf))
                .collect(),
            active: self.buffers[..self.active]
                .iter()
                .filter(|buffer| buffer.document.path().is_some())
                .count(),
            cursors: self
                .buffers
                .iter()
                .filter(|buffer| buffer.document.path().is_some())
                .map(|buffer| {
                    let cursor = buffer.editor.read(cx).cursor_position();
                    (cursor.line, cursor.character)
                })
                .collect(),
            recent: self.recent_paths.clone(),
        };
        let _ = session::save(&path, &snapshot);
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
        language_server::attach(
            &editor,
            &self.servers,
            document.path().map(Path::to_path_buf),
            syntax::language(document.path()),
            cx,
        );
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
                this.sync_lsp_buffer(ix, cx);
                cx.notify();
            }
        });
        Buffer {
            id,
            document,
            editor,
            dirty: false,
            folds: Vec::new(),
            external_change: false,
            _subscription: subscription,
        }
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        if let Some(path) = self.document().path().map(Path::to_path_buf) {
            self.recent_paths.retain(|recent| recent != &path);
            self.recent_paths.insert(0, path);
            self.recent_paths.truncate(100);
        }
        self.show_diff = false;
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        if let Some(vim) = &mut self.vim {
            vim.reset(&editor, cx);
        }
        self.sync_git(cx);
        self.sync_servers(cx);
        self.sync_lsp_buffer(ix, cx);
        cx.notify();
    }

    fn restart_servers(&mut self, cx: &mut Context<Self>) {
        self.servers.update(cx, |servers, cx| servers.stop(cx));
        self.diagnostics.clear();
        self.sync_servers(cx);
        for ix in 0..self.buffers.len() {
            self.sync_lsp_buffer(ix, cx);
        }
        self.status = "Restarting the language server…".into();
        cx.notify();
    }

    fn sync_servers(&mut self, cx: &mut Context<Self>) {
        let root = self.project_directory().map(Path::to_path_buf);
        let configs = settings::language_servers(cx);
        self.servers.update(cx, |servers, cx| {
            servers.set_configs(configs, cx);
            servers.set_root(root, cx);
        });
    }

    fn sync_lsp_buffer(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(buffer) = self.buffers.get(ix) else {
            return;
        };
        let language = syntax::language(buffer.document.path());
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            return;
        };
        let text = buffer.document.text().to_string();
        self.servers
            .update(cx, |servers, cx| servers.sync(&path, language, &text, cx));
    }

    fn handle_server_event(
        &mut self,
        event: &language_server::ServerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            language_server::ServerEvent::Ready => {
                for ix in 0..self.buffers.len() {
                    self.sync_lsp_buffer(ix, cx);
                }
            }
            language_server::ServerEvent::Status(status) => self.status = status.clone(),
            language_server::ServerEvent::Diagnostics { path, diagnostics } => {
                self.apply_diagnostics(path, diagnostics, cx)
            }
        }
        cx.notify();
    }

    fn apply_diagnostics(
        &mut self,
        path: &Path,
        diagnostics: &[lsp_types::Diagnostic],
        cx: &mut Context<Self>,
    ) {
        let hits = diagnostics
            .iter()
            .map(|diagnostic| SearchMatch {
                path: path.to_path_buf(),
                line: diagnostic.range.start.line as usize + 1,
                column: diagnostic.range.start.character as usize + 1,
                preview: diagnostic
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(160)
                    .collect(),
            })
            .collect();
        self.diagnostics.insert(path.to_path_buf(), hits);
        let Some(ix) = self.buffer_for_path(path) else {
            return;
        };
        if !self.buffers[ix].folds.is_empty() {
            return;
        }
        let mut sorted = diagnostics.to_vec();
        sorted.sort_by_key(|diagnostic| {
            (
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            )
        });
        self.buffers[ix].editor.update(cx, |editor, cx| {
            let text = editor.text().clone();
            if let Some(set) = editor.diagnostics_mut() {
                set.reset(&text);
                set.extend(sorted);
            }
            cx.notify();
        });
    }

    fn check_disk_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let documents: Vec<_> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.document.path().is_some())
            .map(|buffer| (buffer.id, buffer.document.clone()))
            .collect();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let changed = executor
                .spawn(async move {
                    documents
                        .into_iter()
                        .filter_map(|(id, document)| match document.has_external_changes() {
                            Ok(true) => Some((id, document.path().unwrap().to_path_buf())),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                for (id, path) in changed {
                    let Some(ix) = this.buffer_index(id) else {
                        continue;
                    };
                    if this.buffers[ix].document.path() != Some(path.as_path()) {
                        continue;
                    }
                    if !this.buffers[ix]
                        .document
                        .has_external_changes()
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if this.buffers[ix].dirty {
                        this.buffers[ix].external_change = true;
                        this.status =
                            format!("{} changed on disk — Save As or reload", path.display());
                    } else if let Ok(document) = Document::open(&path) {
                        let text = document.text().to_string();
                        let buffer = &mut this.buffers[ix];
                        buffer.document = document;
                        buffer.folds.clear();
                        buffer
                            .editor
                            .update(cx, |editor, cx| editor.set_value(text, window, cx));
                        buffer.dirty = false;
                        buffer.external_change = false;
                        this.status = format!("Reloaded {}", path.display());
                    } else {
                        this.buffers[ix].external_change = true;
                        this.status = format!(
                            "{} changed or was deleted — Save As to keep this buffer",
                            path.display()
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reload_file(&mut self, _: &ReloadFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            return;
        };
        match Document::open(&path) {
            Ok(document) => {
                let text = document.text().to_string();
                let buffer = &mut self.buffers[self.active];
                buffer.document = document;
                buffer.folds.clear();
                buffer
                    .editor
                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
                buffer.dirty = false;
                buffer.external_change = false;
                self.status = format!("Reloaded {}", path.display());
            }
            Err(error) => self.status = format!("Reload failed: {error}"),
        }
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
        self.show_sidebar = true;
        cx.notify();
    }

    fn show_git_panel(&mut self, _: &ShowGitPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.active_sidebar = SidebarPanel::Git;
        self.show_sidebar = true;
        self.refresh_git(cx);
        cx.notify();
    }

    fn toggle_sidebar(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
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

    fn load_settings(&mut self, cx: &mut Context<Self>) -> Option<Settings> {
        let Some(settings_path) = settings::path(cx) else {
            self.status = "No settings directory is available".into();
            cx.notify();
            return None;
        };
        match Settings::load(&settings_path) {
            Ok(settings) => Some(settings),
            Err(error) => {
                self.status = error;
                cx.notify();
                None
            }
        }
    }

    fn config_directory(
        &mut self,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Option<Option<PathBuf>> {
        let directory = directory
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    self.resolve(&path.to_string_lossy())
                }
            })
            .or_else(|| self.project_directory().map(Path::to_path_buf));
        if directory.as_ref().is_some_and(|path| !path.is_dir()) {
            self.status = "Command working directory does not exist".into();
            cx.notify();
            return None;
        }
        Some(directory)
    }

    fn launch_config(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.load_settings(cx) else {
            return;
        };
        let Some(config) = settings
            .commands
            .into_iter()
            .nth(index)
            .filter(|config| !config.program.trim().is_empty())
        else {
            self.status = "Configure commands in settings.json (program and args)".into();
            cx.notify();
            return;
        };
        let Some(directory) = self.config_directory(config.working_directory.clone(), cx) else {
            return;
        };
        let result = self.task_terminal.update(cx, |terminal, cx| {
            terminal.run(
                terminal::Options {
                    shell: Some((config.program, config.args)),
                    working_directory: directory.clone(),
                    env: config.env,
                    ..Default::default()
                },
                cx,
            )
        });
        if let Err(error) = result {
            self.status = error;
            cx.notify();
            return;
        }
        self.run_hits.clear();
        self.command_directory = directory;
        self.active_panel = ToolPanel::Run;
        window.focus(&self.task_terminal.focus_handle(cx));
        self.status = format!(
            "Started {}",
            if config.name.is_empty() {
                "command"
            } else {
                &config.name
            }
        );
        cx.notify();
    }

    fn run_command(&mut self, _: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.task_commands = settings::path(cx)
            .and_then(|path| Settings::load(&path).ok())
            .map(|settings| settings.commands)
            .unwrap_or_default();
        if self.task_commands.len() > 1 {
            self.show_search(SearchMode::Tasks, window, cx);
        } else {
            self.launch_config(0, window, cx);
        }
    }

    fn start_debugger(&mut self, _: &StartDebugger, _: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Debug;
        if self.debugger.read(cx).is_paused() {
            self.debug_result(|debugger, cx| debugger.resume(cx), cx);
            return;
        }
        if self.debugger.read(cx).is_active() {
            self.status = "The debugger is already running".into();
            cx.notify();
            return;
        }
        let Some(settings) = self.load_settings(cx) else {
            return;
        };
        let Some(config) = settings.debugger else {
            self.status =
                "Configure \"debugger\" in settings.json (adapter and program to debug)".into();
            cx.notify();
            return;
        };
        let Some(directory) = self.config_directory(config.working_directory.clone(), cx) else {
            return;
        };
        let result = self
            .debugger
            .update(cx, |debugger, cx| debugger.start(config, directory, cx));
        self.status = match result {
            Ok(name) => format!("Debugging {name}"),
            Err(error) => error,
        };
        cx.notify();
    }

    fn stop_command(&mut self, _: &StopCommand, _: &mut Window, cx: &mut Context<Self>) {
        self.task_terminal
            .update(cx, |terminal, cx| terminal.terminate(cx));
        self.status = "Stopped process".into();
        cx.notify();
    }

    fn debug_result(
        &mut self,
        command: impl FnOnce(&mut Debugger, &mut Context<Debugger>) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.debugger.update(cx, command) {
            self.status = error;
        }
        cx.notify();
    }

    fn debug_continue(&mut self, _: &DebugContinue, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_result(|debugger, cx| debugger.resume(cx), cx);
    }

    fn debug_step_over(&mut self, _: &DebugStepOver, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_result(|debugger, cx| debugger.step(Step::Over, cx), cx);
    }

    fn debug_step_into(&mut self, _: &DebugStepInto, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_result(|debugger, cx| debugger.step(Step::Into, cx), cx);
    }

    fn debug_step_out(&mut self, _: &DebugStepOut, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_result(|debugger, cx| debugger.step(Step::Out, cx), cx);
    }

    fn debug_interrupt(&mut self, _: &DebugInterrupt, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_result(|debugger, cx| debugger.pause(cx), cx);
    }

    fn debug_stop(&mut self, _: &DebugStop, _: &mut Window, cx: &mut Context<Self>) {
        self.debugger.update(cx, |debugger, cx| debugger.stop(cx));
        self.status = "Stopped debugging".into();
        cx.notify();
    }

    fn debug_restart(&mut self, _: &DebugRestart, _: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Debug;
        let result = self
            .debugger
            .update(cx, |debugger, cx| debugger.restart(cx));
        self.status = match result {
            Ok(name) => format!("Restarted {name}"),
            Err(error) => error,
        };
        cx.notify();
    }

    fn toggle_breakpoint(&mut self, _: &ToggleBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.editor().read(cx).cursor_position().line as usize;
        self.toggle_breakpoint_at_row(row, cx);
    }

    fn source_line(&self, display_row: usize, cx: &App) -> Option<usize> {
        let buffer = self.buffer();
        let display = buffer.editor.read(cx).text();
        if display_row >= display.lines_len() {
            return None;
        }
        let offset = folding::display_to_source(
            &buffer.folds,
            display.line_start_offset(display_row),
            false,
        );
        Some(buffer.document.text().offset_to_point(offset).row + 1)
    }

    fn display_row(&self, source_line: usize, cx: &App) -> usize {
        let buffer = self.buffer();
        let source = buffer.document.text();
        let offset = source.line_start_offset(source_line.saturating_sub(1));
        let display = folding::source_to_display(&buffer.folds, offset);
        buffer.editor.read(cx).text().offset_to_point(display).row
    }

    fn toggle_breakpoint_at_row(&mut self, display_row: usize, cx: &mut Context<Self>) {
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            self.status = "Save the file before setting a breakpoint".into();
            cx.notify();
            return;
        };
        let Some(line) = self.source_line(display_row, cx) else {
            return;
        };
        let enabled = self.debugger.update(cx, |debugger, cx| {
            debugger.toggle_breakpoint(path.clone(), line, cx)
        });
        self.status = format!(
            "Breakpoint {} at {}:{line}",
            if enabled { "set" } else { "removed" },
            path.display()
        );
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
        if let Some(path) = self.buffers[ix].document.path().map(Path::to_path_buf) {
            self.diagnostics.remove(&path);
            self.servers.update(cx, |servers, _| servers.close(&path));
        }
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
            self.save_session(cx);
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
            PendingAction::CloseWindow => {
                self.save_session(cx);
                window.remove_window();
            }
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
                        let opened_path = document.path().map(Path::to_path_buf);
                        this.status = "Ready".into();
                        this.open_document(document, window, cx);
                        if this
                            .pending_hit
                            .as_ref()
                            .is_some_and(|hit| Some(&hit.path) == opened_path.as_ref())
                        {
                            let hit = this.pending_hit.take().unwrap();
                            this.jump_to_hit(&hit, window, cx);
                        }
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
                            if let Some(old_path) = buffer.document.path().map(Path::to_path_buf)
                                && saved_path.as_ref() != Some(&old_path)
                            {
                                this.servers
                                    .update(cx, |servers, _| servers.close(&old_path));
                            }
                            let language = syntax::language(buffer.document.path());
                            buffer.document.accept_saved(saved);
                            buffer.dirty = buffer.document.is_dirty();
                            buffer.external_change = false;
                            this.status = format!("Saved {}", buffer.document.name());
                            let saved_language = syntax::language(buffer.document.path());
                            if saved_language != language {
                                buffer.editor.update(cx, |editor, cx| {
                                    editor.set_highlighter(saved_language, cx)
                                });
                            }
                            let editor = buffer.editor.clone();
                            language_server::attach(
                                &editor,
                                &this.servers,
                                saved_path.clone(),
                                saved_language,
                                cx,
                            );
                            this.sync_lsp_buffer(ix, cx);
                            if let Some(path) = saved_path.as_deref() {
                                let text = this.buffers[ix].document.text().to_string();
                                this.servers
                                    .update(cx, |servers, _| servers.saved(path, &text));
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
                    Err(error) => {
                        if error.contains("File changed on disk")
                            && let Some(ix) = this.buffer_index(id)
                        {
                            this.buffers[ix].external_change = true;
                        }
                        this.status = format!("Save failed: {error}");
                    }
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
                            let saved_path = saved.path().map(Path::to_path_buf);
                            settings_saved |= saved
                                .path()
                                .is_some_and(|path| settings::is_settings_file(path, cx));
                            if let Some(ix) = this.buffer_index(id) {
                                let buffer = &mut this.buffers[ix];
                                buffer.document.accept_saved(saved);
                                buffer.dirty = buffer.document.is_dirty();
                                buffer.external_change = false;
                                if let Some(path) = saved_path.as_deref() {
                                    let text = this.buffers[ix].document.text().to_string();
                                    this.servers
                                        .update(cx, |servers, _| servers.saved(path, &text));
                                }
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
        if self.workspace.root() != workspace.root() {
            self.task_terminal
                .update(cx, |terminal, cx| terminal.terminate(cx));
            self.debugger.update(cx, |debugger, cx| debugger.stop(cx));
        }
        self.status = format!("Opened folder {}", workspace.display_name());
        self.workspace = workspace;
        self.diagnostics.clear();
        self.sync_servers(cx);
        self.index_workspace(cx);
        self.tree_rows = self.workspace.rows();
        self.tree_scroll = UniformListScrollHandle::new();
        self.sync_git(cx);
        cx.notify();
    }

    fn index_workspace(&mut self, cx: &mut Context<Self>) {
        self.search_paths.clear();
        let Some(root) = self.workspace.root().map(Path::to_path_buf) else {
            return;
        };
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let paths = executor
                .spawn({
                    let root = root.clone();
                    async move { search::files(&root) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace.root() == Some(root.as_path()) {
                    match paths {
                        Ok(paths) => this.search_paths = paths,
                        Err(error) => this.status = format!("Could not index files: {error}"),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn show_search(&mut self, mode: SearchMode, window: &mut Window, cx: &mut Context<Self>) {
        self.search_mode = Some(mode);
        self.search_selected = 0;
        self.search_hits.clear();
        self.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn quick_open(&mut self, _: &QuickOpen, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::Files, window, cx);
    }

    fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::Commands, window, cx);
    }

    fn search_project(&mut self, _: &SearchProject, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::Project, window, cx);
    }

    fn show_diagnostics(
        &mut self,
        _: &ShowDiagnostics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(SearchMode::Diagnostics, window, cx);
    }

    fn find_in_file(&mut self, _: &FindInFile, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::File, window, cx);
    }

    fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::Line, window, cx);
    }

    fn update_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_task = Task::ready(());
        self.search_hits.clear();
        self.search_selected = 0;
        match self.search_mode {
            Some(SearchMode::Project) if !query.is_empty() => {
                let paths = self.search_paths.clone();
                let open: Vec<_> =
                    self.buffers
                        .iter()
                        .filter_map(|buffer| {
                            buffer.document.path().map(|path| {
                                (path.to_path_buf(), buffer.document.text().to_string())
                            })
                        })
                        .collect();
                let executor = cx.background_executor().clone();
                let search_query = query.clone();
                self.search_task = cx.spawn(async move |this, cx| {
                    let hits = executor
                        .spawn({
                            let query = search_query.clone();
                            async move {
                                let mut hits = Vec::new();
                                for (path, source) in &open {
                                    hits.extend(search::search_source(
                                        path,
                                        source,
                                        &query,
                                        200 - hits.len(),
                                    ));
                                    if hits.len() == 200 {
                                        return hits;
                                    }
                                }
                                let closed: Vec<_> = paths
                                    .into_iter()
                                    .filter(|path| !open.iter().any(|(opened, _)| opened == path))
                                    .collect();
                                hits.extend(search::search_text(&closed, &query, 200 - hits.len()));
                                hits
                            }
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.search_mode == Some(SearchMode::Project)
                            && *this.search_input.read(cx).text() == search_query
                        {
                            this.search_hits = hits;
                            cx.notify();
                        }
                    });
                });
            }
            Some(SearchMode::File) if !query.is_empty() => {
                let text = self.document().text().to_string();
                let path = self
                    .document()
                    .path()
                    .unwrap_or(Path::new("Untitled"))
                    .to_path_buf();
                self.search_hits = search::search_source(&path, &text, &query, 200);
            }
            _ => {}
        }
        if self.search_mode == Some(SearchMode::Diagnostics) {
            self.search_hits = self
                .diagnostics
                .values()
                .flatten()
                .chain(self.run_hits.iter())
                .filter(|hit| {
                    query.is_empty() || hit.preview.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect();
            self.search_hits
                .sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
        }
        cx.notify();
    }

    fn choose_search_result(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self.search_mode else { return };
        let query = self.search_input.read(cx).text().to_string();
        let query = query.trim_end_matches(['\r', '\n']);
        let file = (mode == SearchMode::Files)
            .then(|| self.ranked_paths(query).get(ix).cloned())
            .flatten();
        let command = (mode == SearchMode::Commands)
            .then(|| self.ranked_commands(query).get(ix).copied())
            .flatten();
        let task = (mode == SearchMode::Tasks)
            .then(|| self.ranked_tasks(query).get(ix).copied())
            .flatten();
        let completion = (mode == SearchMode::Completion)
            .then(|| self.ranked_completions(query).get(ix).copied())
            .flatten();
        let hit = self.search_hits.get(ix).cloned();
        self.search_mode = None;
        match mode {
            SearchMode::Files => {
                if let Some(path) = file {
                    self.open(Some(path), window, cx);
                }
            }
            SearchMode::Commands => {
                if let Some(command) = command {
                    window.dispatch_action(command.action(), cx);
                }
            }
            SearchMode::Tasks => {
                if let Some(index) = task {
                    self.launch_config(index, window, cx);
                }
            }
            SearchMode::Completion => {
                if let Some(index) = completion {
                    self.insert_completion(index, window, cx);
                }
            }
            SearchMode::Line => {
                let parts: Vec<_> = query.split(':').collect();
                if let Some(line) = parts
                    .first()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|line| *line > 0)
                {
                    let column = parts
                        .get(1)
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1);
                    let path = self
                        .document()
                        .path()
                        .unwrap_or(Path::new("Untitled"))
                        .to_path_buf();
                    let hit = SearchMatch {
                        path,
                        line,
                        column,
                        preview: String::new(),
                    };
                    self.record_location(&hit, cx);
                    self.jump_to_hit(&hit, window, cx);
                }
            }
            SearchMode::File => {
                if let Some(hit) = hit {
                    self.record_location(&hit, cx);
                    self.jump_to_hit(&hit, window, cx);
                }
            }
            SearchMode::Project | SearchMode::Diagnostics => {
                if let Some(hit) = hit {
                    self.record_location(&hit, cx);
                    if self.document().path() == Some(hit.path.as_path()) {
                        self.jump_to_hit(&hit, window, cx);
                    } else {
                        self.open_at(hit, window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn ranked_paths(&self, query: &str) -> Vec<PathBuf> {
        let root = self.workspace.root();
        let mut files: Vec<_> = self
            .search_paths
            .iter()
            .filter_map(|path| {
                search::rank(
                    path.strip_prefix(root.unwrap_or(Path::new("")))
                        .unwrap_or(path),
                    query,
                )
                .map(|score| {
                    (
                        score,
                        self.recent_paths
                            .iter()
                            .position(|recent| recent == path)
                            .unwrap_or(usize::MAX),
                        path.clone(),
                    )
                })
            })
            .collect();
        files.sort_by(|a, b| {
            if query.is_empty() {
                a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2))
            } else {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
            }
        });
        files
            .into_iter()
            .take(30)
            .map(|(_, _, path)| path)
            .collect()
    }

    fn ranked_commands(&self, query: &str) -> Vec<Command> {
        Command::ALL
            .into_iter()
            .filter(|command| search::rank(Path::new(&command.name()), query).is_some())
            .collect()
    }

    fn ranked_tasks(&self, query: &str) -> Vec<usize> {
        let mut ranked: Vec<_> = self
            .task_commands
            .iter()
            .enumerate()
            .filter_map(|(ix, config)| {
                search::rank(Path::new(&config.name), query).map(|score| (score, ix))
            })
            .collect();
        ranked.sort();
        ranked.into_iter().take(30).map(|(_, ix)| ix).collect()
    }

    fn ranked_completions(&self, query: &str) -> Vec<usize> {
        self.completions
            .iter()
            .enumerate()
            .filter_map(|(ix, (label, _))| {
                search::rank(Path::new(label), query).map(|score| (score, ix))
            })
            .take(30)
            .map(|(_, ix)| ix)
            .collect()
    }

    fn insert_completion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let insert = self.completions[index].1.clone();
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| {
            let source = editor.text().to_string();
            let end = editor.cursor().min(source.len());
            if !source.is_char_boundary(end) {
                return;
            }
            let start = source[..end]
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
                .last()
                .map_or(end, |(start, _)| start);
            let range =
                source[..start].encode_utf16().count()..source[..end].encode_utf16().count();
            editor.replace_text_in_range(Some(range), &insert, window, cx);
            editor.focus(window, cx);
        });
    }

    fn jump_to_hit(&mut self, hit: &SearchMatch, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position::new((hit.line - 1) as u32, (hit.column - 1) as u32),
                window,
                cx,
            );
            editor.focus(window, cx);
        });
    }

    fn current_location(&self, cx: &App) -> Option<SearchMatch> {
        let path = self.document().path()?.to_path_buf();
        let cursor = self.editor().read(cx).cursor_position();
        Some(SearchMatch {
            path,
            line: cursor.line as usize + 1,
            column: cursor.character as usize + 1,
            preview: String::new(),
        })
    }

    fn record_location(&mut self, destination: &SearchMatch, cx: &App) {
        let Some(origin) = self.current_location(cx) else {
            return;
        };
        self.locations
            .truncate(self.location_index.saturating_add(1));
        if self.locations.last() != Some(&origin) {
            self.locations.push(origin);
        }
        if self.locations.last() != Some(destination) {
            self.locations.push(destination.clone());
        }
        self.location_index = self.locations.len().saturating_sub(1);
        if self.locations.len() > 100 {
            self.locations.remove(0);
            self.location_index = self.location_index.saturating_sub(1);
        }
    }

    fn navigate_history(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.location_index as isize + direction;
        if index < 0 || index as usize >= self.locations.len() {
            return;
        }
        self.location_index = index as usize;
        let hit = self.locations[self.location_index].clone();
        self.open_at(hit, window, cx);
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_history(-1, window, cx);
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_history(1, window, cx);
    }

    fn open_at(&mut self, hit: SearchMatch, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.buffer_for_path(&hit.path) {
            self.activate(ix, window, cx);
            self.jump_to_hit(&hit, window, cx);
        } else {
            self.open(Some(hit.path.clone()), window, cx);
            // The asynchronous open completes later; location is applied in its completion handler.
            self.pending_hit = Some(hit);
        }
    }

    fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.search_mode.unwrap();
        let query = self.search_input.read(cx).text().to_string();
        let query = query.trim_end_matches(['\r', '\n']);
        let items: Vec<String> = match mode {
            SearchMode::Files => self
                .ranked_paths(query)
                .iter()
                .map(|path| {
                    path.strip_prefix(self.workspace.root().unwrap_or(Path::new("")))
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect(),
            SearchMode::Commands => self
                .ranked_commands(query)
                .iter()
                .map(|command| {
                    format!(
                        "{}  {}",
                        command.name(),
                        shortcut(command.action().as_ref(), cx).unwrap_or_default()
                    )
                })
                .collect(),
            SearchMode::Tasks => self
                .ranked_tasks(query)
                .iter()
                .map(|&ix| {
                    let config = &self.task_commands[ix];
                    format!("{}  {}", config.name, config.program)
                })
                .collect(),
            SearchMode::Completion => self
                .ranked_completions(query)
                .iter()
                .map(|&ix| self.completions[ix].0.clone())
                .collect(),
            SearchMode::Line => vec!["Enter line or line:column, then press Enter".into()],
            SearchMode::File | SearchMode::Project | SearchMode::Diagnostics => self
                .search_hits
                .iter()
                .take(30)
                .map(|hit| {
                    format!(
                        "{}:{}:{}  {}",
                        hit.path.display(),
                        hit.line,
                        hit.column,
                        hit.preview
                    )
                })
                .collect(),
        };
        div()
            .p_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.search_mode = None;
                        this.focus_editor(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "up" => {
                        this.search_selected = this.search_selected.saturating_sub(1);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "down" => {
                        this.search_selected = (this.search_selected + 1).min(29);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(Input::new(&self.search_input))
            .child(div().max_h(px(260.)).overflow_y_scrollbar().children(
                items.into_iter().enumerate().map(|(ix, label)| {
                    div()
                        .id(("search-hit", ix))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .when(ix == self.search_selected, |row| row.bg(theme::hover()))
                        .hover(|style| style.bg(theme::hover()))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_search_result(ix, window, cx)
                        }))
                }),
            ))
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

    fn reveal_tree_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut command = if cfg!(target_os = "windows") {
            let mut command = ProcessCommand::new("explorer.exe");
            let file = path.to_string_lossy();
            let file = file
                .strip_prefix(r"\\?\UNC\")
                .map(|rest| format!(r"\\{rest}"))
                .or_else(|| file.strip_prefix(r"\\?\").map(str::to_string))
                .unwrap_or_else(|| file.into_owned());
            command.arg(format!("/select,{file}"));
            command
        } else if cfg!(target_os = "macos") {
            let mut command = ProcessCommand::new("open");
            command.arg("-R").arg(path);
            command
        } else {
            let mut command = ProcessCommand::new("xdg-open");
            command.arg(path.parent().unwrap_or(path));
            command
        };
        if let Err(error) = command.spawn() {
            self.status = format!("Could not reveal {}: {error}", path.display());
            cx.notify();
        }
    }

    fn confirm_delete_tree_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked()
            || window.has_active_dialog(cx)
            || !self
                .tree_rows
                .iter()
                .any(|row| row.entry.path == path && !row.entry.is_dir)
        {
            return;
        }
        if self
            .buffer_for_path(&path)
            .is_some_and(|ix| self.buffers[ix].dirty)
        {
            self.status = "Save or close the modified file before deleting it".into();
            cx.notify();
            return;
        }
        let shell = cx.entity().downgrade();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, path) = (shell.clone(), path.clone());
            dialog
                .title("Delete File")
                .child(format!("Delete {name}? This cannot be undone."))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Delete"))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.delete_tree_file(path.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn delete_tree_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked()
            || !self
                .tree_rows
                .iter()
                .any(|row| row.entry.path == path && !row.entry.is_dir)
            || self
                .buffer_for_path(&path)
                .is_some_and(|ix| self.buffers[ix].dirty)
        {
            return;
        }
        self.busy = true;
        self.status = format!("Deleting {}…", path.display());
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = executor
                .spawn({
                    let path = path.clone();
                    async move { fs::remove_file(&path) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if let Some(ix) = this.buffer_for_path(&path) {
                            this.remove_buffer(ix, window, cx);
                        }
                        this.refresh_parent(Some(&path), cx);
                        this.index_workspace(cx);
                        this.refresh_git(cx);
                        this.status = format!("Deleted {}", path.display());
                    }
                    Err(error) => {
                        this.status = format!("Delete failed: {}: {error}", path.display())
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
        let language = syntax::language(buffer.document.path());
        let offset = folding::display_to_source(&buffer.folds, display_offset, false);
        let position = position_at(&buffer.document.text().to_string(), offset);
        let request = buffer
            .document
            .path()
            .and_then(|path| self.servers.read(cx).definition(path, language, position));
        let Some(request) = request else {
            return self.go_to_declaration_fallback(display_offset, window, cx);
        };
        self.status = "Looking up the declaration…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let found = request.await;
            let _ = this.update_in(cx, |this, window, cx| match found {
                Some((path, position)) => this.open_declaration_at(path, position, window, cx),
                None => {
                    this.go_to_declaration_fallback(display_offset, window, cx);
                }
            });
        });
        true
    }

    fn go_to_definition_action(
        &mut self,
        _: &GoToDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff {
            return;
        }
        let offset = self.editor().read(cx).cursor();
        self.go_to_declaration(offset, window, cx);
    }

    fn open_declaration_at(
        &mut self,
        path: PathBuf,
        position: Position,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hit = SearchMatch {
            path,
            line: position.line as usize + 1,
            column: position.character as usize + 1,
            preview: String::new(),
        };
        self.record_location(&hit, cx);
        self.open_at(hit, window, cx);
    }

    fn hover_info(&mut self, _: &HoverInfo, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = self.buffer();
        let editor = buffer.editor.clone();
        let cursor = editor.read(cx).cursor();
        let offset = folding::display_to_source(&buffer.folds, cursor, false);
        let source = buffer.document.text().to_string();
        let language = syntax::language(buffer.document.path());
        let origin = buffer.document.path().map(Path::to_path_buf);
        let request = origin.as_deref().and_then(|path| {
            self.servers
                .read(cx)
                .hover(path, language, position_at(&source, offset))
        });
        let root = self.project_directory().map(Path::to_path_buf);
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let mut hover = match request {
                Some(request) => request.await,
                None => None,
            };
            if hover.is_none() {
                let source = source.clone();
                hover = cx
                    .background_executor()
                    .spawn(async move {
                        language_server::declaration_hover(
                            language,
                            &source,
                            offset,
                            root.as_deref(),
                            origin.as_deref(),
                        )
                    })
                    .await;
            }
            let _ = this.update_in(cx, |this, window, cx| match hover {
                Some(hover) => {
                    editor.update(cx, |editor, cx| {
                        editor.show_hover(cursor, &hover, window, cx)
                    });
                    this.status = "Showing the declaration".into();
                }
                None => this.status = "No declaration information here".into(),
            });
        });
        self.status = "Looking up the declaration…".into();
        cx.notify();
    }

    fn complete_code(&mut self, _: &CompleteCode, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = self.buffer();
        let offset =
            folding::display_to_source(&buffer.folds, buffer.editor.read(cx).cursor(), false);
        let position = position_at(&buffer.document.text().to_string(), offset);
        let language = syntax::language(buffer.document.path());
        let buffer_id = buffer.id;
        let request = buffer
            .document
            .path()
            .and_then(|path| self.servers.read(cx).completions(path, language, position));
        let Some(request) = request else {
            self.status = "No language server is running for this file".into();
            cx.notify();
            return;
        };
        self.status = "Loading completions…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let items = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.completions = items
                    .into_iter()
                    .filter_map(|item| {
                        let insert = item
                            .text_edit
                            .as_ref()
                            .map(|edit| match edit {
                                lsp_types::CompletionTextEdit::Edit(edit) => edit.new_text.clone(),
                                lsp_types::CompletionTextEdit::InsertAndReplace(edit) => {
                                    edit.new_text.clone()
                                }
                            })
                            .or_else(|| item.insert_text.clone())
                            .unwrap_or_else(|| item.label.clone());
                        (!insert.contains('$')).then_some((item.label, insert))
                    })
                    .take(100)
                    .collect();
                if this.buffer().id == buffer_id && !this.completions.is_empty() {
                    this.show_search(SearchMode::Completion, window, cx);
                } else if this.completions.is_empty() {
                    this.status = "No completions here".into();
                }
                cx.notify();
            });
        });
    }

    fn go_to_declaration_fallback(
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
            Ok(settings) => {
                self.show_sidebar = settings.show_sidebar;
                settings.apply(Some(window), cx)
            }
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
                                    .child("Hotkeys, colors, tasks and language servers"),
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

    fn render_tree_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = &self.tree_rows[ix];
        let path = row.entry.path.clone();
        let is_file = !row.entry.is_dir;
        let shell = cx.entity().downgrade();
        let is_open = !row.entry.is_dir && self.document().path() == Some(row.entry.path.as_path());
        let git = self.git.read(cx);
        let change = git.tree_change(&row.entry.path);
        let ignored = git.tree_ignored(&row.entry.path);
        let letter = change.filter(|_| !row.entry.is_dir).map(Change::letter);
        let kind = FileKind::of(&row.entry.path);
        let (chevron, icon) = match (row.entry.is_dir, row.expanded) {
            (true, true) => (Some(IconName::ChevronDown), Icon::new(IconName::FolderOpen)),
            (true, false) => (Some(IconName::ChevronRight), Icon::new(IconName::Folder)),
            (false, _) => (None, Icon::new(kind.icon())),
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
            .when(ignored, |row| row.text_color(theme::subtle()))
            .when_some(change.filter(|_| !ignored), |row, change| {
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
            .child(icon.size(px(15.)).flex_shrink_0().text_color(if ignored {
                theme::subtle()
            } else if row.entry.is_dir {
                theme::accent()
            } else {
                theme::file_icon(kind)
            }))
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
            .context_menu(move |menu, _, _| {
                if !is_file {
                    return menu;
                }
                let open_shell = shell.clone();
                let open_path = path.clone();
                let reveal_shell = shell.clone();
                let reveal_path = path.clone();
                let delete_shell = shell.clone();
                let delete_path = path.clone();
                menu.item(
                    PopupMenuItem::new("View File").on_click(move |_, window, cx| {
                        let _ = open_shell.update(cx, |shell, cx| {
                            if !shell.blocked() {
                                shell.open(Some(open_path.clone()), window, cx);
                            }
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("Reveal in File Explorer").on_click(move |_, _, cx| {
                        let _ = reveal_shell
                            .update(cx, |shell, cx| shell.reveal_tree_file(&reveal_path, cx));
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Delete File…").on_click(move |_, window, cx| {
                        let _ = delete_shell.update(cx, |shell, cx| {
                            shell.confirm_delete_tree_file(delete_path.clone(), window, cx);
                        });
                    }),
                )
            })
            .into_any_element()
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

    fn gutter_row(&self, y: Pixels, cx: &App) -> Option<usize> {
        let editor = self.editor().read(cx);
        let geometry = editor.text_geometry()?;
        if y < geometry.viewport.top() || y > geometry.viewport.bottom() {
            return None;
        }
        let row = ((y - geometry.origin.y) / geometry.line_height).floor();
        (row >= 0. && (row as usize) < editor.text().lines_len()).then_some(row as usize)
    }

    fn render_debug_markers(&self, cx: &App) -> impl IntoElement {
        let editor = self.editor().clone();
        let path = self.document().path().map(Path::to_path_buf);
        let debugger = self.debugger.read(cx);
        let markers: Vec<(usize, bool)> = path
            .as_deref()
            .and_then(|path| debugger.lines(path).map(|lines| (path, lines)))
            .map(|(path, lines)| {
                lines
                    .iter()
                    .map(|line| {
                        (
                            self.display_row(*line, cx),
                            debugger.is_rejected(path, *line),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let execution = path
            .as_deref()
            .and_then(|path| debugger.execution_line(path))
            .map(|line| self.display_row(line, cx));
        let hover = self
            .breakpoint_hover
            .filter(|row| !markers.iter().any(|(marked, _)| marked == row));
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let Some(geometry) = editor.read(cx).text_geometry() else {
                    return;
                };
                let top = |row: usize| geometry.origin.y + geometry.line_height * row as f32;
                let visible = |row: usize| {
                    let y = top(row);
                    y + geometry.line_height > bounds.top() && y < bounds.bottom()
                };
                let center_x = bounds.left() + EDITOR_PADDING_LEFT / 2.;
                let radius = px(BREAKPOINT_RADIUS);
                let dot = |row: usize, fill: Hsla, border: Hsla, window: &mut Window| {
                    let center_y = top(row) + geometry.line_height / 2.;
                    window.paint_quad(quad(
                        Bounds::new(
                            point(center_x - radius, center_y - radius),
                            size(radius * 2., radius * 2.),
                        ),
                        radius,
                        fill,
                        px(1.5),
                        border,
                        BorderStyle::Solid,
                    ));
                };
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    if let Some(row) = execution.filter(|row| visible(*row)) {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left(), top(row)),
                                size(bounds.size.width, geometry.line_height),
                            ),
                            theme::execution_wash(),
                        ));
                    }
                    let color = Hsla::from(theme::breakpoint());
                    if let Some(row) = hover.filter(|row| visible(*row)) {
                        dot(row, color.opacity(0.35), color.opacity(0.), window);
                    }
                    for (row, rejected) in markers.iter().copied().filter(|(row, _)| visible(*row))
                    {
                        if rejected {
                            dot(row, color.opacity(0.), color, window);
                        } else {
                            dot(row, color, color, window);
                        }
                    }
                    if let Some(row) = execution.filter(|row| visible(*row)) {
                        let center_y = top(row) + geometry.line_height / 2.;
                        let half = px(5.);
                        let mut arrow = PathBuilder::fill();
                        arrow.move_to(point(center_x - half, center_y - half));
                        arrow.line_to(point(center_x + half, center_y));
                        arrow.line_to(point(center_x - half, center_y + half));
                        arrow.close();
                        if let Ok(path) = arrow.build() {
                            window.paint_path(path, theme::execution_line());
                        }
                    }
                });
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
                .child(self.render_debug_markers(cx))
                .child(
                    div()
                        .id("breakpoint-gutter")
                        .debug_selector(|| "breakpoint-gutter".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(EDITOR_PADDING_LEFT)
                        .cursor_pointer()
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if !hovered && this.breakpoint_hover.take().is_some() {
                                cx.notify();
                            }
                        }))
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            let row = this.gutter_row(event.position.y, cx);
                            if this.breakpoint_hover != row {
                                this.breakpoint_hover = row;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if this.blocked() || this.show_diff {
                                    return;
                                }
                                if let Some(row) = this.gutter_row(event.position.y, cx) {
                                    this.toggle_breakpoint_at_row(row, cx);
                                }
                            }),
                        ),
                )
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
                                ToolPanel::Run => {
                                    this.active_panel = ToolPanel::Run;
                                    cx.notify();
                                }
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
                ToolPanel::Run => div()
                    .flex_1()
                    .min_h_0()
                    .bg(theme::background())
                    .child(self.task_terminal.clone())
                    .into_any_element(),
                ToolPanel::Debug => div()
                    .flex_1()
                    .min_h_0()
                    .child(self.debugger.clone())
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
        let server = (!self.show_diff)
            .then(|| {
                self.servers
                    .read(cx)
                    .state(syntax::language(self.document().path()))
            })
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
                .child("Git")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(SidebarPanel::Git, cx))),
            )
            .child(divider())
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
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::search_project))
            .on_action(cx.listener(Self::show_diagnostics))
            .on_action(cx.listener(Self::go_to_definition_action))
            .on_action(cx.listener(Self::hover_info))
            .on_action(cx.listener(Self::complete_code))
            .on_action(cx.listener(Self::toggle_breakpoint))
            .on_action(cx.listener(Self::debug_continue))
            .on_action(cx.listener(Self::debug_step_over))
            .on_action(cx.listener(Self::debug_step_into))
            .on_action(cx.listener(Self::debug_step_out))
            .on_action(cx.listener(Self::debug_interrupt))
            .on_action(cx.listener(Self::debug_stop))
            .on_action(cx.listener(Self::debug_restart))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::find_in_file))
            .on_action(cx.listener(Self::run_command))
            .on_action(cx.listener(Self::stop_command))
            .on_action(cx.listener(Self::start_debugger))
            .on_action(cx.listener(Self::reload_file))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::stage_all)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::pull)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::push)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::fetch)))
            .child(self.render_title_bar(cx))
            .when(self.buffer().external_change, |view| {
                view.child(
                    div()
                        .px_3()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(theme::panel())
                        .child("File changed on disk. Reload discards your edits.")
                        .child(
                            Button::new("reload-external")
                                .small()
                                .label("Reload")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reload_file(&ReloadFile, window, cx)
                                })),
                        )
                        .child(
                            Button::new("save-external-as")
                                .small()
                                .label("Save As…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_file_as(&SaveFileAs, window, cx)
                                })),
                        ),
                )
            })
            .when(self.search_mode.is_some(), |view| {
                view.child(self.render_search(cx))
            })
            .when(self.pending.is_some(), |view| {
                view.child(self.render_save_prompt(cx))
            })
            .child(
                div().flex_1().min_h_0().flex().child(
                    div().flex_1().min_w_0().h_full().child(
                        h_resizable("sidebar-split")
                            .with_state(&self.sidebar_split)
                            .when(self.show_sidebar, |split| {
                                split.child(
                                    resizable_panel()
                                        .size(px(240.))
                                        .size_range(px(160.)..px(480.))
                                        .child(self.render_sidebar(cx)),
                                )
                            })
                            .child(
                                v_resizable("tool-panel-split")
                                    .with_state(&self.tool_panel_split)
                                    .child(resizable_panel().child(self.render_editor(window, cx)))
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

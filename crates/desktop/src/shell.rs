//! The IDE window: [`IdeShell`] owns the open buffers, workspace, panels and services.
//!
//! This file holds the shell's state and construction; behaviour lives in child modules:
//!
//! - Documents: `buffers`, `saving`, `close_flow`, `disk_changes`, `encoding`,
//!   `recovery`, `autosave`, `persistence` (session).
//! - Editing: `edits`, `line_commands`, `cursors`, `folds`, `completion`,
//!   `signature_help`, `editor_preferences`, `language_mode`, `vim_mode`.
//! - Layout and rendering: `layout` (window + action routing), `chrome` (title and
//!   status bars), `editor_view` (editor area and overlays), `tabs`, `groups` (split
//!   editors), `panels` (sidebar and tool panel).
//! - Navigation and search: `palette`, `location_history`, `declarations`, `outline`,
//!   `picker`, `project_search`.
//! - Workspace: `folders`, `roots` (multi-root), `tree` and `explorer`, `fs_sync`,
//!   `settings_files`, `settings_commands`.
//! - Language servers: `language_servers`, `lsp_features`, `workspace_edits`.
//! - Run, debug, test and Git: `run_debug`, `debugging`, `tasks`, `problems`,
//!   `terminals`, `testing`, `git_features`.

use std::{
    cell::{Cell, RefCell},
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

mod autosave;
mod buffers;
mod chrome;
mod close_flow;
mod completion;
mod cursors;
mod debugging;
mod declarations;
mod disk_changes;
mod editor_preferences;
mod editor_view;
mod edits;
mod encoding;
mod explorer;
mod folders;
mod folds;
mod fs_sync;
mod git_features;
mod groups;
mod language_mode;
mod language_servers;
mod layout;
mod line_commands;
mod location_history;
mod lsp_features;
mod outline;
mod palette;
mod panels;
mod persistence;
mod picker;
mod problems;
mod project_search;
mod recovery;
mod roots;
mod run_debug;
mod saving;
mod settings_commands;
mod settings_files;
mod signature_help;
mod tabs;
mod tasks;
mod terminals;
mod testing;
mod tree;
mod vim_mode;
mod workspace_edits;

#[cfg(test)]
mod ide_tests;
#[cfg(test)]
mod lsp_tests;
#[cfg(test)]
mod mock_lsp;

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shell_feature_tests.rs"]
mod feature_tests;

pub const EDITOR_CONTEXT: &str = "Editor";
/// Active while a completion snippet has remaining tab stops.
pub const SNIPPET_CONTEXT: &str = "Snippet";
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

/// The first line of a document (bounded), used for shebang detection.
fn first_line(document: &Document) -> String {
    document
        .text()
        .chars()
        .take_while(|character| *character != '\n')
        .take(256)
        .collect()
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SidebarPanel {
    #[default]
    Folder,
    Git,
    Search,
    Outline,
    Testing,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ToolPanel {
    #[default]
    Terminal,
    Run,
    Debug,
    Problems,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    Files,
    Commands,
    File,
    Tasks,
    Diagnostics,
    Line,
    Languages,
    Items,
}

impl ToolPanel {
    const ALL: [Self; 4] = [Self::Terminal, Self::Run, Self::Debug, Self::Problems];

    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Run => "Run",
            Self::Debug => "Debug",
            Self::Problems => "Problems",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Terminal => Icon::new(IconName::SquareTerminal),
            Self::Run => Icon::new(AppIcon::Play),
            Self::Debug => Icon::new(AppIcon::Bug),
            Self::Problems => Icon::new(IconName::TriangleAlert),
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

/// Quick Open memo key: query, `search_paths` generation, most recent file.
type RankedPathsKey = (String, u64, Option<PathBuf>);
/// Brace-pair memo key: editor entity and its text revision.
type BracePairsKey = (gpui::EntityId, usize);
/// `{`/`}` byte offsets of an editor's text (see `folding::brace_pairs`).
type BracePairs = Rc<Vec<(usize, usize)>>;

/// Marks a picked file that is a workspace definition (handled after the picker).
const OPEN_WORKSPACE_MARKER: &str = "\u{0}open-workspace:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAction {
    CloseBuffer(BufferId),
    LeaveDiff,
    CloseWindow,
}

struct Buffer {
    id: BufferId,
    document: Document,
    /// Language mode (see `ide_core::language`); detected or chosen by the user.
    language: &'static str,
    /// The user picked the language mode explicitly; keep it across saves.
    language_overridden: bool,
    preferences: crate::preferences::EditorPreferences,
    editor: Entity<InputState>,
    dirty: bool,
    folds: Vec<Fold>,
    external_change: bool,
    /// The file was deleted on disk while open.
    deleted_on_disk: bool,
    /// Restored from a recovery snapshot.
    recovered: bool,
    /// Tab title for buffers without a file (e.g. recovered copies).
    title_override: Option<String>,
    /// One view per editor group showing this document (IDE-012). `editor` is the view
    /// commands act on: the one in the focused group, or the last focused one.
    views: Vec<groups::BufferView>,
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
    /// The explorer item last clicked (target of explorer commands).
    tree_selection: Option<PathBuf>,
    tree_scroll: UniformListScrollHandle,
    sidebar_split: Entity<ResizableState>,
    tool_panel_split: Entity<ResizableState>,
    active_sidebar: SidebarPanel,
    show_sidebar: bool,
    active_panel: ToolPanel,
    /// The active interactive terminal (one of `terminal_sessions`).
    terminal: Entity<TerminalView>,
    terminal_sessions: Vec<terminals::TerminalSession>,
    active_terminal: usize,
    next_terminal_id: u64,
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
    pending_hit: Option<SearchMatch>,
    auto_save: autosave::AutoSaveState,
    /// Appended to the status when the running Save All finishes.
    save_all_note: Option<String>,
    search_panel: Entity<crate::search_panel::SearchPanel>,
    search_state: project_search::SearchState,
    edit_history: workspace_edits::EditHistory,
    picker: Option<picker::Picker>,
    symbol_task: Task<()>,
    outline_timer: Task<()>,
    /// Latest document symbols: buffer, document version, symbols.
    outline: Option<(BufferId, Option<i32>, Vec<lsp_features::Symbol>)>,
    workspace_symbols: Vec<lsp_types::WorkspaceSymbol>,
    workspace_symbol_language: Option<&'static str>,
    code_actions: Option<(lsp_features::Target, Vec<lsp_types::CodeActionOrCommand>)>,
    /// Full diagnostics as published by language servers, per file.
    lsp_diagnostics: HashMap<PathBuf, Vec<lsp_types::Diagnostic>>,
    snippet: Option<completion::SnippetSession>,
    signature: signature_help::SignatureState,
    fs_sync: fs_sync::FsSync,
    recovery: recovery::RecoveryState,
    debug_profile: Option<String>,
    attach_profile: Option<settings::DebuggerConfig>,
    tasks: tasks::TaskState,
    problems: problems::ProblemsState,
    git_state: git_features::GitState,
    testing: testing::TestingState,
    cursors: cursors::CursorState,
    groups: groups::Groups,
    roots: roots::RootsState,
    /// Bumped whenever `search_paths` changes (invalidates `ranked_paths_cache`).
    search_paths_generation: Cell<u64>,
    ranked_paths_cache: RefCell<Option<(RankedPathsKey, Vec<PathBuf>)>>,
    /// Brace pairs of an editor at a text revision (for the brace guide).
    brace_pairs_cache: RefCell<Option<(BracePairsKey, BracePairs)>>,
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
            tree_selection: None,
            tree_scroll: UniformListScrollHandle::new(),
            sidebar_split: cx.new(|_| ResizableState::default()),
            tool_panel_split: cx.new(|_| ResizableState::default()),
            active_sidebar: SidebarPanel::default(),
            show_sidebar: settings::current(cx).show_sidebar,
            active_panel: ToolPanel::default(),
            terminal: cx.new(TerminalView::new),
            terminal_sessions: Vec::new(),
            active_terminal: 0,
            next_terminal_id: 1,
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
            pending_hit: None,
            auto_save: Default::default(),
            save_all_note: None,
            search_panel: cx.new(|cx| crate::search_panel::SearchPanel::new(window, cx)),
            search_state: Default::default(),
            edit_history: Default::default(),
            picker: None,
            symbol_task: Task::ready(()),
            outline_timer: Task::ready(()),
            outline: None,
            workspace_symbols: Vec::new(),
            workspace_symbol_language: None,
            code_actions: None,
            lsp_diagnostics: HashMap::new(),
            snippet: None,
            signature: Default::default(),
            fs_sync: Default::default(),
            recovery: recovery::RecoveryState::new(Self::recovery_directory(cx)),
            debug_profile: None,
            attach_profile: None,
            tasks: Default::default(),
            problems: Default::default(),
            git_state: Default::default(),
            testing: Default::default(),
            cursors: Default::default(),
            groups: Default::default(),
            roots: Default::default(),
            brace_pairs_cache: RefCell::new(None),
            search_paths_generation: Cell::new(0),
            ranked_paths_cache: RefCell::new(None),
            _subscriptions: Vec::new(),
        };
        let shell_cursors_live = shell.cursors_live();
        let search_subscription = Self::search_panel_subscription(&shell.search_panel, window, cx);
        shell._subscriptions = vec![
            search_subscription,
            cx.subscribe_in(
                &shell.servers.clone(),
                window,
                |this, _, event, window, cx| this.handle_server_event(event, window, cx),
            ),
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
                        let switching = this.diff_view.read(cx).switching();
                        if !this.blocked() {
                            if switching {
                                this.request(PendingAction::LeaveDiff, window, cx);
                            } else {
                                this.focus_diff(window, cx);
                            }
                        } else if switching && this.pending != Some(PendingAction::LeaveDiff) {
                            this.cancel_switch(cx);
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
                    DebuggerEvent::StateChanged => this.save_debug_state(cx),
                },
            ),
            cx.observe_in(&shell.diff_view, window, |this, diff_view, window, cx| {
                if this.show_diff && diff_view.read(cx).target().is_none() {
                    this.show_diff = false;
                    this.focus_editor(window, cx);
                }
                cx.notify();
            }),
            {
                // Runs before key bindings resolve, so bound editing keys are seen too.
                let shell = cx.entity().downgrade();
                let live = shell_cursors_live.clone();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if live.get()
                        && let Some(shell) = shell.upgrade()
                    {
                        shell.update(cx, |this, cx| {
                            this.cursors_keystroke(&event.keystroke, window, cx)
                        });
                    }
                })
            },
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_git(cx);
                    this.check_disk_changes(window, cx);
                } else {
                    this.auto_save_on_focus_change(None, cx);
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
                let TerminalEvent::Exited(lines) = event else {
                    return;
                };
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
        shell.terminal_sessions.push(terminals::TerminalSession {
            id: 1,
            name: "Terminal".into(),
            view: shell.terminal.clone(),
        });
        let buffer = shell.new_buffer(Document::default(), window, cx);
        shell.buffers.push(buffer);
        shell.activate(0, window, cx);
        shell
    }

    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    fn blocked(&self) -> bool {
        self.busy || self.pending.is_some()
    }
}

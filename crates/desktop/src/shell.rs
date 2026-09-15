use std::{
    cell::Cell,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::{
    Context, Div, Entity, Focusable, IntoElement, KeyDownEvent, MouseButton, PathPromptOptions,
    Pixels, Render, Stateful, Subscription, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Root, Sizable as _, ThemeMode, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Position, TabSize},
    menu::AppMenuBar,
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    switch::Switch,
};
use ide_core::{
    document::Document,
    git::Change,
    vim::Command as VimCommand,
    workspace::{self, TreeRow, Workspace},
};

use crate::{
    commands::*,
    folding::{self, Fold},
    git_panel::{GitPanel, GitPanelEvent},
    syntax,
    terminal_view::TerminalView,
    theme,
    vim::VimInput,
};

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

const INDENT_WIDTH: usize = 2;

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
    pending: Option<PendingAction>,
    busy: bool,
    vim: Option<VimInput>,
    status: String,
    _subscriptions: Vec<Subscription>,
}

impl IdeShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            git: cx.new(|cx| GitPanel::new(window, cx)),
            pending: None,
            busy: false,
            vim: None,
            status: "Ready — open a file or start typing".into(),
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
                },
            ),
            cx.observe(&shell.git, |_, _, cx| cx.notify()),
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
        self.activate(active, window, cx);
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
                let saved = results.iter().filter(|(_, result)| result.is_ok()).count();
                for (id, result) in results {
                    match result {
                        Ok(saved) => {
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
                match failures.first() {
                    Some(error) => this.status = format!("Save failed: {error}"),
                    None => {
                        let plural = if saved == 1 { "" } else { "s" };
                        this.status = format!("Saved {saved} file{plural}");
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
        self.request(PendingAction::CloseBuffer(self.buffer().id), window, cx);
    }

    fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let len = self.buffers.len() as isize;
        let ix = (self.active as isize + step).rem_euclid(len) as usize;
        self.activate(ix, window, cx);
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
        if self.blocked() {
            return;
        }
        let cursor = self.editor().read(cx).cursor();
        let active = self.active;
        let source = self.buffers[active].document.text().to_string();
        let Some(toggle) = folding::toggle(&source, &mut self.buffers[active].folds, cursor) else {
            self.status = "No brace block at cursor".into();
            cx.notify();
            return;
        };
        let displayed = folding::projected(&source, &self.buffers[active].folds);
        let position = position_at(&displayed, toggle.cursor);
        self.editor().update(cx, |editor, cx| {
            editor.set_value(displayed, window, cx);
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

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        let vim_enabled = Rc::new(Cell::new(self.vim.is_some()));
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, vim_enabled) = (shell.clone(), vim_enabled.clone());
            dialog.title("Settings").w(px(420.)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
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
                    .child(
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
                    ),
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
        div()
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
                                    .disabled(self.blocked())
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
            .gap_1()
            .pl(px(8. + 12. * row.depth as f32))
            .pr_2()
            .cursor_pointer()
            .hover(|style| style.bg(theme::border()))
            .when(is_open, |row| {
                row.bg(theme::border()).text_color(theme::accent())
            })
            .when_some(change, |row, change| {
                row.text_color(theme::git_change(change))
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.entry.name.clone()),
            )
            .when_some(letter, |row, letter| {
                row.child(div().flex_shrink_0().child(letter))
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

    fn document_path_label(&self) -> String {
        let Some(path) = self.document().path() else {
            return "Not saved to disk".into();
        };
        self.workspace
            .root()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tabs")
            .debug_selector(|| "tabs".into())
            .h(px(34.))
            .flex_shrink_0()
            .flex()
            .overflow_x_scroll()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .children(
                self.buffers
                    .iter()
                    .enumerate()
                    .map(|(ix, buffer)| self.render_tab(ix, buffer, cx)),
            )
    }

    fn render_tab(&self, ix: usize, buffer: &Buffer, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = buffer.id;
        let active = ix == self.active;
        div()
            .id(("tab", id))
            .h_full()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_1()
            .border_r_1()
            .border_color(theme::border())
            .cursor_pointer()
            .map(|tab| {
                if active {
                    tab.bg(theme::background()).text_color(theme::text())
                } else {
                    tab.text_color(theme::muted())
                        .hover(|style| style.bg(theme::border()))
                }
            })
            .child(self.tab_title(buffer))
            .child(
                div()
                    .w(px(10.))
                    .text_color(theme::accent())
                    .child(if buffer.dirty { "•" } else { "" }),
            )
            .child(
                div()
                    .id(("close-tab", id))
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .hover(|style| style.bg(theme::border()))
                    .child(Icon::new(IconName::Close).size(px(12.)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request(PendingAction::CloseBuffer(id), window, cx);
                    })),
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

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let vim_context = self.vim.as_ref().map(VimInput::key_context);
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(
                div()
                    .h(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_b_1()
                    .border_color(theme::border())
                    .text_color(theme::muted())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.document_path_label()),
                    )
                    .child(
                        Button::new("toggle-fold")
                            .label("Fold")
                            .xsmall()
                            .ghost()
                            .disabled(self.blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_fold(&ToggleFold, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when_some(vim_context, |editor, context| {
                        editor
                            .key_context(context)
                            .capture_key_down(cx.listener(Self::editor_key_down))
                    })
                    .child(
                        Input::new(self.editor())
                            .h_full()
                            .w_full()
                            .bordered(false)
                            .focus_bordered(false)
                            .appearance(false)
                            .disabled(self.blocked())
                            .font_family(if cfg!(target_os = "windows") {
                                "Consolas"
                            } else {
                                "monospace"
                            })
                            .text_size(px(14.))
                            .bg(theme::background())
                            .text_color(theme::text()),
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
                            .on_click(cx.listener(move |this, _, window, cx| match panel {
                                ToolPanel::Terminal => this.focus_terminal(window, cx),
                                ToolPanel::Debug => {
                                    this.show_debug_panel(&ShowDebugPanel, window, cx)
                                }
                            }))
                            .child(panel.label())
                    })),
            )
            .child(match self.active_panel {
                ToolPanel::Terminal => div()
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus_terminal(window, cx)),
                    )
                    .child(self.terminal.clone())
                    .into_any_element(),
                ToolPanel::Debug => div()
                    .p_3()
                    .text_color(theme::muted())
                    .child("Debug adapter integration is not implemented yet.")
                    .into_any_element(),
            })
    }
}

impl Render for IdeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buffer = self.buffer();
        let name = format!(
            "{}{}",
            buffer.document.name(),
            if buffer.dirty { " •" } else { "" }
        );
        let dirty = buffer.dirty;
        window.set_window_title(&match self.workspace.root() {
            Some(_) => format!("{name} — {} — Hephaestus", self.workspace.display_name()),
            None => format!("{name} — Hephaestus"),
        });
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
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::show_folder_panel))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::show_git_panel))
            .on_action(cx.listener(Self::show_debug_panel))
            .on_action(cx.listener(Self::toggle_vim_mode))
            .on_action(cx.listener(Self::toggle_fold))
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
                            .child(div().text_color(theme::muted()).child(name)),
                    )
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        this.request(PendingAction::CloseWindow, window, cx)
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
                        .child(div().flex_1().child(format!(
                            "Save changes to {} before closing?",
                            self.document().name()
                        )))
                        .child(
                            Button::new("confirm-save")
                                .label("Save")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save(SaveTarget::Current, None, window, cx)
                                })),
                        )
                        .child(
                            Button::new("discard")
                                .label("Discard")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.discard(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pending = None;
                                    this.focus_editor(window, cx);
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
                                .child(resizable_panel().child(self.render_editor(cx)))
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
                    .child(div().flex_1().min_w_0().truncate().child(status))
                    .child(
                        div()
                            .id("quick-folder")
                            .debug_selector(|| "quick-folder".into())
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(if self.active_sidebar == SidebarPanel::Folder {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .hover(|style| style.text_color(theme::text()))
                            .child("Folder")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_folder_panel(&ShowFolderPanel, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("quick-terminal")
                            .debug_selector(|| "quick-terminal".into())
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(if self.active_panel == ToolPanel::Terminal {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .hover(|style| style.text_color(theme::text()))
                            .child("Terminal")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.focus_terminal(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("quick-git")
                            .debug_selector(|| "quick-git".into())
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(if self.active_sidebar == SidebarPanel::Git {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .hover(|style| style.text_color(theme::text()))
                            .child("Git")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_git_panel(&ShowGitPanel, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("quick-debug")
                            .debug_selector(|| "quick-debug".into())
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(if self.active_panel == ToolPanel::Debug {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .hover(|style| style.text_color(theme::text()))
                            .child("Debug")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_debug_panel(&ShowDebugPanel, window, cx)
                            })),
                    )
                    .when_some(vim_mode, |bar, mode| {
                        bar.child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme::accent())
                                .child(mode),
                        )
                    })
                    .when_some(self.git.read(cx).branch(), |bar, branch| {
                        bar.child(
                            div()
                                .id("status-branch")
                                .flex_shrink_0()
                                .cursor_pointer()
                                .hover(|style| style.text_color(theme::text()))
                                .child(branch)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_git_panel(&ShowGitPanel, window, cx)
                                })),
                        )
                    })
                    .child(div().flex_shrink_0().child(format!(
                        "{} | UTF-8 | {}",
                        if dirty { "Modified" } else { "Saved" },
                        self.document().line_ending().label()
                    ))),
            )
            .children(Root::render_dialog_layer(window, cx))
    }
}

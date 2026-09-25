use std::{
    collections::{HashMap, HashSet},
    io,
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{
    Action, App, Context, Corner, Div, Entity, EventEmitter, Focusable as _, FontWeight,
    IntoElement, Render, Stateful, Task, UniformListScrollHandle, WeakEntity, Window, actions, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use ide_core::git::{self, Change, FileStatus, Repository, Status};

use crate::{
    assets::AppIcon,
    commands::{self, CloneRepository, EditSettings},
    diff_view::DiffView,
    theme,
    ui::{self, ROW_GROUP},
};

#[cfg(test)]
#[path = "git_panel_tests.rs"]
pub(crate) mod tests;

actions!(git, [Commit, StageAll, Pull, Push, Fetch]);

pub const COMMIT_CONTEXT: &str = "GitCommit";

const ROW_HEIGHT: f32 = 24.;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const NO_REPOSITORY: &str = "Open a folder in a Git repository first";

pub enum GitPanelEvent {
    OpenFile(PathBuf),
    ShowDiff,
    Status(String),
}

#[derive(Clone, Copy)]
enum Remote {
    Pull,
    Push,
    Fetch,
}

impl Remote {
    fn name(self) -> &'static str {
        match self {
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Fetch => "Fetch",
        }
    }

    fn progress(self) -> &'static str {
        match self {
            Self::Pull => "Pulling…",
            Self::Push => "Pushing…",
            Self::Fetch => "Fetching…",
        }
    }

    fn run(self, repository: &Repository) -> io::Result<String> {
        match self {
            Self::Pull => repository.pull(),
            Self::Push => repository.push(),
            Self::Fetch => repository.fetch(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Selection {
    pub(crate) relative: String,
    pub(crate) staged: bool,
}

#[derive(Clone, Copy)]
enum Row {
    Header { staged: bool },
    File { ix: usize, staged: bool },
}

enum Notice {
    Info(String),
    Error(String),
}

pub struct GitPanel {
    directory: Option<PathBuf>,
    repository: Option<Repository>,
    status: Status,
    loaded: bool,
    load_error: Option<String>,
    tree_changes: HashMap<PathBuf, Change>,
    ignored_paths: HashSet<PathBuf>,
    rows: Vec<Row>,
    staged_count: usize,
    unstaged_count: usize,
    selected: Option<Selection>,
    checked: HashSet<Selection>,
    diff_view: Entity<DiffView>,
    notice: Option<Notice>,
    busy: bool,
    commit_message: Entity<InputState>,
    list_scroll: UniformListScrollHandle,
    refresh_task: Option<Task<()>>,
    refreshing: bool,
    _poll_task: Task<()>,
}

impl EventEmitter<GitPanelEvent> for GitPanel {}

fn side(file: &FileStatus, staged: bool) -> Option<Change> {
    if staged { file.staged } else { file.unstaged }
}

impl GitPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let poll_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let polled = this.update_in(cx, |this, window, cx| {
                    if window.is_window_active() && !this.refreshing && !this.busy {
                        this.refresh(cx);
                    }
                });
                if polled.is_err() {
                    break;
                }
            }
        });
        Self {
            directory: None,
            repository: None,
            status: Status::default(),
            loaded: false,
            load_error: None,
            tree_changes: HashMap::new(),
            ignored_paths: HashSet::new(),
            rows: Vec::new(),
            staged_count: 0,
            unstaged_count: 0,
            selected: None,
            checked: HashSet::new(),
            diff_view: cx.new(DiffView::new),
            notice: None,
            busy: false,
            commit_message: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(1, 6)
                    .placeholder("Commit message")
            }),
            list_scroll: UniformListScrollHandle::new(),
            refresh_task: None,
            refreshing: false,
            _poll_task: poll_task,
        }
    }

    pub fn diff_view(&self) -> &Entity<DiffView> {
        &self.diff_view
    }

    pub fn branch(&self) -> Option<String> {
        self.repository.as_ref().map(|_| self.status.branch.label())
    }

    pub fn tree_change(&self, path: &Path) -> Option<Change> {
        self.tree_changes.get(path).copied()
    }

    pub fn tree_ignored(&self, path: &Path) -> bool {
        self.repository.as_ref().is_some_and(|repository| {
            path.starts_with(repository.root())
                && path
                    .ancestors()
                    .take_while(|ancestor| ancestor.starts_with(repository.root()))
                    .any(|ancestor| self.ignored_paths.contains(ancestor))
        })
    }

    pub fn set_directory(&mut self, directory: Option<PathBuf>, cx: &mut Context<Self>) -> bool {
        if directory == self.directory {
            return false;
        }
        if let (Some(directory), Some(repository)) = (&directory, &self.repository)
            && directory.starts_with(repository.root())
        {
            self.directory = Some(directory.clone());
            return false;
        }
        self.directory = directory;
        self.refresh_task = None;
        self.refreshing = false;
        self.loaded = false;
        self.load_error = None;
        self.notice = None;
        self.selected = None;
        self.checked.clear();
        self.apply_status(None, Status::default(), HashSet::new(), cx);
        self.refresh(cx);
        true
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        let repository = self.repository.clone();
        let executor = cx.background_executor().clone();
        self.refreshing = true;
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            let result = executor
                .spawn(async move {
                    let repository = match repository {
                        Some(repository) => repository,
                        None => match Repository::discover(&directory)? {
                            Some(repository) => repository,
                            None => return Ok(None),
                        },
                    };
                    let status = repository.status()?;
                    let ignored_paths = repository.ignored_paths()?;
                    io::Result::Ok(Some((repository, status, ignored_paths)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.refreshing = false;
                this.loaded = true;
                this.load_error = None;
                match result {
                    Ok(Some((repository, status, ignored_paths))) => {
                        this.apply_status(Some(repository), status, ignored_paths, cx)
                    }
                    Ok(None) => this.apply_status(None, Status::default(), HashSet::new(), cx),
                    Err(error) => {
                        this.apply_status(None, Status::default(), HashSet::new(), cx);
                        this.load_error = Some(error.to_string());
                    }
                }
            });
        }));
    }

    fn apply_status(
        &mut self,
        repository: Option<Repository>,
        status: Status,
        ignored_paths: HashSet<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.tree_changes = repository
            .as_ref()
            .map(|repository| status.tree_changes(repository.root()))
            .unwrap_or_default();
        self.repository = repository;
        self.status = status;
        self.ignored_paths = ignored_paths;
        self.rows.clear();
        for staged in [true, false] {
            let files: Vec<_> = (0..self.status.files.len())
                .filter(|&ix| side(&self.status.files[ix], staged).is_some())
                .collect();
            if staged {
                self.staged_count = files.len();
            } else {
                self.unstaged_count = files.len();
            }
            if !files.is_empty() {
                self.rows.push(Row::Header { staged });
                self.rows
                    .extend(files.into_iter().map(|ix| Row::File { ix, staged }));
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|selection| self.file(selection).is_none())
        {
            self.selected = None;
        }
        let available: HashSet<_> = self
            .rows
            .iter()
            .filter_map(|row| match *row {
                Row::File { ix, staged } => Some(Selection {
                    relative: self.status.files[ix].relative.clone(),
                    staged,
                }),
                Row::Header { .. } => None,
            })
            .collect();
        self.checked
            .retain(|selection| available.contains(selection));
        self.load_diff(cx);
        cx.notify();
    }

    fn file(&self, selection: &Selection) -> Option<&FileStatus> {
        self.status.files.iter().find(|file| {
            file.relative == selection.relative && side(file, selection.staged).is_some()
        })
    }

    fn files(&self, staged: bool) -> Vec<FileStatus> {
        self.status
            .files
            .iter()
            .filter(|file| side(file, staged).is_some())
            .cloned()
            .collect()
    }

    fn checked_files(&self, staged: Option<bool>) -> Vec<FileStatus> {
        self.status
            .files
            .iter()
            .filter(|file| {
                self.checked.iter().any(|selection| {
                    selection.relative == file.relative
                        && staged.is_none_or(|staged| selection.staged == staged)
                })
            })
            .cloned()
            .collect()
    }

    fn checked_count(&self, staged: Option<bool>) -> usize {
        self.checked_files(staged).len()
    }

    fn set_checked(&mut self, selection: Selection, checked: bool, cx: &mut Context<Self>) {
        if checked {
            self.checked.insert(selection);
        } else {
            self.checked.remove(&selection);
        }
        cx.notify();
    }

    pub(crate) fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selected = Some(selection);
        self.load_diff(cx);
        cx.emit(GitPanelEvent::ShowDiff);
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.load_diff(cx);
        cx.notify();
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let file = self
            .selected
            .as_ref()
            .and_then(|selection| self.file(selection));
        match (
            self.repository.clone(),
            self.selected.clone(),
            file.cloned(),
        ) {
            (Some(repository), Some(selection), Some(file)) => self
                .diff_view
                .update(cx, |view, cx| view.load(repository, selection, file, cx)),
            _ => self.diff_view.update(cx, |view, cx| view.clear(cx)),
        }
    }

    fn run<T: Send + 'static>(
        &mut self,
        operation: impl FnOnce() -> io::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = executor.spawn(async move { operation() }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => this.notice = Some(Notice::Error(error.to_string())),
                }
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn move_files(
        &mut self,
        files: Vec<FileStatus>,
        staged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let moved: Vec<_> = files.iter().map(|file| file.relative.clone()).collect();
        self.run(
            move || {
                if staged {
                    repository.unstage(&files)
                } else {
                    repository.stage(&files)
                }
            },
            move |this, (), _, _| {
                if let Some(selection) = &mut this.selected
                    && selection.staged == staged
                    && moved.contains(&selection.relative)
                {
                    selection.staged = !staged;
                }
                this.checked = this
                    .checked
                    .drain()
                    .map(|mut selection| {
                        if selection.staged == staged && moved.contains(&selection.relative) {
                            selection.staged = !staged;
                        }
                        selection
                    })
                    .collect();
            },
            window,
            cx,
        );
    }

    fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let files = self.checked_files(None);
        let message = self.commit_message.read(cx).value().trim_end().to_string();
        if files.is_empty() {
            self.notice = Some(Notice::Error("Select changes to commit them".into()));
        } else if message.trim().is_empty() {
            self.notice = Some(Notice::Error("Write a commit message first".into()));
        } else {
            self.run(
                move || {
                    repository.stage(&files)?;
                    repository.commit_files(&files, &message)
                },
                |this, summary, window, cx| {
                    this.notice = Some(Notice::Info(summary));
                    this.commit_message
                        .update(cx, |input, cx| input.set_value("", window, cx));
                },
                window,
                cx,
            );
        }
        cx.notify();
    }

    fn unstage_checked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files = self.checked_files(Some(true));
        if files.is_empty() {
            self.notice = Some(Notice::Info("Select staged changes to unstage".into()));
            cx.notify();
        } else {
            self.move_files(files, true, window, cx);
        }
    }

    fn revert_checked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let files = self.checked_files(Some(false));
        if files.is_empty() {
            self.notice = Some(Notice::Info("Select unstaged changes to revert".into()));
            cx.notify();
            return;
        }
        self.run(
            move || repository.revert(&files),
            |_, (), _, _| {},
            window,
            cx,
        );
    }

    pub fn stage_all(&mut self, _: &StageAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.repository.is_none() {
            cx.emit(GitPanelEvent::Status(NO_REPOSITORY.into()));
        } else if self.unstaged_count == 0 {
            self.notice = Some(Notice::Info("No changes to stage".into()));
            cx.notify();
        } else {
            let files = self.files(false);
            self.move_files(files, false, window, cx);
        }
    }

    pub fn pull(&mut self, _: &Pull, window: &mut Window, cx: &mut Context<Self>) {
        self.sync(Remote::Pull, window, cx);
    }

    pub fn push(&mut self, _: &Push, window: &mut Window, cx: &mut Context<Self>) {
        self.sync(Remote::Push, window, cx);
    }

    pub fn fetch(&mut self, _: &Fetch, window: &mut Window, cx: &mut Context<Self>) {
        self.sync(Remote::Fetch, window, cx);
    }

    fn sync(&mut self, remote: Remote, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            cx.emit(GitPanelEvent::Status(NO_REPOSITORY.into()));
            return;
        };
        if self.busy {
            return;
        }
        self.run(
            move || Ok(remote.run(&repository)),
            move |this, result, _, cx| {
                let status = match result {
                    Ok(summary) => {
                        this.notice = Some(Notice::Info(summary.clone()));
                        summary
                    }
                    Err(error) => {
                        this.notice = Some(Notice::Error(error.to_string()));
                        format!("{} failed: {error}", remote.name())
                    }
                };
                cx.emit(GitPanelEvent::Status(status));
            },
            window,
            cx,
        );
        self.notice = Some(Notice::Info(remote.progress().into()));
        cx.emit(GitPanelEvent::Status(remote.progress().into()));
    }

    fn init_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        self.run(
            move || Repository::init(&directory),
            |this, repository, _, _| {
                this.notice = Some(Notice::Info(format!(
                    "Created a Git repository in {}",
                    repository.root().display()
                )));
                this.repository = Some(repository);
            },
            window,
            cx,
        );
    }

    fn open_file(&mut self, relative: &str, cx: &mut Context<Self>) {
        if let Some(repository) = &self.repository {
            cx.emit(GitPanelEvent::OpenFile(git::absolute(
                repository.root(),
                relative,
            )));
        }
    }

    fn render_changes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_count = self.checked_count(None);
        let selected_staged = self.checked_count(Some(true));
        let selected_unstaged = self.checked_count(Some(false));
        let can_commit = !self.busy && selected_count > 0;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .key_context(COMMIT_CONTEXT)
                    .on_action(cx.listener(Self::commit))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_3()
                    .pb_3()
                    .child(
                        div()
                            .debug_selector(|| "git-commit-message".into())
                            .min_w_0()
                            .child(Input::new(&self.commit_message).small()),
                    )
                    .child(
                        Button::new("git-commit")
                            .primary()
                            .small()
                            .w_full()
                            .icon(IconName::Check)
                            .label(if selected_count > 0 {
                                format!("Commit {selected_count} selected")
                            } else {
                                "Commit".into()
                            })
                            .tooltip(if cfg!(target_os = "macos") {
                                "Commit selected changes (Cmd+Enter)"
                            } else {
                                "Commit selected changes (Ctrl+Enter)"
                            })
                            .disabled(!can_commit)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit(&Commit, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("git-unstage-selected")
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .icon(IconName::Minus)
                                    .label(format!("Unstage ({selected_staged})"))
                                    .disabled(self.busy || selected_staged == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.unstage_checked(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("git-revert-selected")
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .icon(IconName::Close)
                                    .label(format!("Revert ({selected_unstaged})"))
                                    .disabled(self.busy || selected_unstaged == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.revert_checked(window, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(self.notice.as_ref(), |column, notice| {
                let (message, color) = match notice {
                    Notice::Info(message) => (message, theme::muted()),
                    Notice::Error(message) => (message, theme::error()),
                };
                column.child(
                    div()
                        .flex_shrink_0()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(color)
                        .child(message.clone()),
                )
            })
            .child(if self.rows.is_empty() {
                ui::empty_state(Icon::new(IconName::CircleCheck).size(px(24.)), "No changes")
                    .pt_4()
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "git-changes",
                            self.rows.len(),
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                            }),
                        )
                        .size_full()
                        .px_1p5()
                        .track_scroll(self.list_scroll.clone()),
                    )
                    .into_any_element()
            })
    }

    fn render_actions_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let commit_input = self.commit_message.read(cx).focus_handle(cx);
        let button = Button::new("git-actions")
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .tooltip("More Actions")
            .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, cx| {
                let Some(this) = panel.upgrade() else {
                    return menu;
                };
                let this = this.read(cx);
                let (busy, selected, unstaged) =
                    (this.busy, this.checked_count(None), this.unstaged_count);
                let item =
                    |icon: Icon,
                     label: &'static str,
                     action: &dyn Action,
                     run: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                        menu_item(&panel, icon, label, action, run, cx)
                    };
                menu.min_w(px(220.))
                    .action_context(commit_input.clone())
                    .item(
                        item(
                            Icon::new(IconName::Check),
                            "Commit",
                            &Commit,
                            |this, window, cx| this.commit(&Commit, window, cx),
                        )
                        .disabled(busy || selected == 0),
                    )
                    .item(
                        item(
                            Icon::new(IconName::Plus),
                            "Stage All Changes",
                            &StageAll,
                            |this, window, cx| this.stage_all(&StageAll, window, cx),
                        )
                        .disabled(busy || unstaged == 0),
                    )
                    .separator()
                    .item(
                        item(
                            Icon::new(IconName::ArrowDown),
                            "Pull",
                            &Pull,
                            |this, window, cx| this.pull(&Pull, window, cx),
                        )
                        .disabled(busy),
                    )
                    .item(
                        item(
                            Icon::new(IconName::ArrowUp),
                            "Push",
                            &Push,
                            |this, window, cx| this.push(&Push, window, cx),
                        )
                        .disabled(busy),
                    )
                    .item(
                        item(
                            Icon::new(AppIcon::CloudDownload),
                            "Fetch",
                            &Fetch,
                            |this, window, cx| this.fetch(&Fetch, window, cx),
                        )
                        .disabled(busy),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Keyboard Shortcuts…")
                            .icon(IconName::Settings)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(EditSettings), cx)
                            }),
                    )
            });
        div()
            .debug_selector(|| "git-actions".into())
            .flex_shrink_0()
            .child(button)
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = div()
            .id(ix)
            .group(ROW_GROUP)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .gap_2()
            .pr_1()
            .rounded_md();
        let on_hover = |button: Button, shown: bool| {
            div()
                .flex_shrink_0()
                .when(!shown, |slot| {
                    slot.invisible()
                        .group_hover(ROW_GROUP, |style| style.visible())
                })
                .child(button)
        };
        match self.rows[ix] {
            Row::Header { staged } => {
                let (title, count, icon, tooltip) = if staged {
                    (
                        "STAGED CHANGES",
                        self.staged_count,
                        IconName::Minus,
                        "Unstage all",
                    )
                } else {
                    ("CHANGES", self.unstaged_count, IconName::Plus, "Stage all")
                };
                row.pl_2()
                    .child(ui::caption(title))
                    .child(ui::count_badge(count))
                    .child(div().flex_1())
                    .child(on_hover(
                        Button::new(("git-move-all", ix))
                            .ghost()
                            .xsmall()
                            .icon(icon)
                            .tooltip(tooltip)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let files = this.files(staged);
                                this.move_files(files, staged, window, cx);
                            })),
                        false,
                    ))
            }
            Row::File {
                ix: file_ix,
                staged,
            } => {
                let file = &self.status.files[file_ix];
                let change = side(file, staged).unwrap_or(Change::Modified);
                let selection = Selection {
                    relative: file.relative.clone(),
                    staged,
                };
                let selected = self.selected.as_ref() == Some(&selection);
                let checked = self.checked.contains(&selection);
                let (folder, name) = match file.relative.rsplit_once('/') {
                    Some((folder, name)) => (folder, name),
                    None => ("", file.relative.as_str()),
                };
                let detail = match &file.original {
                    Some(original) => format!("← {original}"),
                    None => folder.to_string(),
                };
                let (icon, tooltip) = if staged {
                    (IconName::Minus, "Unstage")
                } else {
                    (IconName::Plus, "Stage")
                };
                let relative = file.relative.clone();
                let moved = file.clone();
                let color = theme::git_change(change);
                let file_icon = match change {
                    Change::Added | Change::Untracked => IconName::Plus,
                    Change::Deleted => IconName::Close,
                    _ => IconName::File,
                };
                let checkbox_panel = cx.entity().downgrade();
                let checkbox_selection = selection.clone();
                row.debug_selector(move || format!("git-row-{ix}"))
                    .pl_3()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::hover()))
                    .when(selected, |row| row.bg(theme::active_row()))
                    .child(
                        Checkbox::new(("git-check", ix))
                            .xsmall()
                            .checked(checked)
                            .disabled(self.busy)
                            .on_click(move |checked, _, cx| {
                                let _ = checkbox_panel.update(cx, |panel, cx| {
                                    panel.set_checked(checkbox_selection.clone(), *checked, cx)
                                });
                            }),
                    )
                    .child(
                        Icon::new(file_icon)
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(theme::subtle()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(color)
                            .when(change == Change::Deleted, |name| name.line_through())
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme::subtle())
                            .child(detail),
                    )
                    .when(change != Change::Deleted, |row| {
                        row.child(on_hover(
                            Button::new(("git-open", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ExternalLink)
                                .tooltip("Open file")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_file(&relative, cx);
                                })),
                            selected,
                        ))
                    })
                    .child(on_hover(
                        Button::new(("git-move", ix))
                            .ghost()
                            .xsmall()
                            .icon(icon)
                            .tooltip(tooltip)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.move_files(vec![moved.clone()], staged, window, cx);
                            })),
                        selected,
                    ))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_shrink_0()
                            .text_center()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(change.letter()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(selection.clone(), cx)))
            }
        }
    }
}

fn menu_item(
    panel: &WeakEntity<GitPanel>,
    icon: Icon,
    label: &'static str,
    action: &dyn Action,
    run: fn(&mut GitPanel, &mut Window, &mut Context<GitPanel>),
    cx: &App,
) -> PopupMenuItem {
    let shortcut = commands::shortcut(action, cx);
    let panel = panel.clone();
    PopupMenuItem::element(move |_, _| {
        div()
            .debug_selector(move || format!("git-menu-{label}"))
            .flex_1()
            .flex()
            .items_center()
            .justify_between()
            .gap_6()
            .child(label)
            .children(
                shortcut
                    .clone()
                    .map(|keys| div().text_xs().text_color(theme::subtle()).child(keys)),
            )
    })
    .icon(icon)
    .on_click(move |_, window, cx| {
        let _ = panel.update(cx, |panel, cx| run(panel, window, cx));
    })
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = |text: String| div().px_4().py_2().text_color(theme::muted()).child(text);
        let body = if self.directory.is_none() {
            ui::empty_state(
                Icon::new(AppIcon::GitBranch).size(px(28.)),
                "Open a folder to see its Git changes.",
            )
            .child(
                div().pt_2().child(
                    Button::new("git-clone")
                        .small()
                        .label("Clone Repository…")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(CloneRepository), cx)
                        }),
                ),
            )
            .into_any_element()
        } else if let Some(error) = &self.load_error {
            div()
                .px_4()
                .py_2()
                .text_color(theme::error())
                .child(format!("Could not read the Git repository: {error}"))
                .into_any_element()
        } else if !self.loaded {
            message("Loading…".into()).into_any_element()
        } else if self.repository.is_none() {
            ui::empty_state(
                Icon::new(AppIcon::GitBranch).size(px(28.)),
                "This folder is not in a Git repository.",
            )
            .child(
                div().pt_2().child(
                    Button::new("git-init")
                        .primary()
                        .small()
                        .label("Initialize Repository")
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.init_repository(window, cx)),
                        ),
                ),
            )
            .when_some(self.notice.as_ref(), |view, notice| {
                let (Notice::Info(text) | Notice::Error(text)) = notice;
                view.child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(text.clone()),
                )
            })
            .into_any_element()
        } else {
            self.render_changes(cx).into_any_element()
        };
        let has_repository = self.repository.is_some();
        let actions = has_repository.then(|| self.render_actions_menu(cx));
        div()
            .debug_selector(|| "git-panel".into())
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                ui::panel_header("SOURCE CONTROL").when(has_repository, |header| {
                    header
                        .child(
                            div()
                                .debug_selector(|| "git-branch".into())
                                .max_w(px(140.))
                                .flex()
                                .flex_shrink()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .px_1p5()
                                .h(px(20.))
                                .rounded_sm()
                                .bg(theme::accent_wash())
                                .text_xs()
                                .text_color(theme::accent())
                                .child(Icon::new(AppIcon::GitBranch).size(px(12.)).flex_shrink_0())
                                .child(
                                    div().min_w_0().truncate().child(self.status.branch.label()),
                                ),
                        )
                        .children(actions)
                }),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

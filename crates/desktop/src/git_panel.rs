use std::{
    collections::HashMap,
    io,
    ops::Range,
    path::{Path, PathBuf},
};

use gpui::{
    Context, Div, Entity, EventEmitter, FontWeight, IntoElement, Render, Stateful, Task,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
};
use ide_core::git::{self, Change, FileStatus, Repository, Status};

use crate::{
    assets::AppIcon,
    diff_view::DiffView,
    theme,
    ui::{self, ROW_GROUP},
};

#[cfg(test)]
#[path = "git_panel_tests.rs"]
pub(crate) mod tests;

actions!(git, [Commit]);

pub const COMMIT_CONTEXT: &str = "GitCommit";

const ROW_HEIGHT: f32 = 24.;

pub enum GitPanelEvent {
    OpenFile(PathBuf),
    ShowDiff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    rows: Vec<Row>,
    staged_count: usize,
    unstaged_count: usize,
    selected: Option<Selection>,
    diff_view: Entity<DiffView>,
    notice: Option<Notice>,
    busy: bool,
    commit_message: Entity<InputState>,
    list_scroll: UniformListScrollHandle,
    refresh_task: Option<Task<()>>,
}

impl EventEmitter<GitPanelEvent> for GitPanel {}

fn side(file: &FileStatus, staged: bool) -> Option<Change> {
    if staged { file.staged } else { file.unstaged }
}

impl GitPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            directory: None,
            repository: None,
            status: Status::default(),
            loaded: false,
            load_error: None,
            tree_changes: HashMap::new(),
            rows: Vec::new(),
            staged_count: 0,
            unstaged_count: 0,
            selected: None,
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
        self.loaded = false;
        self.load_error = None;
        self.notice = None;
        self.selected = None;
        self.apply_status(None, Status::default(), cx);
        self.refresh(cx);
        true
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        let repository = self.repository.clone();
        let executor = cx.background_executor().clone();
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
                    io::Result::Ok(Some((repository, status)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loaded = true;
                this.load_error = None;
                match result {
                    Ok(Some((repository, status))) => {
                        this.apply_status(Some(repository), status, cx)
                    }
                    Ok(None) => this.apply_status(None, Status::default(), cx),
                    Err(error) => {
                        this.apply_status(None, Status::default(), cx);
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
        cx: &mut Context<Self>,
    ) {
        self.tree_changes = repository
            .as_ref()
            .map(|repository| status.tree_changes(repository.root()))
            .unwrap_or_default();
        self.repository = repository;
        self.status = status;
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
            },
            window,
            cx,
        );
    }

    fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let message = self.commit_message.read(cx).value().trim_end().to_string();
        if self.staged_count == 0 {
            self.notice = Some(Notice::Error("Stage changes to commit them".into()));
        } else if message.trim().is_empty() {
            self.notice = Some(Notice::Error("Write a commit message first".into()));
        } else {
            self.run(
                move || repository.commit(&message),
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
        let can_commit = !self.busy && self.staged_count > 0;
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
                            .label(if self.staged_count > 0 {
                                format!("Commit {} staged", self.staged_count)
                            } else {
                                "Commit".into()
                            })
                            .tooltip(if cfg!(target_os = "macos") {
                                "Commit staged changes (Cmd+Enter)"
                            } else {
                                "Commit staged changes (Ctrl+Enter)"
                            })
                            .disabled(!can_commit)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit(&Commit, window, cx)),
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
                row.debug_selector(move || format!("git-row-{ix}"))
                    .pl_3()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::hover()))
                    .when(selected, |row| row.bg(theme::active_row()))
                    .child(
                        Icon::new(IconName::File)
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

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = |text: String| div().px_4().py_2().text_color(theme::muted()).child(text);
        let body = if self.directory.is_none() {
            ui::empty_state(
                Icon::new(AppIcon::GitBranch).size(px(28.)),
                "Open a folder to see its Git changes.",
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
                        .child(
                            Button::new("git-refresh")
                                .ghost()
                                .xsmall()
                                .icon(AppIcon::Refresh)
                                .tooltip("Refresh")
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        )
                }),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

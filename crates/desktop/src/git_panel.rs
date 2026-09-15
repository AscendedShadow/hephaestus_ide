use std::{
    collections::HashMap,
    io,
    ops::Range,
    path::{Path, PathBuf},
};

use gpui::{
    Context, Div, Entity, EventEmitter, IntoElement, ListHorizontalSizingBehavior, Render,
    SharedString, Stateful, Task, UniformListScrollHandle, Window, actions, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{
    Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    resizable::{ResizableState, resizable_panel, v_resizable},
};
use ide_core::git::{self, Change, Diff, DiffLine, FileStatus, LineKind, Repository, Status};

use crate::theme;

#[cfg(test)]
#[path = "git_panel_tests.rs"]
pub(crate) mod tests;

actions!(git, [Commit]);

pub const COMMIT_CONTEXT: &str = "GitCommit";

const ROW_HEIGHT: f32 = 24.;
const DIFF_ROW_HEIGHT: f32 = 20.;

pub enum GitPanelEvent {
    OpenFile(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    relative: String,
    staged: bool,
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
    diff: Option<(Selection, Result<Diff, String>)>,
    widest_line: usize,
    notice: Option<Notice>,
    busy: bool,
    commit_message: Entity<InputState>,
    split: Entity<ResizableState>,
    list_scroll: UniformListScrollHandle,
    diff_scroll: UniformListScrollHandle,
    refresh_task: Option<Task<()>>,
    diff_task: Option<Task<()>>,
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
            diff: None,
            widest_line: 0,
            notice: None,
            busy: false,
            commit_message: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(1, 6)
                    .placeholder("Commit message")
            }),
            split: cx.new(|_| ResizableState::default()),
            list_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            refresh_task: None,
            diff_task: None,
        }
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

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selected = Some(selection);
        self.load_diff(cx);
        cx.notify();
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let file = self
            .selected
            .as_ref()
            .and_then(|selection| self.file(selection));
        let (Some(repository), Some(selection), Some(file)) = (
            self.repository.clone(),
            self.selected.clone(),
            file.cloned(),
        ) else {
            self.diff_task = None;
            self.diff = None;
            return;
        };
        let executor = cx.background_executor().clone();
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            let staged = selection.staged;
            let diff = executor
                .spawn(async move { repository.diff(&file, staged) })
                .await
                .map_err(|error| error.to_string());
            let _ = this.update(cx, |this, cx| {
                let widest = diff.as_ref().ok().and_then(|diff| {
                    (0..diff.lines.len()).max_by_key(|&ix| diff.lines[ix].text.chars().count())
                });
                if this
                    .diff
                    .as_ref()
                    .is_none_or(|(shown, _)| *shown != selection)
                {
                    this.diff_scroll = UniformListScrollHandle::new();
                }
                this.widest_line = widest.unwrap_or(0);
                this.diff = Some((selection, diff));
                cx.notify();
            });
        }));
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
                    .h(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .child(
                        div()
                            .debug_selector(|| "git-branch".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::accent())
                            .child(self.status.branch.label()),
                    )
                    .child(
                        Button::new("git-refresh")
                            .ghost()
                            .xsmall()
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .key_context(COMMIT_CONTEXT)
                    .on_action(cx.listener(Self::commit))
                    .flex_shrink_0()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_2()
                    .pb_2()
                    .child(
                        div()
                            .debug_selector(|| "git-commit-message".into())
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.commit_message).small()),
                    )
                    .child(
                        Button::new("git-commit")
                            .primary()
                            .small()
                            .label("Commit")
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
                        .px_2()
                        .pb_1()
                        .text_color(color)
                        .child(message.clone()),
                )
            })
            .child(if self.rows.is_empty() {
                div()
                    .px_2()
                    .text_color(theme::muted())
                    .child("No changes")
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
                        .track_scroll(self.list_scroll.clone()),
                    )
                    .into_any_element()
            })
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = div()
            .id(ix)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .gap_2()
            .pr_2();
        match self.rows[ix] {
            Row::Header { staged } => {
                let (title, count, icon, tooltip) = if staged {
                    (
                        "Staged Changes",
                        self.staged_count,
                        IconName::Minus,
                        "Unstage all",
                    )
                } else {
                    ("Changes", self.unstaged_count, IconName::Plus, "Stage all")
                };
                row.pl_2()
                    .text_color(theme::muted())
                    .child(div().flex_1().child(format!("{title} ({count})")))
                    .child(
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
                    )
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
                row.pl(px(20.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::border()))
                    .when(selected, |row| row.bg(theme::border()))
                    .child(
                        div()
                            .w(px(12.))
                            .flex_shrink_0()
                            .text_color(theme::git_change(change))
                            .child(change.letter()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::git_change(change))
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::muted())
                            .child(detail),
                    )
                    .when(change != Change::Deleted, |row| {
                        row.child(
                            Button::new(("git-open", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ExternalLink)
                                .tooltip("Open file")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_file(&relative, cx);
                                })),
                        )
                    })
                    .child(
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
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(selection.clone(), cx)))
            }
        }
    }

    fn render_diff(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.selected.as_ref().map(|selection| {
            let side = if selection.staged {
                "Staged"
            } else {
                "Unstaged"
            };
            format!("{} — {side}", selection.relative)
        });
        let shown = match (&self.selected, &self.diff) {
            (Some(selected), Some((shown, diff))) if selected == shown => Some(diff),
            _ => None,
        };
        let message = |text: SharedString| {
            div()
                .p_3()
                .text_color(theme::muted())
                .child(text)
                .into_any_element()
        };
        let body = match shown {
            None if self.selected.is_none() => message("Select a file to see its changes.".into()),
            None => message("Loading diff…".into()),
            Some(Err(error)) => div()
                .p_3()
                .text_color(theme::error())
                .child(error.clone())
                .into_any_element(),
            Some(Ok(diff)) if diff.lines.is_empty() => message("No changes to show.".into()),
            Some(Ok(diff)) => div()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "git-diff",
                        diff.lines.len(),
                        cx.processor(|this, range: Range<usize>, _, _| {
                            let Some((_, Ok(diff))) = &this.diff else {
                                return Vec::new();
                            };
                            range
                                .filter_map(|ix| diff.lines.get(ix))
                                .map(render_diff_line)
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(self.widest_line))
                    .track_scroll(self.diff_scroll.clone()),
                )
                .into_any_element(),
        };
        div()
            .debug_selector(|| "git-diff".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .when_some(title, |view, title| {
                view.child(
                    div()
                        .h(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .px_3()
                        .border_b_1()
                        .border_color(theme::border())
                        .text_color(theme::muted())
                        .child(div().min_w_0().truncate().child(title)),
                )
            })
            .child(body)
    }
}

fn render_diff_line(line: &DiffLine) -> Div {
    let (background, sign, color) = match line.kind {
        LineKind::Added => (Some(theme::diff_added()), "+", theme::text()),
        LineKind::Removed => (Some(theme::diff_removed()), "-", theme::text()),
        LineKind::Hunk => (Some(theme::panel()), "", theme::accent()),
        LineKind::Context => (None, "", theme::text()),
        LineKind::Meta | LineKind::Note => (None, "", theme::muted()),
    };
    let number = |number: Option<u32>| {
        div()
            .w(px(44.))
            .flex_shrink_0()
            .pr_2()
            .text_right()
            .text_color(theme::muted())
            .child(number.map(|number| number.to_string()).unwrap_or_default())
    };
    div()
        .w_full()
        .h(px(DIFF_ROW_HEIGHT))
        .flex()
        .items_center()
        .whitespace_nowrap()
        .font(theme::monospace_font())
        .text_size(px(13.))
        .when_some(background, |row, background| row.bg(background))
        .child(number(line.old_line))
        .child(number(line.new_line))
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(color)
                .child(sign),
        )
        .child(div().pr_4().text_color(color).child(line.text.clone()))
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = |text: String| div().p_3().text_color(theme::muted()).child(text);
        let body = if self.directory.is_none() {
            message("Open a folder to see its Git changes.".into()).into_any_element()
        } else if let Some(error) = &self.load_error {
            div()
                .p_3()
                .text_color(theme::error())
                .child(format!("Could not read the Git repository: {error}"))
                .into_any_element()
        } else if !self.loaded {
            message("Loading…".into()).into_any_element()
        } else if self.repository.is_none() {
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .child(message("This folder is not in a Git repository.".into()))
                .child(
                    div().px_3().child(
                        Button::new("git-init")
                            .label("Initialize Repository")
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.init_repository(window, cx)),
                            ),
                    ),
                )
                .when_some(self.notice.as_ref(), |view, notice| {
                    let (Notice::Info(text) | Notice::Error(text)) = notice;
                    view.child(message(text.clone()))
                })
                .into_any_element()
        } else {
            v_resizable("git-split")
                .with_state(&self.split)
                .child(
                    resizable_panel()
                        .size(px(320.))
                        .size_range(px(160.)..px(640.))
                        .child(self.render_changes(cx)),
                )
                .child(resizable_panel().child(self.render_diff(cx)))
                .into_any_element()
        };
        div()
            .debug_selector(|| "git-panel".into())
            .size_full()
            .overflow_hidden()
            .bg(theme::panel())
            .child(body)
    }
}

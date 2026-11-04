//! Branches (IDE-026), history and blame (IDE-027) and hunk staging (IDE-028).
//!
//! Git operations run in the background and only one runs at a time. Historical content
//! and diffs open as new, unsaved tabs, so the worktree and dirty buffers are never
//! touched.

use ide_core::{
    git::FileStatus,
    git_ops::{ChangedFile, Commit, Hunk},
};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;

const PAGE: usize = 200;

#[derive(Clone, Debug)]
pub(super) enum GitAction {
    CreateBranch,
    Switch(String),
    CheckoutRemote(String),
    LoadMoreHistory,
    Commit(usize),
    CommitFile {
        commit: usize,
        parent: usize,
        file: ChangedFile,
    },
    FileAtRevision {
        commit: usize,
        path: String,
    },
    StageHunk(usize),
}

#[derive(Default)]
pub(super) struct GitState {
    pub busy: bool,
    history: Vec<Commit>,
    history_path: Option<String>,
    hunks: Vec<(FileStatus, bool, Hunk)>,
    pending_switch: Option<String>,
}

fn relative_time(time: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(time);
    let seconds = (now - time).max(0);
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        86_400..2_592_000 => format!("{} days ago", seconds / 86_400),
        _ => format!("{} months ago", seconds / 2_592_000),
    }
}

impl IdeShell {
    fn repository(&mut self, cx: &mut Context<Self>) -> Option<Repository> {
        let repository = self.git.read(cx).repository_handle();
        if repository.is_none() {
            self.status = "Open a folder inside a Git repository".into();
            cx.notify();
        }
        repository
    }

    /// Run a Git operation in the background (one at a time), then `done`.
    fn git_job<T: Send + 'static>(
        &mut self,
        label: &str,
        job: impl FnOnce() -> std::io::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_state.busy {
            self.status = "Another Git operation is still running".into();
            cx.notify();
            return;
        }
        self.git_state.busy = true;
        self.status = format!("{label}…");
        cx.notify();
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.git_state.busy = false;
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => this.status = format!("Git: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_git_action(
        &mut self,
        action: GitAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            GitAction::CreateBranch => self.prompt_create_branch(window, cx),
            GitAction::Switch(name) => self.request_switch(name, window, cx),
            GitAction::CheckoutRemote(remote) => self.checkout_remote(remote, window, cx),
            GitAction::LoadMoreHistory => self.load_history(true, window, cx),
            GitAction::Commit(ix) => self.show_commit(ix, window, cx),
            GitAction::CommitFile {
                commit,
                parent,
                file,
            } => self.show_commit_file(commit, parent, file, window, cx),
            GitAction::FileAtRevision { commit, path } => {
                self.show_file_at_revision(commit, path, window, cx)
            }
            GitAction::StageHunk(ix) => self.apply_hunk(ix, window, cx),
        }
    }

    // ----- branches -----

    pub(super) fn switch_branch(
        &mut self,
        _: &SwitchBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository(cx) else {
            return;
        };
        self.git_job(
            "Listing branches",
            move || repository.branches(),
            |this, branches, window, cx| {
                let mut items = vec![PickerItem {
                    label: "+ Create Branch…".into(),
                    detail: "from the current commit or another ref".into(),
                    filter: "create new branch".into(),
                    depth: 0,
                    action: PickerAction::Git(GitAction::CreateBranch),
                }];
                for branch in &branches {
                    let mut detail = String::new();
                    if branch.current {
                        detail.push_str("current · ");
                    }
                    if let Some(upstream) = &branch.upstream {
                        detail.push_str(&format!("{upstream} {} · ", branch.track));
                    }
                    detail.push_str(&format!("{} {}", branch.commit, branch.subject));
                    let action = match (branch.remote, branch.current) {
                        (_, true) => PickerAction::Nothing,
                        (true, _) => {
                            PickerAction::Git(GitAction::CheckoutRemote(branch.name.clone()))
                        }
                        (false, _) => PickerAction::Git(GitAction::Switch(branch.name.clone())),
                    };
                    items.push(PickerItem {
                        label: if branch.remote {
                            format!("{}  (remote — check out with tracking)", branch.name)
                        } else {
                            branch.name.clone()
                        },
                        detail,
                        filter: branch.name.clone(),
                        depth: 0,
                        action,
                    });
                }
                this.show_picker(
                    PickerKind::Locations,
                    "Switch branch".into(),
                    items,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    fn prompt_create_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let field = |placeholder: &'static str,
                     value: &str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
                    .validate(|text, _| !text.contains('\n'))
            })
        };
        let name = field("Branch name", "", window, cx);
        let from = field("Start point (branch, tag or commit)", "HEAD", window, cx);
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, {
            let (name, from) = (name.clone(), from.clone());
            move |dialog, _, _| {
                let (shell, name, from) = (shell.clone(), name.clone(), from.clone());
                dialog
                    .title("Create Branch")
                    .w(px(420.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(Input::new(&name))
                            .child(Input::new(&from)),
                    )
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Create and Switch"))
                    .on_ok(move |_, window, cx| {
                        let name = name.read(cx).value().trim().to_string();
                        let from = from.read(cx).value().trim().to_string();
                        shell
                            .update(cx, |shell, cx| shell.create_branch(name, from, window, cx))
                            .unwrap_or(true)
                    })
            }
        });
        name.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn create_branch(
        &mut self,
        name: String,
        from: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(repository) = self.repository(cx) else {
            return true;
        };
        if let Err(error) = repository.check_branch_name(&name) {
            self.status = error;
            cx.notify();
            return false;
        }
        let from = if from.is_empty() {
            "HEAD".to_string()
        } else {
            from
        };
        let label = name.clone();
        self.git_job(
            "Creating branch",
            move || repository.create_branch(&name, &from, true),
            move |this, (), window, cx| this.after_switch(&label, window, cx),
            window,
            cx,
        );
        true
    }

    fn checkout_remote(&mut self, remote: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let local = remote
            .split_once('/')
            .map_or(remote.clone(), |(_, name)| name.to_string());
        let label = local.clone();
        self.git_job(
            "Checking out",
            move || repository.checkout_remote(&remote, &local, true),
            move |this, (), window, cx| this.after_switch(&label, window, cx),
            window,
            cx,
        );
    }

    /// Switching branches with unsaved editors asks first: save everything and switch,
    /// or cancel.
    fn request_switch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let dirty: Vec<String> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.dirty && buffer.document.path().is_some())
            .map(|buffer| buffer.document.name())
            .collect();
        if dirty.is_empty() {
            return self.perform_switch(name, window, cx);
        }
        if window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, name) = (shell.clone(), name.clone());
            dialog
                .title(format!("Switch to {name}"))
                .child(format!(
                    "Unsaved changes in {}. Save them before switching branches?",
                    dirty.join(", ")
                ))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Save All and Switch"))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.git_state.pending_switch = Some(name.clone());
                        shell.save_all(None, window, cx);
                    });
                    true
                })
        });
    }

    /// Continue a switch that waited for Save All (called after saving).
    pub(super) fn resume_pending_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.git_state.pending_switch.take() else {
            return;
        };
        if self
            .buffers
            .iter()
            .any(|buffer| buffer.dirty && buffer.document.path().is_some())
        {
            self.status = format!("Not switching to {name}: some files could not be saved");
            return;
        }
        self.perform_switch(name, window, cx);
    }

    fn perform_switch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let label = name.clone();
        self.git_job(
            "Switching branch",
            move || repository.switch_branch(&name),
            move |this, (), window, cx| this.after_switch(&label, window, cx),
            window,
            cx,
        );
    }

    fn after_switch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_git(cx);
        self.after_tree_change(None, cx);
        self.check_disk_changes(window, cx);
        self.status = format!("Switched to {name}");
    }

    // ----- history -----

    pub(super) fn show_history(
        &mut self,
        _: &ShowHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_state.history_path = None;
        self.load_history(false, window, cx);
    }

    pub(super) fn show_file_history(
        &mut self,
        _: &ShowFileHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let Some(relative) = self
            .document()
            .path()
            .and_then(|path| path.strip_prefix(repository.root()).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
        else {
            self.status = "The active file is not in the repository".into();
            cx.notify();
            return;
        };
        self.git_state.history_path = Some(relative);
        self.load_history(false, window, cx);
    }

    fn load_history(&mut self, more: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository(cx) else {
            return;
        };
        if !more {
            self.git_state.history.clear();
        }
        let skip = self.git_state.history.len();
        let path = self.git_state.history_path.clone();
        self.git_job(
            "Loading history",
            move || repository.log(skip, PAGE, path.as_deref()),
            move |this, commits, window, cx| {
                let complete = commits.len() < PAGE;
                this.git_state.history.extend(commits);
                if this.git_state.history.is_empty() {
                    this.status = "No commits yet".into();
                    return;
                }
                let mut items: Vec<PickerItem> = this
                    .git_state
                    .history
                    .iter()
                    .enumerate()
                    .map(|(ix, commit)| PickerItem {
                        label: format!("{}  {}", commit.short(), commit.summary),
                        detail: format!(
                            "{} · {}{}",
                            commit.author,
                            relative_time(commit.time),
                            if commit.parents.len() > 1 {
                                " · merge"
                            } else {
                                ""
                            }
                        ),
                        filter: format!("{} {} {}", commit.hash, commit.summary, commit.author),
                        depth: 0,
                        action: PickerAction::Git(GitAction::Commit(ix)),
                    })
                    .collect();
                if !complete {
                    items.push(PickerItem {
                        label: "Load more…".into(),
                        detail: String::new(),
                        filter: "load more".into(),
                        depth: 0,
                        action: PickerAction::Git(GitAction::LoadMoreHistory),
                    });
                }
                let title = match &this.git_state.history_path {
                    Some(path) => format!("History of {path}"),
                    None => "Repository history".into(),
                };
                this.show_picker(PickerKind::Locations, title, items, window, cx);
            },
            window,
            cx,
        );
    }

    fn show_commit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(commit) = self.git_state.history.get(ix).cloned() else {
            return;
        };
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let parents = commit.parents.len().max(1);
        self.git_job(
            "Loading commit",
            move || {
                (0..parents)
                    .map(|parent| {
                        repository
                            .changed_files(&commit, parent)
                            .map(|files| (parent, files))
                    })
                    .collect::<std::io::Result<Vec<_>>>()
            },
            move |this, sides, window, cx| {
                let merge = sides.len() > 1;
                let mut items = Vec::new();
                for (parent, files) in sides {
                    if merge {
                        items.push(PickerItem {
                            label: format!("Changes against parent {}", parent + 1),
                            detail: String::new(),
                            filter: String::new(),
                            depth: 0,
                            action: PickerAction::Nothing,
                        });
                    }
                    for file in files {
                        let renamed = file
                            .original
                            .as_ref()
                            .map(|original| format!(" (from {original})"))
                            .unwrap_or_default();
                        items.push(PickerItem {
                            label: format!("{} {}{renamed}", file.status, file.path),
                            detail: "diff".into(),
                            filter: file.path.clone(),
                            depth: usize::from(merge),
                            action: PickerAction::Git(GitAction::CommitFile {
                                commit: ix,
                                parent,
                                file: file.clone(),
                            }),
                        });
                        if file.status != 'D' {
                            items.push(PickerItem {
                                label: format!("   open {} at this commit", file.path),
                                detail: String::new(),
                                filter: file.path.clone(),
                                depth: usize::from(merge),
                                action: PickerAction::Git(GitAction::FileAtRevision {
                                    commit: ix,
                                    path: file.path.clone(),
                                }),
                            });
                        }
                    }
                }
                let title = this
                    .git_state
                    .history
                    .get(ix)
                    .map(|commit| format!("{} {}", commit.short(), commit.summary))
                    .unwrap_or_default();
                this.show_picker(PickerKind::Locations, title, items, window, cx);
            },
            window,
            cx,
        );
    }

    /// Open generated text in a new, clean tab (never written unless the user saves it).
    pub(super) fn open_scratch(
        &mut self,
        title: String,
        text: &str,
        language: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.new_buffer(Document::from_text(text), window, cx);
        self.buffers.push(buffer);
        let ix = self.buffers.len() - 1;
        self.buffers[ix].title_override = Some(title);
        self.set_buffer_language(ix, language, cx);
        self.activate(ix, window, cx);
    }

    fn show_commit_file(
        &mut self,
        ix: usize,
        parent: usize,
        file: ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(commit) = self.git_state.history.get(ix).cloned() else {
            return;
        };
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let title = format!(
            "{} {} (diff vs parent {})",
            commit.short(),
            file.path,
            parent + 1
        );
        self.git_job(
            "Loading diff",
            move || repository.commit_diff(&commit, parent, &file),
            move |this, diff, window, cx| {
                let text: String = diff
                    .lines
                    .iter()
                    .map(|line| {
                        let prefix = match line.kind {
                            ide_core::git::LineKind::Added => "+",
                            ide_core::git::LineKind::Removed => "-",
                            ide_core::git::LineKind::Context => " ",
                            _ => "",
                        };
                        format!("{prefix}{}\n", line.text)
                    })
                    .collect();
                this.open_scratch(title, &text, ide_core::language::PLAIN_TEXT, window, cx);
            },
            window,
            cx,
        );
    }

    fn show_file_at_revision(
        &mut self,
        ix: usize,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(commit) = self.git_state.history.get(ix).cloned() else {
            return;
        };
        let Some(repository) = self.repository(cx) else {
            return;
        };
        let title = format!("{path} @ {}", commit.short());
        let language = syntax::language(Some(Path::new(&path)));
        self.git_job(
            "Loading file",
            move || repository.show_file(&commit.hash, &path),
            move |this, bytes, window, cx| match String::from_utf8(bytes) {
                Ok(text) => this.open_scratch(title, &text, language, window, cx),
                Err(_) => this.status = "That revision is not UTF-8 text".into(),
            },
            window,
            cx,
        );
    }

    // ----- blame -----

    fn blame_relative(&mut self, cx: &mut Context<Self>) -> Option<(Repository, String, String)> {
        let repository = self.repository(cx)?;
        let relative = self
            .document()
            .path()
            .and_then(|path| path.strip_prefix(repository.root()).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        let Some(relative) = relative else {
            self.status = "The active file is not in the repository".into();
            cx.notify();
            return None;
        };
        let contents = self.document().text().to_string();
        Some((repository, relative, contents))
    }

    pub(super) fn blame_line(
        &mut self,
        _: &BlameLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((repository, relative, contents)) = self.blame_relative(cx) else {
            return;
        };
        let (selection, _) = self.source_selection(self.active, cx);
        let line = contents[..selection.start.min(contents.len())]
            .matches('\n')
            .count()
            + 1;
        self.git_job(
            "Blaming",
            move || repository.blame(&relative, &contents),
            move |this, lines, _, cx| {
                this.status = match lines.iter().find(|blame| blame.line == line) {
                    Some(blame) if blame.uncommitted => format!("Line {line}: not committed yet"),
                    Some(blame) => format!(
                        "Line {line}: {} {} · {} — {}",
                        &blame.commit[..8.min(blame.commit.len())],
                        blame.author,
                        relative_time(blame.time),
                        blame.summary
                    ),
                    None => format!("No blame information for line {line}"),
                };
                cx.notify();
            },
            window,
            cx,
        );
    }

    pub(super) fn blame_file(
        &mut self,
        _: &BlameFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((repository, relative, contents)) = self.blame_relative(cx) else {
            return;
        };
        let text_lines: Vec<String> = contents.lines().map(str::to_string).collect();
        let title = format!("{relative} (blame)");
        self.git_job(
            "Blaming",
            move || repository.blame(&relative, &contents),
            move |this, lines, window, cx| {
                let text: String = lines
                    .iter()
                    .map(|blame| {
                        let source = text_lines
                            .get(blame.line - 1)
                            .map(String::as_str)
                            .unwrap_or("");
                        let who = if blame.uncommitted {
                            "uncommitted".to_string()
                        } else {
                            format!("{} {:<12.12}", &blame.commit[..8], blame.author)
                        };
                        format!("{who:<22} {:>5} │ {source}\n", blame.line)
                    })
                    .collect();
                this.open_scratch(title, &text, ide_core::language::PLAIN_TEXT, window, cx);
            },
            window,
            cx,
        );
    }

    // ----- hunks -----

    fn hunk_target(&mut self, cx: &mut Context<Self>) -> Option<(Repository, FileStatus)> {
        let repository = self.repository(cx)?;
        let relative = self
            .diff_view
            .read(cx)
            .target()
            .map(|target| target.relative.clone())
            .or_else(|| {
                self.document()
                    .path()
                    .and_then(|path| path.strip_prefix(repository.root()).ok())
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            })?;
        match self.git.read(cx).file_status(&relative) {
            Some(file) => Some((repository, file)),
            None => {
                self.status = format!("{relative} has no changes");
                cx.notify();
                None
            }
        }
    }

    fn pick_hunk(&mut self, staged: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((repository, file)) = self.hunk_target(cx) else {
            return;
        };
        let listed = file.clone();
        self.git_job(
            "Reading hunks",
            move || {
                repository
                    .hunks(&file, staged)
                    .map_err(|error| std::io::Error::other(error.to_string()))
            },
            move |this, hunks, window, cx| {
                if hunks.is_empty() {
                    this.status = format!(
                        "No {} hunks in {}",
                        if staged { "staged" } else { "unstaged" },
                        listed.relative
                    );
                    return;
                }
                let items = hunks
                    .iter()
                    .enumerate()
                    .map(|(ix, hunk)| {
                        let first_change = String::from_utf8_lossy(&hunk.patch)
                            .lines()
                            .skip(1)
                            .find(|line| line.starts_with(['+', '-']))
                            .unwrap_or_default()
                            .chars()
                            .take(80)
                            .collect::<String>();
                        PickerItem {
                            label: hunk.header.clone(),
                            detail: first_change,
                            filter: hunk.header.clone(),
                            depth: 0,
                            action: PickerAction::Git(GitAction::StageHunk(ix)),
                        }
                    })
                    .collect();
                this.git_state.hunks = hunks
                    .into_iter()
                    .map(|hunk| (listed.clone(), staged, hunk))
                    .collect();
                let title = format!(
                    "{} a hunk of {}",
                    if staged { "Unstage" } else { "Stage" },
                    listed.relative
                );
                this.show_picker(PickerKind::Locations, title, items, window, cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn stage_hunk(
        &mut self,
        _: &StageHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_hunk(false, window, cx);
    }

    pub(super) fn unstage_hunk(
        &mut self,
        _: &UnstageHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_hunk(true, window, cx);
    }

    fn apply_hunk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, staged, hunk)) = self.git_state.hunks.get(ix).cloned() else {
            return;
        };
        let Some(repository) = self.repository(cx) else {
            return;
        };
        self.git_job(
            if staged {
                "Unstaging hunk"
            } else {
                "Staging hunk"
            },
            move || {
                repository
                    .apply_hunk(&file, &hunk, staged)
                    .map_err(|error| std::io::Error::other(error.to_string()))
            },
            move |this, (), _, cx| {
                this.status = format!("{} the hunk", if staged { "Unstaged" } else { "Staged" });
                this.refresh_git(cx);
            },
            window,
            cx,
        );
        // Refresh both sides even when the hunk was stale.
        self.refresh_git(cx);
    }
}

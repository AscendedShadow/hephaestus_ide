//! Applying workspace edits from language servers (IDE-005), shared by rename, code
//! actions, formatting and server-initiated `workspace/applyEdit`.
//!
//! Plans are validated in full before mutation (see `workspace_edit::plan`). Text edits
//! to open buffers become one undo step per buffer; edits to closed files open those
//! files as modified tabs so they can be reviewed and saved explicitly. Resource
//! operations run on disk in order; if one fails the remaining steps are not applied
//! (LSP "abort" failure handling) and the result reports exactly what was applied.
//! **Undo Workspace Edit** reverses the last applied edit as a group.

use std::collections::HashMap;

use lsp_types::{ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, WorkspaceEdit};

use super::*;
use crate::workspace_edit::{self, Plan, Step};

/// What the last applied workspace edit changed, for grouped undo.
#[derive(Clone, Debug)]
pub(super) enum Applied {
    BufferEdit(BufferId),
    Created(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
    DeletedFile { path: PathBuf, bytes: Vec<u8> },
    DeletedDirectory(PathBuf),
}

#[derive(Default)]
pub(super) struct EditHistory {
    pub(super) last: Vec<Applied>,
    pub(super) label: String,
}

struct ShellSnapshot {
    open: HashMap<PathBuf, (String, Option<i32>)>,
}

impl workspace_edit::Snapshot for ShellSnapshot {
    fn open_document(&self, path: &Path) -> Option<(String, Option<i32>)> {
        self.open.get(path).cloned()
    }

    fn read_file(&self, path: &Path) -> Option<String> {
        Document::open(path)
            .ok()
            .map(|document| document.text().to_string())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

impl IdeShell {
    fn edit_snapshot(&self, cx: &App) -> ShellSnapshot {
        let servers = self.servers.read(cx);
        ShellSnapshot {
            open: self
                .buffers
                .iter()
                .filter_map(|buffer| {
                    let path = buffer.document.path()?.to_path_buf();
                    let version = servers.version(&path);
                    Some((path, (buffer.document.text().to_string(), version)))
                })
                .collect(),
        }
    }

    pub(super) fn plan_workspace_edit(
        &self,
        edit: &WorkspaceEdit,
        cx: &App,
    ) -> Result<Plan, String> {
        workspace_edit::plan(edit, &self.edit_snapshot(cx))
    }

    /// Validate and apply a workspace edit. Multi-file edits and resource operations
    /// are shown for review first when `preview` is set.
    pub(super) fn request_workspace_edit(
        &mut self,
        edit: WorkspaceEdit,
        label: String,
        preview: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = match self.plan_workspace_edit(&edit, cx) {
            Ok(plan) => plan,
            Err(error) => {
                self.status = format!("{label} not applied: {error}");
                cx.notify();
                return;
            }
        };
        if plan.is_empty() {
            self.status = format!("{label}: no changes");
            cx.notify();
            return;
        }
        if preview && (plan.files() > 1 || plan.has_resource_operations()) {
            self.preview_workspace_edit(edit, plan, label, window, cx);
            return;
        }
        self.status = match self.apply_plan(&plan, &label, window, cx) {
            Ok(summary) => summary,
            Err(error) => error,
        };
        cx.notify();
    }

    fn preview_workspace_edit(
        &mut self,
        edit: WorkspaceEdit,
        plan: Plan,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let root = self.workspace.root().map(Path::to_path_buf);
        let lines: Vec<String> = plan
            .summary()
            .into_iter()
            .map(|(path, change)| {
                let shown = root
                    .as_deref()
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(&path);
                let state = match self.buffer_for_path(&path) {
                    Some(ix) if self.buffers[ix].dirty => " (open, unsaved)",
                    Some(_) => " (open)",
                    None => " (closed: opens as a modified tab)",
                };
                format!("{} — {change}{state}", shown.display())
            })
            .collect();
        let shell = cx.entity().downgrade();
        let title = format!(
            "{label}: {} change(s) in {} file(s)",
            plan.edit_count(),
            plan.files()
        );
        let edit = Rc::new(edit);
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, edit, label) = (shell.clone(), edit.clone(), label.clone());
            dialog
                .title(title.clone())
                .w(px(560.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .max_h(px(320.))
                        .overflow_y_scrollbar()
                        .text_xs()
                        .children(lines.iter().map(|line| div().child(line.clone()))),
                )
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Apply"))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        // Re-plan: the workspace may have changed while the preview was open.
                        match shell.plan_workspace_edit(&edit, cx) {
                            Ok(plan) => {
                                shell.status = match shell.apply_plan(&plan, &label, window, cx) {
                                    Ok(summary) => summary,
                                    Err(error) => error,
                                }
                            }
                            Err(error) => shell.status = format!("{label} not applied: {error}"),
                        }
                        shell.focus_editor(window, cx);
                        cx.notify();
                    });
                    true
                })
        });
    }

    /// Apply a validated plan. Returns a summary, or an error describing what was and
    /// was not applied.
    pub(super) fn apply_plan(
        &mut self,
        plan: &Plan,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let mut applied = Vec::new();
        let total = plan.steps.len();
        for (done, step) in plan.steps.iter().enumerate() {
            if let Err(error) = self.apply_step(step, &mut applied, window, cx) {
                self.edit_history = EditHistory {
                    last: applied,
                    label: label.to_string(),
                };
                self.refresh_after_resource_changes(cx);
                return Err(format!(
                    "{label} stopped after {done} of {total} change(s): {error} — \
                     use Undo Workspace Edit to revert the applied part"
                ));
            }
        }
        self.edit_history = EditHistory {
            last: applied,
            label: label.to_string(),
        };
        if plan.has_resource_operations() {
            self.refresh_after_resource_changes(cx);
        }
        Ok(format!(
            "{label}: {} change(s) in {} file(s)",
            plan.edit_count(),
            plan.files()
        ))
    }

    fn apply_step(
        &mut self,
        step: &Step,
        applied: &mut Vec<Applied>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match step {
            Step::Edit { path, edits, .. } => {
                let ix = match self.buffer_for_path(path) {
                    Some(ix) => ix,
                    None => self.open_in_background(path, window, cx)?,
                };
                let (selection, reversed) = self.source_selection(ix, cx);
                let map = |offset| ide_core::text_edit::map_offset(edits, offset);
                let selection = Some((map(selection.start)..map(selection.end), reversed));
                if !self.apply_source_edits(ix, edits, selection, window, cx) {
                    return Err(format!("{}: the edit no longer applies", path.display()));
                }
                applied.push(Applied::BufferEdit(self.buffers[ix].id));
            }
            Step::Create { path, overwrite } => {
                if path.exists() && !overwrite {
                    return Ok(());
                }
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                }
                fs::write(path, "").map_err(|error| format!("{}: {error}", path.display()))?;
                applied.push(Applied::Created(path.clone()));
            }
            Step::Rename {
                from,
                to,
                overwrite,
            } => {
                if to.exists() && !overwrite {
                    return Err(format!("{} already exists", to.display()));
                }
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("{}: {error}", to.display()))?;
                }
                fs::rename(from, to).map_err(|error| format!("{}: {error}", from.display()))?;
                self.retarget_paths(from, to, cx);
                applied.push(Applied::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
            Step::Delete { path, recursive } => {
                if path.is_dir() {
                    let result = if *recursive {
                        fs::remove_dir_all(path)
                    } else {
                        fs::remove_dir(path)
                    };
                    result.map_err(|error| format!("{}: {error}", path.display()))?;
                    applied.push(Applied::DeletedDirectory(path.clone()));
                } else {
                    let bytes = fs::read(path).unwrap_or_default();
                    fs::remove_file(path)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                    applied.push(Applied::DeletedFile {
                        path: path.clone(),
                        bytes,
                    });
                }
                self.forget_deleted(path, window, cx);
            }
        }
        Ok(())
    }

    /// Open a file as a new tab without activating it (for edits to closed files).
    pub(super) fn open_in_background(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let document =
            Document::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let buffer = self.new_buffer(document, window, cx);
        self.buffers.push(buffer);
        let ix = self.buffers.len() - 1;
        self.sync_lsp_buffer(ix, cx);
        Ok(ix)
    }

    /// Follow a file or folder move on disk: buffer paths, language-server identities,
    /// diagnostics, breakpoints, recent files and navigation history move with it.
    /// Unsaved edits are kept.
    pub(super) fn retarget_paths(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let moved = |path: &Path| -> Option<PathBuf> {
            let suffix = path.strip_prefix(from).ok()?;
            Some(if suffix.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(suffix)
            })
        };
        for ix in 0..self.buffers.len() {
            let Some(old) = self.buffers[ix].document.path().map(Path::to_path_buf) else {
                continue;
            };
            let Some(new) = moved(&old) else { continue };
            self.servers.update(cx, |servers, _| servers.close(&old));
            self.diagnostics.remove(&old);
            let buffer = &mut self.buffers[ix];
            buffer.document.set_path(new.clone());
            let editor = buffer.editor.clone();
            if !buffer.language_overridden {
                let language = syntax::detect(Some(&new), Some(&first_line(&buffer.document)));
                if language != buffer.language {
                    buffer.language = language;
                    editor.update(cx, |editor, cx| editor.set_highlighter(language, cx));
                }
            }
            let language = buffer.language;
            language_server::attach(&editor, &self.servers, Some(new), language, cx);
            self.sync_lsp_buffer(ix, cx);
        }
        for path in self
            .recent_paths
            .iter_mut()
            .chain(self.search_paths.iter_mut())
        {
            if let Some(new) = moved(path) {
                *path = new;
            }
        }
        for location in &mut self.locations {
            if let Some(new) = moved(&location.path) {
                location.path = new;
            }
        }
        self.debugger
            .update(cx, |debugger, cx| debugger.move_breakpoints(from, to, cx));
    }

    /// A file or folder was deleted: close clean buffers below it; dirty buffers stay
    /// open, marked as changed on disk, so their content can be saved elsewhere.
    pub(super) fn forget_deleted(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut ix = 0;
        while ix < self.buffers.len() {
            let inside = self.buffers[ix]
                .document
                .path()
                .is_some_and(|buffer_path| buffer_path.starts_with(path));
            if inside && !self.buffers[ix].dirty {
                self.remove_buffer(ix, window, cx);
                continue;
            }
            if inside {
                self.buffers[ix].external_change = true;
            }
            ix += 1;
        }
        self.recent_paths.retain(|recent| !recent.starts_with(path));
    }

    pub(super) fn refresh_after_resource_changes(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.workspace.root().map(Path::to_path_buf) {
            let expanded: Vec<PathBuf> = self
                .tree_rows
                .iter()
                .filter(|row| row.expanded)
                .map(|row| row.entry.path.clone())
                .chain([root])
                .collect();
            for directory in expanded {
                if directory.is_dir() {
                    self.load_directory(directory, cx);
                } else {
                    self.workspace.collapse(&directory);
                }
            }
            self.tree_rows = self.workspace.rows();
        }
        self.index_workspace(cx);
        self.refresh_git(cx);
    }

    pub(super) fn undo_workspace_edit(
        &mut self,
        _: &UndoWorkspaceEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = std::mem::take(&mut self.edit_history);
        if history.last.is_empty() {
            self.status = "No workspace edit to undo".into();
            cx.notify();
            return;
        }
        let mut problems = Vec::new();
        for step in history.last.into_iter().rev() {
            match step {
                Applied::BufferEdit(id) => {
                    if let Some(ix) = self.buffer_index(id) {
                        self.buffers[ix]
                            .editor
                            .update(cx, |editor, cx| editor.undo_change(window, cx));
                    }
                }
                Applied::Created(path) => {
                    let unchanged = fs::metadata(&path).is_ok_and(|metadata| metadata.len() == 0)
                        && self
                            .buffer_for_path(&path)
                            .is_none_or(|ix| !self.buffers[ix].dirty);
                    if unchanged {
                        if let Some(ix) = self.buffer_for_path(&path) {
                            self.remove_buffer(ix, window, cx);
                        }
                        let _ = fs::remove_file(&path);
                    } else {
                        problems.push(format!("kept {} (modified since)", path.display()));
                    }
                }
                Applied::Renamed { from, to } => match fs::rename(&to, &from) {
                    Ok(()) => self.retarget_paths(&to, &from, cx),
                    Err(error) => problems.push(format!("{}: {error}", to.display())),
                },
                Applied::DeletedFile { path, bytes } => {
                    if let Err(error) = fs::write(&path, bytes) {
                        problems.push(format!("{}: {error}", path.display()));
                    }
                }
                Applied::DeletedDirectory(path) => {
                    problems.push(format!("cannot restore deleted folder {}", path.display()))
                }
            }
        }
        self.refresh_after_resource_changes(cx);
        self.status = if problems.is_empty() {
            format!("Undid {}", history.label)
        } else {
            format!("Undid {} partially: {}", history.label, problems.join("; "))
        };
        cx.notify();
    }

    /// Handle a server-initiated `workspace/applyEdit` with the same applier, and report
    /// the truthful outcome back to the server.
    pub(super) fn server_apply_edit(
        &mut self,
        server: String,
        id: serde_json::Value,
        params: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let response = match serde_json::from_value::<ApplyWorkspaceEditParams>(params) {
            Ok(params) => {
                let label = params.label.unwrap_or_else(|| format!("{server} edit"));
                match self
                    .plan_workspace_edit(&params.edit, cx)
                    .and_then(|plan| self.apply_plan(&plan, &label, window, cx))
                {
                    Ok(summary) => {
                        self.status = summary;
                        ApplyWorkspaceEditResponse {
                            applied: true,
                            failure_reason: None,
                            failed_change: None,
                        }
                    }
                    Err(error) => {
                        self.status = error.clone();
                        ApplyWorkspaceEditResponse {
                            applied: false,
                            failure_reason: Some(error),
                            failed_change: None,
                        }
                    }
                }
            }
            Err(error) => ApplyWorkspaceEditResponse {
                applied: false,
                failure_reason: Some(format!("invalid workspace/applyEdit parameters: {error}")),
                failed_change: None,
            },
        };
        let result = serde_json::to_value(response).map_err(|error| error.to_string());
        self.servers.read(cx).respond(&server, id, result);
        cx.notify();
    }
}

//! Multi-root workspaces (IDE-017).
//!
//! A workspace can hold several root folders and be saved to a definition file with
//! paths relative to it. Explorer, Quick Open and project search cover every root (each
//! root can be excluded from search) and show root-qualified names. Language servers get
//! all roots as workspace folders. Root-scoped features — workspace settings, Git, run
//! and debug working directories, test discovery — use the *active root*, which the
//! user picks explicitly (Select Active Root, or the root's row in the explorer); it does
//! not follow the active editor.

use ide_core::workspace::{DEFINITION_EXTENSION, Workspace};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;

#[derive(Clone, Debug)]
pub(super) enum RootAction {
    Activate(PathBuf),
    Remove(PathBuf),
    ToggleSearch(PathBuf),
}

#[derive(Default)]
pub(super) struct RootsState {
    /// The explicitly selected active root (when it is still part of the workspace).
    active: Option<PathBuf>,
}

impl IdeShell {
    /// The root that root-scoped features use: the selected one, else the primary root.
    pub(super) fn active_root(&self) -> Option<&Path> {
        self.roots
            .active
            .as_deref()
            .filter(|active| {
                self.workspace
                    .roots()
                    .iter()
                    .any(|root| root.path == *active)
            })
            .or(self.workspace.root())
    }

    pub(super) fn settings_root(&self) -> Option<PathBuf> {
        self.active_root().map(Path::to_path_buf)
    }

    pub(super) fn set_active_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.roots.active = root;
        let root = self.settings_root();
        if let Some(problem) = settings::summary(&settings::reload(root.as_deref(), None, cx)) {
            self.status = problem;
        } else if let Some(root) = &root {
            self.status = format!("Active root: {}", self.workspace.label(root));
        }
        self.refresh_preferences(None, cx);
        self.sync_servers(cx);
        self.sync_git(cx);
        cx.notify();
    }

    /// Add a root folder to the open workspace (or open it when none is open).
    pub(super) fn add_workspace_root(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.workspace.root().is_none() {
            let workspace =
                Workspace::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
            self.set_workspace(workspace, cx);
            return Ok(());
        }
        let added = self
            .workspace
            .add_root(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !added {
            self.status = format!("{} is already in the workspace", path.display());
            cx.notify();
            return Ok(());
        }
        self.roots_changed(cx);
        self.status = format!("Added {} to the workspace", path.display());
        Ok(())
    }

    /// Refresh everything that depends on the set of roots. Open buffers (including
    /// unsaved ones) are untouched.
    pub(super) fn roots_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.workspace.file().map(Path::to_path_buf)
            && let Err(error) = self.workspace.save_definition(&file)
        {
            self.status = format!("Could not save {}: {error}", file.display());
        }
        self.tree_rows = self.workspace.rows();
        self.index_workspace(cx);
        self.invalidate_search_files();
        self.start_watching(cx);
        self.sync_servers(cx);
        self.sync_git(cx);
        cx.notify();
    }

    fn root_items(&self, action: fn(PathBuf) -> RootAction) -> Vec<PickerItem> {
        let active = self.active_root().map(Path::to_path_buf);
        self.workspace
            .roots()
            .iter()
            .map(|root| {
                let mut notes = vec![root.path.display().to_string()];
                if active.as_ref() == Some(&root.path) {
                    notes.push("active".into());
                }
                if !root.search {
                    notes.push("excluded from search".into());
                }
                if let Some(error) = &root.error {
                    notes.push(format!("unavailable: {error}"));
                }
                let label = self.workspace.label(&root.path);
                PickerItem {
                    filter: label.clone(),
                    label,
                    detail: notes.join(" · "),
                    depth: 0,
                    action: PickerAction::Root(action(root.path.clone())),
                }
            })
            .collect()
    }

    fn show_root_picker(
        &mut self,
        title: &str,
        action: fn(PathBuf) -> RootAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        if self.workspace.root().is_none() {
            self.status = "Open a folder first".into();
            cx.notify();
            return;
        }
        let items = self.root_items(action);
        self.show_picker(PickerKind::Locations, title.into(), items, window, cx);
    }

    pub(super) fn run_root_action(
        &mut self,
        action: RootAction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            RootAction::Activate(root) => {
                if let Some(error) = self
                    .workspace
                    .roots()
                    .iter()
                    .find(|candidate| candidate.path == root)
                    .and_then(|root| root.error.clone())
                {
                    // Selecting an unavailable root retries it.
                    match self.workspace.retry(&root) {
                        Ok(()) => self.roots_changed(cx),
                        Err(_) => {
                            self.status = format!("{} is unavailable: {error}", root.display());
                            cx.notify();
                            return;
                        }
                    }
                }
                self.set_active_root(Some(root), cx);
            }
            RootAction::Remove(root) => self.remove_workspace_root(&root, cx),
            RootAction::ToggleSearch(root) => {
                let search = self
                    .workspace
                    .roots()
                    .iter()
                    .find(|candidate| candidate.path == root)
                    .is_some_and(|root| !root.search);
                self.workspace.set_search(&root, search);
                self.status = format!(
                    "{} {} search",
                    self.workspace.label(&root),
                    if search {
                        "included in"
                    } else {
                        "excluded from"
                    }
                );
                self.roots_changed(cx);
            }
        }
    }

    pub(super) fn remove_workspace_root(&mut self, root: &Path, cx: &mut Context<Self>) {
        let label = self.workspace.label(root);
        if !self.workspace.remove_root(root) {
            self.status = "The workspace needs at least one folder".into();
            cx.notify();
            return;
        }
        if self.roots.active.as_deref() == Some(root) {
            self.set_active_root(None, cx);
        }
        self.roots_changed(cx);
        self.status = format!("Removed {label} from the workspace (open files stay open)");
    }

    pub(super) fn add_folder_to_workspace(
        &mut self,
        _: &AddFolderToWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add folder to workspace".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picker.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                for path in paths {
                    if let Err(error) = this.add_workspace_root(&path, cx) {
                        this.status = format!("Could not add folder: {error}");
                    }
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn remove_folder_from_workspace(
        &mut self,
        _: &RemoveFolderFromWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_root_picker(
            "Remove folder from workspace",
            RootAction::Remove,
            window,
            cx,
        );
    }

    pub(super) fn select_active_root(
        &mut self,
        _: &SelectActiveRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_root_picker("Select active root", RootAction::Activate, window, cx);
    }

    pub(super) fn toggle_root_search(
        &mut self,
        _: &ToggleRootSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_root_picker(
            "Include or exclude a root from search",
            RootAction::ToggleSearch,
            window,
            cx,
        );
    }

    pub(super) fn save_workspace_as(
        &mut self,
        _: &SaveWorkspaceAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(root) = self.workspace.root().map(Path::to_path_buf) else {
            self.status = "Open a folder first".into();
            cx.notify();
            return;
        };
        let directory = root.parent().unwrap_or(&root).to_path_buf();
        let name = format!(
            "{}.{DEFINITION_EXTENSION}",
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".into())
        );
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picker.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.save_workspace_definition(&path, cx);
                this.focus_editor(window, cx);
            });
        })
        .detach();
    }

    pub(super) fn save_workspace_definition(&mut self, path: &Path, cx: &mut Context<Self>) {
        let path = if path
            .extension()
            .is_some_and(|extension| extension == DEFINITION_EXTENSION)
        {
            path.to_path_buf()
        } else {
            path.with_extension(DEFINITION_EXTENSION)
        };
        self.status = match self.workspace.save_definition(&path) {
            Ok(()) => {
                self.tree_rows = self.workspace.rows();
                format!("Saved workspace {}", path.display())
            }
            Err(error) => format!("Could not save the workspace: {error}"),
        };
        cx.notify();
    }

    /// Open a saved workspace definition. Unsaved buffers stay open.
    pub(super) fn open_workspace_definition(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let workspace = Workspace::open_definition(path).map_err(|error| error.to_string())?;
        let unavailable: Vec<String> = workspace
            .roots()
            .iter()
            .filter(|root| root.error.is_some())
            .map(|root| workspace.label(&root.path))
            .collect();
        self.set_workspace(workspace, cx);
        if !unavailable.is_empty() {
            self.status = format!(
                "Unavailable folders: {} — select them in Select Active Root to retry, or remove them",
                unavailable.join(", ")
            );
        }
        Ok(())
    }

    pub(super) fn is_workspace_definition(path: &Path) -> bool {
        path.extension()
            .is_some_and(|extension| extension == DEFINITION_EXTENSION)
    }

    /// The workspace part of the session.
    pub(super) fn session_roots(&self) -> crate::session::Session {
        let multi = self.workspace.roots().len() > 1;
        crate::session::Session {
            workspace_file: self.workspace.file().map(Path::to_path_buf),
            roots: if multi && self.workspace.file().is_none() {
                self.workspace.root_paths()
            } else {
                Vec::new()
            },
            excluded_roots: self
                .workspace
                .roots()
                .iter()
                .filter(|root| !root.search)
                .map(|root| root.path.clone())
                .collect(),
            active_root: self.roots.active.clone(),
            ..Default::default()
        }
    }

    /// Reopen the session's workspace: its definition file, else its folders. Folders
    /// that are unavailable now stay listed with an error.
    pub(super) fn restore_roots(
        &mut self,
        snapshot: &crate::session::Session,
        cx: &mut Context<Self>,
    ) {
        let from_file = snapshot
            .workspace_file
            .as_ref()
            .is_some_and(|file| self.open_workspace_definition(file, cx).is_ok());
        if from_file {
            // Roots, search scope and errors come from the definition.
        } else if let Some(folder) = &snapshot.folder
            && let Ok(workspace) = Workspace::open(folder)
        {
            self.set_workspace(workspace, cx);
            for root in snapshot.roots.iter().filter(|root| *root != folder) {
                if let Err(error) = self.workspace.add_root(root) {
                    self.workspace
                        .add_unavailable_root(root.clone(), error.to_string());
                }
            }
            for root in &snapshot.excluded_roots {
                self.workspace.set_search(root, false);
            }
            if snapshot.roots.len() > 1 || !snapshot.excluded_roots.is_empty() {
                self.roots_changed(cx);
            }
        } else {
            return;
        }
        if let Some(active) = &snapshot.active_root
            && self
                .workspace
                .roots()
                .iter()
                .any(|root| root.path == *active)
        {
            self.set_active_root(Some(active.clone()), cx);
        }
    }
}

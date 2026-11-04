//! Explorer file operations (IDE-001): create, rename, move and delete files and folders.
//!
//! Operations target the selected explorer item (or the workspace root) and are
//! available from context menus and the command palette. Renames and moves keep open
//! buffers (including unsaved edits), language-server identities, breakpoints, search
//! entries and expanded folders in sync; Git decorations and Quick Open refresh after.

use gpui_component::input::InputState;
use ide_core::workspace as fs_ops;

use super::*;

type NameAction =
    fn(&mut IdeShell, PathBuf, String, &mut Window, &mut Context<IdeShell>) -> Result<(), String>;

impl IdeShell {
    /// The folder new items go into: the selected folder, the selected file's folder, or
    /// the workspace root.
    pub(super) fn target_directory(&self) -> Option<PathBuf> {
        let root = self.workspace.root()?.to_path_buf();
        Some(match &self.tree_selection {
            Some(path) if path.is_dir() => path.clone(),
            Some(path) => path.parent().map(Path::to_path_buf).unwrap_or(root),
            None => root,
        })
    }

    /// The item explorer commands act on: the selected item, else the active file.
    fn selected_item(&self) -> Option<PathBuf> {
        self.tree_selection
            .clone()
            .filter(|path| path.exists())
            .or_else(|| self.document().path().map(Path::to_path_buf))
            .filter(|path| {
                self.workspace
                    .root()
                    .is_some_and(|root| path.starts_with(root) && path != root)
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn prompt_name(
        &mut self,
        title: String,
        context: PathBuf,
        initial: String,
        ok: &'static str,
        action: NameAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || window.has_active_dialog(cx) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial.clone())
                .validate(|text, _| !text.contains('\n'))
        });
        // Preselect the name without its extension.
        let stem = initial
            .rfind('.')
            .filter(|ix| *ix > 0)
            .unwrap_or(initial.len());
        input.update(cx, |input, cx| {
            input.select_byte_range(0..stem, false, window, cx)
        });
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let (shell, input, context) = (shell.clone(), input.clone(), context.clone());
                dialog
                    .title(title.clone())
                    .w(px(420.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text(ok))
                    .on_ok(move |_, window, cx| {
                        let value = input.read(cx).value().to_string();
                        shell
                            .update(cx, |shell, cx| {
                                let result = action(shell, context.clone(), value, window, cx);
                                if let Err(error) = &result {
                                    shell.status = error.clone();
                                    cx.notify();
                                }
                                result.is_ok()
                            })
                            .unwrap_or(true)
                    })
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn explorer_new_file(
        &mut self,
        _: &ExplorerNewFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.target_directory() else {
            self.status = "Open a folder to create files in it".into();
            cx.notify();
            return;
        };
        self.new_item_in(directory, false, window, cx);
    }

    pub(super) fn explorer_new_folder(
        &mut self,
        _: &ExplorerNewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.target_directory() else {
            self.status = "Open a folder to create folders in it".into();
            cx.notify();
            return;
        };
        self.new_item_in(directory, true, window, cx);
    }

    pub(super) fn new_item_in(
        &mut self,
        directory: PathBuf,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.relative_name(&directory);
        let title = format!("New {} in {shown}", if folder { "Folder" } else { "File" });
        let action: NameAction = if folder {
            |shell, directory, name, window, cx| {
                let path = fs_ops::create(&directory, &name, true)?;
                shell.after_tree_change(Some(&path), cx);
                shell.status = format!("Created folder {name}");
                let _ = window;
                Ok(())
            }
        } else {
            |shell, directory, name, window, cx| {
                let path = fs_ops::create(&directory, &name, false)?;
                shell.after_tree_change(Some(&path), cx);
                shell.open(Some(path), window, cx);
                Ok(())
            }
        };
        self.prompt_name(
            title,
            directory,
            String::new(),
            "Create",
            action,
            window,
            cx,
        );
    }

    fn relative_name(&self, path: &Path) -> String {
        match self.workspace.root() {
            Some(root) if path == root => self.workspace.display_name().to_string(),
            Some(root) => path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string(),
            None => path.display().to_string(),
        }
    }

    pub(super) fn rename_item(
        &mut self,
        _: &RenameItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.selected_item() {
            Some(path) => self.prompt_rename_item(path, window, cx),
            None => {
                self.status = "Select a file or folder in the explorer to rename".into();
                cx.notify();
            }
        }
    }

    pub(super) fn prompt_rename_item(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = format!("Rename {name}");
        self.prompt_name(
            title,
            path,
            name,
            "Rename",
            |shell, path, name, _, cx| {
                fs_ops::validate_name(&name, false)?;
                let target = path.with_file_name(&name);
                shell.move_item_to(&path, &target, cx)
            },
            window,
            cx,
        );
    }

    pub(super) fn move_item(&mut self, _: &MoveItem, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_item() {
            Some(path) => self.prompt_move_item(path, window, cx),
            None => {
                self.status = "Select a file or folder in the explorer to move".into();
                cx.notify();
            }
        }
    }

    pub(super) fn prompt_move_item(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = path
            .parent()
            .map(|parent| {
                let relative = self.relative_name(parent);
                if Some(parent) == self.workspace.root() {
                    String::new()
                } else {
                    relative
                }
            })
            .unwrap_or_default();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.prompt_name(
            format!("Move {name} to folder (relative to the workspace root)"),
            path,
            parent,
            "Move",
            |shell, path, folder, _, cx| {
                let Some(root) = shell.workspace.root().map(Path::to_path_buf) else {
                    return Err("No workspace is open".into());
                };
                let folder = folder.trim().trim_matches(['/', '\\']);
                if !folder.is_empty() {
                    fs_ops::validate_name(folder, true)?;
                }
                let destination = if folder.is_empty() {
                    root.clone()
                } else {
                    root.join(folder)
                };
                if !destination.is_dir() {
                    return Err(format!("{folder} is not a folder in the workspace"));
                }
                let target = destination.join(path.file_name().unwrap_or_default());
                shell.move_item_to(&path, &target, cx)
            },
            window,
            cx,
        );
    }

    /// Move or rename on disk, then retarget everything that referred to the old path.
    pub(super) fn move_item_to(
        &mut self,
        from: &Path,
        to: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        fs_ops::move_path(from, to)?;
        self.workspace.remap(from, to);
        if self.tree_selection.as_deref() == Some(from) {
            self.tree_selection = Some(to.to_path_buf());
        }
        self.retarget_paths(from, to, cx);
        self.after_tree_change(Some(to), cx);
        self.status = format!(
            "Moved {} to {}",
            self.relative_name(from),
            self.relative_name(to)
        );
        cx.notify();
        Ok(())
    }

    /// Refresh listings (keeping expanded folders), the Quick Open index and Git state.
    pub(super) fn after_tree_change(&mut self, changed: Option<&Path>, cx: &mut Context<Self>) {
        self.workspace.prune();
        for directory in self.workspace.expanded() {
            self.load_directory(directory, cx);
        }
        if let Some(parent) = changed.and_then(Path::parent)
            && !self.workspace.is_expanded(parent)
            && self
                .workspace
                .root()
                .is_some_and(|root| parent.starts_with(root))
        {
            self.load_directory(parent.to_path_buf(), cx);
        }
        self.tree_rows = self.workspace.rows();
        self.index_workspace(cx);
        self.refresh_git(cx);
    }

    pub(super) fn delete_item(
        &mut self,
        _: &DeleteItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.selected_item() {
            Some(path) if path.is_dir() => self.confirm_delete_folder(path, window, cx),
            Some(path) => self.confirm_delete_tree_file(path, window, cx),
            None => {
                self.status = "Select a file or folder in the explorer to delete".into();
                cx.notify();
            }
        }
    }

    /// Delete a folder after confirmation. Open files inside it are closed; unsaved
    /// changes in them are listed explicitly and discarded only on confirmation.
    pub(super) fn confirm_delete_folder(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() || window.has_active_dialog(cx) {
            return;
        }
        let dirty: Vec<String> = self
            .buffers
            .iter()
            .filter(|buffer| {
                buffer.dirty
                    && buffer
                        .document
                        .path()
                        .is_some_and(|file| file.starts_with(&path))
            })
            .map(|buffer| buffer.document.name())
            .collect();
        let name = self.relative_name(&path);
        let mut message =
            format!("Delete the folder {name} and everything in it? This cannot be undone.");
        if !dirty.is_empty() {
            message.push_str(&format!(
                "\n\nUnsaved changes in {} will be discarded.",
                dirty.join(", ")
            ));
        }
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, path) = (shell.clone(), path.clone());
            dialog
                .title("Delete Folder")
                .child(message.clone())
                .confirm()
                .button_props(DialogButtonProps::default().ok_text(if dirty.is_empty() {
                    "Delete"
                } else {
                    "Discard Changes and Delete"
                }))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.delete_folder(path.clone(), window, cx)
                    });
                    true
                })
        });
    }

    pub(super) fn delete_folder(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = fs::remove_dir_all(&path) {
            self.status = format!("Delete failed: {}: {error}", path.display());
            cx.notify();
            return;
        }
        let mut ix = 0;
        while ix < self.buffers.len() {
            if self.buffers[ix]
                .document
                .path()
                .is_some_and(|file| file.starts_with(&path))
            {
                self.remove_buffer(ix, window, cx);
            } else {
                ix += 1;
            }
        }
        if self
            .tree_selection
            .as_ref()
            .is_some_and(|selected| selected.starts_with(&path))
        {
            self.tree_selection = None;
        }
        self.workspace.collapse(&path);
        self.after_tree_change(Some(&path), cx);
        self.status = format!("Deleted {}", self.relative_name(&path));
        cx.notify();
    }
}

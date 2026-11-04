//! Opening and cloning folders, and indexing the workspace for Quick Open.

use super::*;

impl IdeShell {
    pub(super) fn open_folder(
        &mut self,
        _: &OpenFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn clone_repository(
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

    pub(super) fn choose_clone_location(
        &mut self,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn clone_into(&mut self, url: String, parent: PathBuf, cx: &mut Context<Self>) {
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

    pub(super) fn set_workspace(&mut self, workspace: Workspace, cx: &mut Context<Self>) {
        if self.workspace.root_paths() != workspace.root_paths() {
            self.task_terminal
                .update(cx, |terminal, cx| terminal.terminate(cx));
            self.debugger.update(cx, |debugger, cx| debugger.stop(cx));
        }
        self.status = format!("Opened folder {}", workspace.display_name());
        self.workspace = workspace;
        self.roots = Default::default();
        let root = self.settings_root();
        if let Some(problem) = settings::summary(&settings::reload(root.as_deref(), None, cx)) {
            self.status = problem;
        }
        self.refresh_preferences(None, cx);
        self.diagnostics.clear();
        self.sync_servers(cx);
        self.index_workspace(cx);
        self.tree_rows = self.workspace.rows();
        self.tree_scroll = UniformListScrollHandle::new();
        self.tree_selection = None;
        self.cancel_tasks("Workspace changed", cx);
        self.cancel_test_run(cx);
        self.testing = Default::default();
        self.problems = Default::default();
        self.load_debug_state(cx);
        self.start_watching(cx);
        self.sync_git(cx);
        cx.notify();
    }

    pub(super) fn index_workspace(&mut self, cx: &mut Context<Self>) {
        self.search_paths.clear();
        self.bump_search_paths();
        self.invalidate_search_files();
        let roots: Vec<PathBuf> = self
            .workspace
            .roots()
            .iter()
            .filter(|root| root.search && root.error.is_none())
            .map(|root| root.path.clone())
            .collect();
        if roots.is_empty() {
            return;
        }
        let all = self.workspace.root_paths();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let paths = executor
                .spawn({
                    let roots = roots.clone();
                    async move {
                        let mut paths = Vec::new();
                        for root in &roots {
                            paths.extend(search::files(root)?);
                        }
                        Ok::<_, std::io::Error>(paths)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace.root_paths() == all {
                    match paths {
                        Ok(paths) => {
                            this.search_paths = paths;
                            this.bump_search_paths();
                        }
                        Err(error) => this.status = format!("Could not index files: {error}"),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

//! The explorer tree: loading folders, selection, deletion and rendering.

use super::*;

impl IdeShell {
    pub(super) fn load_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
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

    pub(super) fn activate_tree_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.tree_rows.get(ix) else {
            return;
        };
        let path = row.entry.path.clone();
        self.tree_selection = Some(path.clone());
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

    pub(super) fn reveal_tree_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut command = if cfg!(target_os = "windows") {
            let mut command = ProcessCommand::new("explorer.exe");
            let file = path.to_string_lossy();
            let file = file
                .strip_prefix(r"\\?\UNC\")
                .map(|rest| format!(r"\\{rest}"))
                .or_else(|| file.strip_prefix(r"\\?\").map(str::to_string))
                .unwrap_or_else(|| file.into_owned());
            command.arg(format!("/select,{file}"));
            command
        } else if cfg!(target_os = "macos") {
            let mut command = ProcessCommand::new("open");
            command.arg("-R").arg(path);
            command
        } else {
            let mut command = ProcessCommand::new("xdg-open");
            command.arg(path.parent().unwrap_or(path));
            command
        };
        if let Err(error) = command.spawn() {
            self.status = format!("Could not reveal {}: {error}", path.display());
            cx.notify();
        }
    }

    pub(super) fn confirm_delete_tree_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked()
            || window.has_active_dialog(cx)
            || !self
                .tree_rows
                .iter()
                .any(|row| row.entry.path == path && !row.entry.is_dir)
        {
            return;
        }
        if self
            .buffer_for_path(&path)
            .is_some_and(|ix| self.buffers[ix].dirty)
        {
            self.status = "Save or close the modified file before deleting it".into();
            cx.notify();
            return;
        }
        let shell = cx.entity().downgrade();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, path) = (shell.clone(), path.clone());
            dialog
                .title("Delete File")
                .child(format!("Delete {name}? This cannot be undone."))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Delete"))
                .on_ok(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.delete_tree_file(path.clone(), window, cx)
                    });
                    true
                })
        });
    }

    pub(super) fn delete_tree_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked()
            || !self
                .tree_rows
                .iter()
                .any(|row| row.entry.path == path && !row.entry.is_dir)
            || self
                .buffer_for_path(&path)
                .is_some_and(|ix| self.buffers[ix].dirty)
        {
            return;
        }
        self.busy = true;
        self.status = format!("Deleting {}…", path.display());
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = executor
                .spawn({
                    let path = path.clone();
                    async move { fs::remove_file(&path) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if let Some(ix) = this.buffer_for_path(&path) {
                            this.remove_buffer(ix, window, cx);
                        }
                        this.refresh_parent(Some(&path), cx);
                        this.index_workspace(cx);
                        this.refresh_git(cx);
                        this.status = format!("Deleted {}", path.display());
                    }
                    Err(error) => {
                        this.status = format!("Delete failed: {}: {error}", path.display())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_root = self.workspace.root().is_some();
        let caption = if has_root {
            self.workspace.display_name().to_uppercase()
        } else {
            "EXPLORER".into()
        };
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                ui::panel_header(caption)
                    .child(
                        Button::new("sidebar-new-file")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New File")
                            .disabled(self.blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_file(&NewFile, window, cx)
                            })),
                    )
                    .child(
                        Button::new("sidebar-new-named-file")
                            .ghost()
                            .xsmall()
                            .icon(IconName::File)
                            .tooltip("New File in Folder…")
                            .disabled(self.blocked() || !has_root)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.explorer_new_file(&ExplorerNewFile, window, cx)
                            })),
                    )
                    .child(
                        Button::new("sidebar-new-folder")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Folder)
                            .tooltip("New Folder…")
                            .disabled(self.blocked() || !has_root)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.explorer_new_folder(&ExplorerNewFolder, window, cx)
                            })),
                    )
                    .child(
                        Button::new("sidebar-open-folder")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderOpen)
                            .tooltip("Open Folder…")
                            .disabled(self.blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_folder(&OpenFolder, window, cx)
                            })),
                    ),
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
                            .px_1p5()
                            .track_scroll(self.tree_scroll.clone()),
                        ),
                    )
                } else {
                    sidebar.child(
                        ui::empty_state(
                            Icon::new(IconName::FolderOpen).size(px(28.)),
                            "No folder open",
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::subtle())
                                .child("Open a folder to browse and edit its files."),
                        )
                        .child(
                            div()
                                .pt_2()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    Button::new("open-folder")
                                        .primary()
                                        .small()
                                        .label("Open Folder…")
                                        .disabled(self.blocked())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_folder(&OpenFolder, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("clone-repository")
                                        .small()
                                        .label(if self.cloning {
                                            "Cloning…"
                                        } else {
                                            "Clone Repository…"
                                        })
                                        .loading(self.cloning)
                                        .disabled(self.blocked() || self.cloning)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clone_repository(&CloneRepository, window, cx)
                                        })),
                                ),
                        ),
                    )
                }
            })
    }

    pub(super) fn render_tree_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = &self.tree_rows[ix];
        let path = row.entry.path.clone();
        let is_file = !row.entry.is_dir;
        let shell = cx.entity().downgrade();
        let is_open = !row.entry.is_dir && self.document().path() == Some(row.entry.path.as_path());
        let selected = self.tree_selection.as_deref() == Some(row.entry.path.as_path());
        let git = self.git.read(cx);
        let change = git.tree_change(&row.entry.path);
        let ignored = git.tree_ignored(&row.entry.path);
        let letter = change.filter(|_| !row.entry.is_dir).map(Change::letter);
        let kind = FileKind::of(&row.entry.path);
        let (chevron, icon) = match (row.entry.is_dir, row.expanded) {
            (true, true) => (Some(IconName::ChevronDown), Icon::new(IconName::FolderOpen)),
            (true, false) => (Some(IconName::ChevronRight), Icon::new(IconName::Folder)),
            (false, _) => (None, Icon::new(kind.icon())),
        };
        div()
            .id(ix)
            .w_full()
            .h(px(24.))
            .flex()
            .items_center()
            .gap_1p5()
            .pl(px(4. + 14. * row.depth as f32))
            .pr_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()))
            .when(is_open || selected, |row| row.bg(theme::active_row()))
            .when(ignored, |row| row.text_color(theme::subtle()))
            .when_some(change.filter(|_| !ignored), |row, change| {
                row.text_color(theme::git_change(change))
            })
            .child(
                div()
                    .size(px(14.))
                    .flex_shrink_0()
                    .when_some(chevron, |slot, chevron| {
                        slot.child(Icon::new(chevron).size_full().text_color(theme::subtle()))
                    }),
            )
            .child(icon.size(px(15.)).flex_shrink_0().text_color(if ignored {
                theme::subtle()
            } else if row.entry.is_dir {
                theme::accent()
            } else {
                theme::file_icon(kind)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.entry.name.clone()),
            )
            .when_some(letter, |row, letter| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(letter),
                )
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate_tree_row(ix, window, cx)),
            )
            .context_menu(move |menu, _, _| {
                let item = |label: &'static str,
                            path: &PathBuf,
                            run: fn(
                    &mut IdeShell,
                    PathBuf,
                    &mut Window,
                    &mut Context<IdeShell>,
                )| {
                    let (shell, path) = (shell.clone(), path.clone());
                    PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = shell.update(cx, |shell, cx| {
                            if !shell.blocked() {
                                run(shell, path.clone(), window, cx)
                            }
                        });
                    })
                };
                if !is_file {
                    return menu
                        .item(item("New File…", &path, |shell, path, window, cx| {
                            shell.new_item_in(path, false, window, cx)
                        }))
                        .item(item("New Folder…", &path, |shell, path, window, cx| {
                            shell.new_item_in(path, true, window, cx)
                        }))
                        .separator()
                        .item(item(
                            "Find in Folder…",
                            &path,
                            |shell, path, window, cx| shell.search_in_folder(path, window, cx),
                        ))
                        .item(item(
                            "New Terminal Here",
                            &path,
                            |shell, path, window, cx| shell.new_terminal_in(Some(path), window, cx),
                        ))
                        .separator()
                        .item(item("Rename…", &path, |shell, path, window, cx| {
                            shell.prompt_rename_item(path, window, cx)
                        }))
                        .item(item("Move…", &path, |shell, path, window, cx| {
                            shell.prompt_move_item(path, window, cx)
                        }))
                        .separator()
                        .item(item(
                            "Delete Folder…",
                            &path,
                            |shell, path, window, cx| shell.confirm_delete_folder(path, window, cx),
                        ));
                }
                menu.item(item("View File", &path, |shell, path, window, cx| {
                    shell.open(Some(path), window, cx)
                }))
                .item(item(
                    "Reveal in File Explorer",
                    &path,
                    |shell, path, _, cx| shell.reveal_tree_file(&path, cx),
                ))
                .separator()
                .item(item("Rename…", &path, |shell, path, window, cx| {
                    shell.prompt_rename_item(path, window, cx)
                }))
                .item(item("Move…", &path, |shell, path, window, cx| {
                    shell.prompt_move_item(path, window, cx)
                }))
                .separator()
                .item(item(
                    "Delete File…",
                    &path,
                    |shell, path, window, cx| shell.confirm_delete_tree_file(path, window, cx),
                ))
            })
            .into_any_element()
    }
}

//! Project search panel integration (IDE-003) and workspace replace (IDE-004).

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ide_core::project_search::{self as project, FileList, Filter, Matcher, PlanError};

use super::*;
use crate::search_panel::{SearchPanel, SearchPanelEvent};

/// Files written by the last workspace replace, for Undo Workspace Replace.
#[derive(Default)]
pub(super) struct ReplaceUndo {
    /// Closed files: path, the text before replacing, the text written.
    files: Vec<(PathBuf, String, String)>,
    /// Open buffers that received a single undoable edit.
    buffers: Vec<BufferId>,
}

#[derive(Default)]
pub(super) struct SearchState {
    cancel: Arc<AtomicBool>,
    job: Option<Task<()>>,
    cache: Option<(String, Arc<FileList>)>,
    pub(super) undo: ReplaceUndo,
}

impl IdeShell {
    pub(super) fn search_panel_subscription(
        panel: &Entity<SearchPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(panel, window, |this, _, event, window, cx| match event {
            SearchPanelEvent::Search | SearchPanelEvent::LoadMore => this.run_project_search(cx),
            SearchPanelEvent::Open(hit) => {
                if this.blocked() {
                    return;
                }
                let location = hit.location();
                this.record_location(&location, cx);
                this.open_at(location, window, cx);
            }
            SearchPanelEvent::Replace => this.replace_in_workspace(window, cx),
        })
    }

    pub(super) fn search_project(
        &mut self,
        _: &SearchProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_sidebar = SidebarPanel::Search;
        self.show_sidebar = true;
        let root = self.workspace.root().map(Path::to_path_buf);
        let selection = {
            let editor = self.editor().read(cx);
            let range = editor.selected_byte_range();
            let text = editor.text().to_string();
            text.get(range)
                .filter(|text| !text.is_empty() && !text.contains('\n'))
                .map(str::to_string)
        };
        let roots: Vec<(PathBuf, String)> = self
            .workspace
            .root_paths()
            .into_iter()
            .map(|root| {
                let label = self.workspace.label(&root);
                (root, label)
            })
            .collect();
        self.search_panel.update(cx, |panel, cx| {
            panel.root = root;
            panel.roots = roots;
            if let Some(selection) = selection {
                panel
                    .query
                    .update(cx, |query, cx| query.set_value(selection, window, cx));
            }
            panel.focus_query(window, cx);
        });
        cx.notify();
    }

    /// Search within a folder from the explorer.
    pub(super) fn search_in_folder(
        &mut self,
        folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_project(&SearchProject, window, cx);
        self.search_panel
            .update(cx, |panel, cx| panel.set_scope(Some(folder), cx));
    }

    pub(super) fn invalidate_search_files(&mut self) {
        self.search_state.cache = None;
    }

    pub(super) fn run_project_search(&mut self, cx: &mut Context<Self>) {
        self.search_state.cancel.store(true, Ordering::Relaxed);
        self.search_state.job = None;
        let panel = self.search_panel.read(cx);
        let options = panel.options(cx);
        let limit = panel.limit;
        if options.query.is_empty() {
            self.search_panel.update(cx, |panel, cx| {
                panel.set_results(Default::default(), cx);
            });
            return;
        }
        let compiled =
            Matcher::new(&options).and_then(|matcher| Ok((matcher, Filter::new(&options)?)));
        let (matcher, filter) = match compiled {
            Ok(compiled) => compiled,
            Err(error) => {
                self.search_panel
                    .update(cx, |panel, cx| panel.set_error(Some(error), cx));
                return;
            }
        };
        // Every root included in search (IDE-017).
        let roots: Vec<PathBuf> = self
            .workspace
            .roots()
            .iter()
            .filter(|root| root.search && root.error.is_none())
            .map(|root| root.path.clone())
            .collect();
        let no_workspace = self.workspace.root().is_none();
        let open: Vec<(PathBuf, String)> = self
            .buffers
            .iter()
            .filter_map(|buffer| {
                let path = buffer.document.path()?;
                let inside = no_workspace || roots.iter().any(|root| path.starts_with(root));
                inside.then(|| (path.to_path_buf(), buffer.document.text().to_string()))
            })
            .collect();
        let cache_key = format!(
            "{:?}|{:?}|{}|{}|{}",
            roots, options.scope, options.include, options.exclude, options.use_ignore_files
        );
        let cached = self
            .search_state
            .cache
            .as_ref()
            .filter(|(key, _)| *key == cache_key)
            .map(|(_, files)| files.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_state.cancel = cancel.clone();
        self.search_panel.update(cx, |panel, cx| {
            panel.searching = true;
            panel.report = None;
            cx.notify();
        });
        let executor = cx.background_executor().clone();
        let scope = options.scope.clone();
        self.search_state.job = Some(cx.spawn(async move |this, cx| {
            let outcome = executor
                .spawn({
                    let cancel = cancel.clone();
                    async move {
                        let files = match cached {
                            Some(files) => files,
                            None => {
                                let mut all = FileList::default();
                                for root in &roots {
                                    // A folder scope limits the search to its root.
                                    let here =
                                        scope.as_deref().filter(|scope| scope.starts_with(root));
                                    if scope.is_some() && here.is_none() {
                                        continue;
                                    }
                                    let listed = project::list_files(
                                        root,
                                        &filter,
                                        here,
                                        project::DEFAULT_FILE_LIMIT,
                                        &cancel,
                                    )
                                    .map_err(|error| error.to_string())?;
                                    all.paths.extend(listed.paths);
                                    all.truncated |= listed.truncated;
                                }
                                Arc::new(all)
                            }
                        };
                        let results = project::search(
                            &matcher,
                            &files,
                            &open,
                            &filter,
                            &roots,
                            scope.as_deref(),
                            limit,
                            &cancel,
                        );
                        Ok::<_, String>((files, results))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                match outcome {
                    Ok((files, results)) => {
                        this.search_state.cache = Some((cache_key, files));
                        this.search_panel
                            .update(cx, |panel, cx| panel.set_results(results, cx));
                    }
                    Err(error) => this
                        .search_panel
                        .update(cx, |panel, cx| panel.set_error(Some(error), cx)),
                }
                cx.notify();
            });
        }));
    }

    /// Replace the selected results. Open buffers (including unsaved ones) are edited as
    /// one undoable step each and left modified; closed files are written through the
    /// normal save path. A file whose content no longer matches the preview is skipped.
    pub(super) fn replace_in_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let panel = self.search_panel.read(cx);
        let options = panel.options(cx);
        let replace = panel.replacement(cx);
        let groups = panel.selected_hits();
        let Ok(matcher) = Matcher::new(&options) else {
            return;
        };
        if groups.is_empty() {
            return;
        }
        let mut undo = ReplaceUndo::default();
        let (mut modified, mut saved, mut stale, mut failed, mut count) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0);
        for (path, hits) in groups {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let refs: Vec<_> = hits.iter().collect();
            if let Some(ix) = self.buffer_for_path(&path) {
                let source = self.buffers[ix].document.text().to_string();
                match project::plan_replacements(&matcher, &source, &refs, &replace) {
                    Ok(edits) => {
                        let (selection, reversed) = self.source_selection(ix, cx);
                        let map = |offset| ide_core::text_edit::map_offset(&edits, offset);
                        let selection = Some((map(selection.start)..map(selection.end), reversed));
                        if self.apply_source_edits(ix, &edits, selection, window, cx) {
                            count += edits.len();
                            modified.push(name);
                            undo.buffers.push(self.buffers[ix].id);
                        } else {
                            failed.push(format!("{name}: edit rejected"));
                        }
                    }
                    Err(PlanError::Stale) => stale.push(name),
                    Err(PlanError::Invalid(error)) => failed.push(format!("{name}: {error}")),
                }
                continue;
            }
            let result = Document::open(&path)
                .map_err(|error| error.to_string())
                .and_then(|mut document| {
                    let before = document.text().to_string();
                    let edits = project::plan_replacements(&matcher, &before, &refs, &replace)
                        .map_err(|error| match error {
                            PlanError::Stale => "stale".to_string(),
                            PlanError::Invalid(error) => error,
                        })?;
                    let after = ide_core::text_edit::apply(&before, &edits)
                        .map_err(|error| error.to_string())?;
                    document.set_text(after.as_str().into());
                    document.save_to(&path).map_err(|error| error.to_string())?;
                    Ok((edits.len(), before, after))
                });
            match result {
                Ok((edits, before, after)) => {
                    count += edits;
                    saved.push(name);
                    undo.files.push((path, before, after));
                }
                Err(error) if error == "stale" => stale.push(name),
                Err(error) => failed.push(format!("{name}: {error}")),
            }
        }
        self.search_state.undo = undo;
        let mut report = format!(
            "Replaced {count} match{}",
            if count == 1 { "" } else { "es" }
        );
        if !modified.is_empty() {
            report.push_str(&format!(
                "; modified (unsaved) open files: {}",
                modified.join(", ")
            ));
        }
        if !saved.is_empty() {
            report.push_str(&format!("; saved: {}", saved.join(", ")));
        }
        if !stale.is_empty() {
            report.push_str(&format!(
                "; skipped, changed since the preview: {}",
                stale.join(", ")
            ));
        }
        if !failed.is_empty() {
            report.push_str(&format!("; failed: {}", failed.join("; ")));
        }
        self.status = report.clone();
        if !saved.is_empty() {
            self.invalidate_search_files();
            self.refresh_git(cx);
        }
        self.run_project_search(cx);
        self.search_panel.update(cx, |panel, cx| {
            panel.report = Some(report);
            cx.notify();
        });
        cx.notify();
    }

    /// Undo the last workspace replace: open buffers undo their edit, closed files are
    /// restored only if they still contain exactly what the replace wrote.
    pub(super) fn undo_workspace_replace(
        &mut self,
        _: &UndoWorkspaceReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let undo = std::mem::take(&mut self.search_state.undo);
        if undo.files.is_empty() && undo.buffers.is_empty() {
            self.status = "Nothing to undo".into();
            cx.notify();
            return;
        }
        let mut restored = 0;
        let mut skipped = Vec::new();
        for id in undo.buffers {
            if let Some(ix) = self.buffer_index(id) {
                self.buffers[ix]
                    .editor
                    .update(cx, |editor, cx| editor.undo_change(window, cx));
                restored += 1;
            }
        }
        for (path, before, after) in undo.files {
            let result = Document::open(&path).and_then(|mut document| {
                if *document.text() != after.as_str() {
                    return Err(std::io::Error::other("changed since the replace"));
                }
                document.set_text(before.as_str().into());
                document.save_to(&path).map(|_| ())
            });
            match result {
                Ok(()) => restored += 1,
                Err(error) => skipped.push(format!(
                    "{}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        self.status = if skipped.is_empty() {
            format!("Restored {restored} file(s)")
        } else {
            format!(
                "Restored {restored} file(s); not restored: {}",
                skipped.join("; ")
            )
        };
        self.check_disk_changes(window, cx);
        self.run_project_search(cx);
        cx.notify();
    }
}

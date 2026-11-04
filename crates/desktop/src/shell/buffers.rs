//! Buffers: creating, activating, opening and removing documents.

use super::*;

impl IdeShell {
    pub(super) fn buffer(&self) -> &Buffer {
        &self.buffers[self.active]
    }

    pub(super) fn document(&self) -> &Document {
        &self.buffer().document
    }

    pub(super) fn editor(&self) -> &Entity<InputState> {
        &self.buffer().editor
    }

    pub(super) fn buffer_index(&self, id: BufferId) -> Option<usize> {
        self.buffers.iter().position(|buffer| buffer.id == id)
    }

    pub(super) fn buffer_for_path(&self, path: &Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|buffer| buffer.document.path() == Some(path))
    }

    pub(super) fn new_buffer(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Buffer {
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let language = syntax::detect(document.path(), Some(&first_line(&document)));
        let preferences = Self::resolve_preferences(&document, language, cx);
        let group = self.groups.focused_id();
        let view = self.create_view(
            id,
            document.text().to_string(),
            language,
            &preferences,
            document.path().map(Path::to_path_buf),
            group,
            window,
            cx,
        );
        let editor = view.editor.clone();
        Buffer {
            id,
            document,
            language,
            language_overridden: false,
            preferences,
            editor,
            dirty: false,
            folds: Vec::new(),
            external_change: false,
            deleted_on_disk: false,
            recovered: false,
            title_override: None,
            views: vec![view],
        }
    }

    pub(super) fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.buffers.get(self.active).map(|buffer| buffer.id)
            && self
                .buffers
                .get(ix)
                .is_some_and(|buffer| buffer.id != previous)
        {
            self.auto_save_on_focus_change(Some(previous), cx);
        }
        self.active = ix;
        self.ensure_focused_view(ix, window, cx);
        self.dismiss_signature_help();
        if let Some(path) = self.document().path().map(Path::to_path_buf) {
            self.recent_paths.retain(|recent| recent != &path);
            self.recent_paths.insert(0, path);
            self.recent_paths.truncate(100);
        }
        self.show_diff = false;
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        let indent = self.buffer().preferences.tab_width;
        if let Some(vim) = &mut self.vim {
            vim.set_indent_width(indent);
            vim.reset(&editor, cx);
        }
        self.sync_git(cx);
        self.sync_servers(cx);
        self.sync_lsp_buffer(ix, cx);
        if self.show_sidebar && self.active_sidebar == SidebarPanel::Outline {
            self.refresh_outline(window, cx);
        }
        cx.notify();
    }

    pub(super) fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor()
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub(super) fn open_document(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = document.path().and_then(|path| self.buffer_for_path(path)) {
            self.activate(ix, window, cx);
            return;
        }
        let buffer = self.new_buffer(document, window, cx);
        if self.buffer().is_pristine() {
            self.buffers[self.active] = buffer;
        } else {
            self.active += 1;
            self.buffers.insert(self.active, buffer);
        }
        self.activate(self.active, window, cx);
    }

    pub(super) fn remove_buffer(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.buffers[ix].document.path().map(Path::to_path_buf) {
            self.diagnostics.remove(&path);
            self.servers.update(cx, |servers, _| servers.close(&path));
        }
        self.buffers.remove(ix);
        self.write_recovery_snapshots(cx);
        let tabs = self.focused_tabs();
        if tabs.is_empty() && self.groups.list.len() > 1 {
            if self.active >= self.buffers.len() || self.active > ix {
                self.active = self.active.saturating_sub(1);
            }
            self.remove_group(self.groups.focused_id(), window, cx);
            return;
        }
        if self.buffers.is_empty() {
            let buffer = self.new_buffer(Document::default(), window, cx);
            self.buffers.push(buffer);
        }
        let tabs = self.focused_tabs();
        let active = tabs
            .iter()
            .rev()
            .find(|tab| **tab < ix)
            .or(tabs.first())
            .copied()
            .unwrap_or_else(|| self.buffers.len() - 1);
        let show_diff = self.show_diff;
        self.activate(active, window, cx);
        if show_diff {
            self.focus_diff(window, cx);
        }
    }

    pub(super) fn open(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = path
            .as_deref()
            .filter(|path| Self::is_workspace_definition(path))
        {
            if let Err(error) = self.open_workspace_definition(path, cx) {
                self.status = format!("Could not open the workspace: {error}");
            }
            cx.notify();
            return;
        }
        if let Some(ix) = path.as_deref().and_then(|path| self.buffer_for_path(path)) {
            self.activate(ix, window, cx);
            return;
        }
        self.busy = true;
        let picker = if path.is_none() {
            self.status = "Choose a UTF-8 text file…".into();
            Some(cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open file".into()),
            }))
        } else {
            self.status = "Opening…".into();
            None
        };
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let path = if let Some(picker) = picker {
                    picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                        .and_then(|paths| paths.into_iter().next())
                } else {
                    path
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                if Self::is_workspace_definition(&path) {
                    return Err(format!("{OPEN_WORKSPACE_MARKER}{}", path.display()));
                }
                executor
                    .spawn(async move {
                        Document::open(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(document)) => {
                        let opened_path = document.path().map(Path::to_path_buf);
                        this.status = "Ready".into();
                        this.open_document(document, window, cx);
                        if this
                            .pending_hit
                            .as_ref()
                            .is_some_and(|hit| Some(&hit.path) == opened_path.as_ref())
                        {
                            let hit = this.pending_hit.take().unwrap();
                            this.jump_to_hit(&hit, window, cx);
                        }
                    }
                    Ok(None) => this.status = "Open cancelled".into(),
                    Err(error) if error.starts_with(OPEN_WORKSPACE_MARKER) => {
                        let path = PathBuf::from(&error[OPEN_WORKSPACE_MARKER.len()..]);
                        if let Err(error) = this.open_workspace_definition(&path, cx) {
                            this.status = format!("Could not open the workspace: {error}");
                        }
                    }
                    Err(error) => {
                        this.status = format!("Open failed: {error}");
                        if error.contains("Reopen with Encoding")
                            && let Some(path) = error.split(": ").next().map(PathBuf::from)
                        {
                            this.offer_encodings_for(path, window, cx);
                            cx.notify();
                            return;
                        }
                    }
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked() {
            return;
        }
        let buffer = self.new_buffer(Document::default(), window, cx);
        self.active += 1;
        self.buffers.insert(self.active, buffer);
        self.activate(self.active, window, cx);
        self.status = "New file".into();
    }

    pub(super) fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        if !self.blocked() {
            self.open(None, window, cx);
        }
    }
}

//! Saving buffers (Save, Save As, Save All).

use super::*;

impl IdeShell {
    /// Save, running the language server's formatter first when format-on-save is on.
    /// The save waits for the formatting result (up to a timeout); a failed, timed-out or
    /// stale formatting result is skipped and the current text is saved.
    pub(super) fn save(
        &mut self,
        target: SaveTarget,
        then: Option<PendingAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let wants_format = matches!(target, SaveTarget::Current)
            && self.buffer().preferences.format_on_save
            && self.document().path().is_some();
        let request = if wants_format {
            self.formatting_request(false, true, cx)
        } else {
            None
        };
        let Some((format_target, request)) = request else {
            return self.save_now(target, then, window, cx);
        };
        self.busy = true;
        self.status = "Formatting before save…".into();
        cx.notify();
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(3));
        cx.spawn_in(window, async move |this, cx| {
            let response = futures_lite::future::or(async { Some(request.await) }, async {
                timer.await;
                None
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match response {
                    Some(response) => {
                        this.apply_formatting(&format_target, response, window, cx);
                    }
                    None => this.status = "Formatting timed out; saving without it".into(),
                }
                this.save_now(target, then, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn save_now(
        &mut self,
        target: SaveTarget,
        then: Option<PendingAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let new_file = self.document().path().is_none() || !matches!(target, SaveTarget::Current);
        self.apply_save_policies(self.active, new_file, window, cx);
        let buffer = self.buffer();
        let id = buffer.id;
        let existing_path = match target {
            SaveTarget::Current => buffer.document.path().map(PathBuf::from),
            SaveTarget::Ask => None,
            SaveTarget::Path(path) => Some(path),
        };
        let picker = if existing_path.is_none() {
            let directory = buffer
                .document
                .path()
                .and_then(|path| path.parent())
                .or(self.workspace.root())
                .unwrap_or(Path::new("."));
            Some(cx.prompt_for_new_path(directory, Some(&buffer.document.name())))
        } else {
            None
        };
        let snapshot = buffer.document.clone();
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let path = if let Some(picker) = picker {
                    picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                } else {
                    existing_path
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        snapshot
                            .save_to(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(saved)) => {
                        let saved_path = saved.path().map(Path::to_path_buf);
                        if let Some(ix) = this.buffer_index(id) {
                            let buffer = &mut this.buffers[ix];
                            if let Some(old_path) = buffer.document.path().map(Path::to_path_buf)
                                && saved_path.as_ref() != Some(&old_path)
                            {
                                this.servers
                                    .update(cx, |servers, _| servers.close(&old_path));
                            }
                            let language = buffer.language;
                            buffer.document.accept_saved(saved);
                            buffer.dirty = buffer.document.is_dirty();
                            buffer.external_change = false;
                            this.status = format!("Saved {}", buffer.document.name());
                            let saved_language = if buffer.language_overridden {
                                language
                            } else {
                                syntax::detect(
                                    buffer.document.path(),
                                    Some(&first_line(&buffer.document)),
                                )
                            };
                            buffer.language = saved_language;
                            let preferences =
                                Self::resolve_preferences(&buffer.document, saved_language, cx);
                            Self::apply_preferences(&buffer.editor, &preferences, cx);
                            buffer.preferences = preferences;
                            if saved_language != language {
                                buffer.editor.update(cx, |editor, cx| {
                                    editor.set_highlighter(saved_language, cx)
                                });
                            }
                            let editor = buffer.editor.clone();
                            language_server::attach(
                                &editor,
                                &this.servers,
                                saved_path.clone(),
                                saved_language,
                                cx,
                            );
                            this.sync_lsp_buffer(ix, cx);
                            if let Some(path) = saved_path.as_deref() {
                                let text = this.buffers[ix].document.text().to_string();
                                this.servers
                                    .update(cx, |servers, _| servers.saved(path, &text));
                            }
                        }
                        this.refresh_parent(saved_path.as_deref(), cx);
                        this.write_recovery_snapshots(cx);
                        let test_source = saved_path.as_deref().is_some_and(|path| {
                            path.extension().is_some_and(|extension| extension == "rs")
                                || path.file_name().is_some_and(|name| name == "Cargo.toml")
                        });
                        if test_source
                            && this.show_sidebar
                            && this.active_sidebar == SidebarPanel::Testing
                            && this.testing.run.is_none()
                        {
                            this.discover_tests(cx);
                        }
                        this.refresh_git(cx);
                        if saved_path
                            .as_deref()
                            .is_some_and(|path| settings::is_settings_file(path, cx))
                        {
                            this.status = this.apply_settings(window, cx);
                        }
                        let clean = this
                            .buffer_index(id)
                            .is_some_and(|ix| !this.buffers[ix].dirty);
                        if clean && let Some(action) = then.or_else(|| this.pending.take()) {
                            this.perform(action, window, cx);
                        }
                    }
                    Ok(None) => this.status = "Save cancelled".into(),
                    Err(error) => {
                        if error.contains("File changed on disk")
                            && let Some(ix) = this.buffer_index(id)
                        {
                            this.buffers[ix].external_change = true;
                        }
                        this.status = format!("Save failed: {error}");
                    }
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_all(
        &mut self,
        then: Option<PendingAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        for ix in 0..self.buffers.len() {
            if self.buffers[ix].dirty && self.buffers[ix].document.path().is_some() {
                self.apply_save_policies(ix, false, window, cx);
            }
        }
        let snapshots: Vec<_> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.dirty)
            .filter_map(|buffer| {
                let path = buffer.document.path()?.to_path_buf();
                Some((buffer.id, buffer.document.clone(), path))
            })
            .collect();
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let results = executor
                .spawn(async move {
                    snapshots
                        .into_iter()
                        .map(|(id, snapshot, path)| {
                            let result = snapshot
                                .save_to(&path)
                                .map_err(|error| format!("{}: {error}", path.display()));
                            (id, result)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                let mut failures = Vec::new();
                let mut settings_saved = false;
                let saved = results.iter().filter(|(_, result)| result.is_ok()).count();
                for (id, result) in results {
                    match result {
                        Ok(saved) => {
                            let saved_path = saved.path().map(Path::to_path_buf);
                            settings_saved |= saved
                                .path()
                                .is_some_and(|path| settings::is_settings_file(path, cx));
                            if let Some(ix) = this.buffer_index(id) {
                                let buffer = &mut this.buffers[ix];
                                buffer.document.accept_saved(saved);
                                buffer.dirty = buffer.document.is_dirty();
                                buffer.external_change = false;
                                if let Some(path) = saved_path.as_deref() {
                                    let text = this.buffers[ix].document.text().to_string();
                                    this.servers
                                        .update(cx, |servers, _| servers.saved(path, &text));
                                }
                            }
                        }
                        Err(error) => failures.push(error),
                    }
                }
                if saved > 0 {
                    this.refresh_git(cx);
                    this.write_recovery_snapshots(cx);
                }
                let settings_status = settings_saved.then(|| this.apply_settings(window, cx));
                match failures.first() {
                    Some(error) => this.status = format!("Save failed: {error}"),
                    None => {
                        let plural = if saved == 1 { "" } else { "s" };
                        this.status = settings_status
                            .unwrap_or_else(|| format!("Saved {saved} file{plural}"));
                        if let Some(note) = this.save_all_note.take() {
                            this.status = format!("{} · {note}", this.status);
                        }
                        if let Some(action) = then {
                            this.perform(action, window, cx);
                        }
                        this.resume_pending_switch(window, cx);
                    }
                }
                this.focus_editor(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_parent(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        if let Some(parent) = path.and_then(Path::parent)
            && self.workspace.is_expanded(parent)
        {
            self.load_directory(parent.to_path_buf(), cx);
        }
    }

    pub(super) fn save_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        self.save(SaveTarget::Current, None, window, cx);
    }

    pub(super) fn save_file_as(
        &mut self,
        _: &SaveFileAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save(SaveTarget::Ask, None, window, cx);
    }
}

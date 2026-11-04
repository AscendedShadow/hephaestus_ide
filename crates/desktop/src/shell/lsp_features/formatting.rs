//! Formatting documents and selections.

use super::*;

impl IdeShell {
    pub(in crate::shell) fn formatting_options(&self, ix: usize) -> FormattingOptions {
        let preferences = &self.buffers[ix].preferences;
        FormattingOptions {
            tab_size: preferences.tab_width as u32,
            insert_spaces: preferences.insert_spaces,
            properties: HashMap::new(),
            trim_trailing_whitespace: Some(preferences.trim_trailing_whitespace),
            insert_final_newline: Some(preferences.insert_final_newline),
            trim_final_newlines: None,
        }
    }

    /// Start a formatting request for the active buffer. Returns the request future and
    /// its target, or `None` (with a status message) when formatting is unavailable.
    pub(in crate::shell) fn formatting_request(
        &mut self,
        selection_only: bool,
        quiet: bool,
        cx: &mut Context<Self>,
    ) -> Option<(Target, FormattingFuture)> {
        let target = if quiet {
            let ix = self.active;
            let path = self.buffers[ix].document.path()?.to_path_buf();
            let servers = self.servers.read(cx);
            servers.server_capabilities(self.buffers[ix].language)?;
            Target {
                ix,
                id: self.buffers[ix].id,
                language: self.buffers[ix].language,
                position: Default::default(),
                origin: servers.origin(&path),
                features: servers.features(self.buffers[ix].language),
                path,
            }
        } else {
            self.lsp_target(cx)?
        };
        let document = Servers::text_document(&target.path)?;
        let options = self.formatting_options(target.ix);
        let servers = self.servers.read(cx);
        if selection_only {
            if !target.features.range_formatting {
                if !quiet {
                    self.unsupported("Format Selection", &target, cx);
                }
                return None;
            }
            let range = self.source_range(target.ix, cx);
            let request = servers.send::<RangeFormatting>(
                target.language,
                DocumentRangeFormattingParams {
                    text_document: document,
                    range,
                    options,
                    work_done_progress_params: Default::default(),
                },
            )?;
            Some((target, Box::pin(request)))
        } else {
            if !target.features.formatting {
                if !quiet {
                    self.unsupported("Format Document", &target, cx);
                }
                return None;
            }
            let request = servers.send::<Formatting>(
                target.language,
                DocumentFormattingParams {
                    text_document: document,
                    options,
                    work_done_progress_params: Default::default(),
                },
            )?;
            Some((target, Box::pin(request)))
        }
    }

    /// Apply formatting edits if the document is unchanged since the request.
    pub(in crate::shell) fn apply_formatting(
        &mut self,
        target: &Target,
        response: Result<Option<Vec<lsp_types::TextEdit>>, lsp::Error>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.still_current(target, "Formatting", cx) {
            return false;
        }
        let edits = match response {
            Ok(edits) => edits.unwrap_or_default(),
            Err(error) => {
                self.status = format!("Formatting failed: {error}");
                cx.notify();
                return false;
            }
        };
        if edits.is_empty() {
            self.status = "Already formatted".into();
            cx.notify();
            return true;
        }
        let Some(uri) = lsp::path_to_uri(&target.path) else {
            return false;
        };
        let edit = WorkspaceEdit {
            changes: Some([(uri, edits)].into_iter().collect()),
            ..Default::default()
        };
        match self
            .plan_workspace_edit(&edit, cx)
            .and_then(|plan| self.apply_plan(&plan, "Format", window, cx))
        {
            Ok(_) => {
                self.status = "Formatted".into();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = format!("Formatting not applied: {error}");
                cx.notify();
                false
            }
        }
    }

    pub(in crate::shell) fn format_document(
        &mut self,
        _: &FormatDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format(false, window, cx);
    }

    pub(in crate::shell) fn format_selection(
        &mut self,
        _: &FormatSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format(true, window, cx);
    }

    pub(in crate::shell) fn format(
        &mut self,
        selection_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, request)) = self.formatting_request(selection_only, false, cx) else {
            return;
        };
        self.status = "Formatting…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_formatting(&target, response, window, cx);
            });
        });
    }
}

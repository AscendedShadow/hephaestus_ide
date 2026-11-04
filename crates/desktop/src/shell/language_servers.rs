//! Keeping language servers in sync with the workspace and buffers.

use super::*;

impl IdeShell {
    pub(super) fn restart_servers(&mut self, cx: &mut Context<Self>) {
        self.servers.update(cx, |servers, cx| servers.stop(cx));
        self.diagnostics.clear();
        self.sync_servers(cx);
        for ix in 0..self.buffers.len() {
            self.sync_lsp_buffer(ix, cx);
        }
        self.status = "Restarting the language server…".into();
        cx.notify();
    }

    /// Language servers start in the primary root and get every root as a workspace
    /// folder.
    pub(super) fn sync_servers(&mut self, cx: &mut Context<Self>) {
        let root = self
            .workspace
            .root()
            .or_else(|| self.document().path().and_then(Path::parent))
            .map(Path::to_path_buf);
        let folders = self.workspace.root_paths();
        let configs = settings::language_servers(cx);
        self.servers.update(cx, |servers, cx| {
            servers.set_configs(configs, cx);
            servers.set_root(root, cx);
            servers.set_folders(folders, cx);
        });
    }

    pub(super) fn sync_lsp_buffer(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(buffer) = self.buffers.get(ix) else {
            return;
        };
        let language = buffer.language;
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            return;
        };
        let text = buffer.document.text().clone();
        self.servers
            .update(cx, |servers, cx| servers.sync(&path, language, &text, cx));
    }

    pub(super) fn handle_server_event(
        &mut self,
        event: &language_server::ServerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            language_server::ServerEvent::Ready => {
                for ix in 0..self.buffers.len() {
                    self.sync_lsp_buffer(ix, cx);
                }
            }
            language_server::ServerEvent::Status(status) => self.status = status.clone(),
            language_server::ServerEvent::ApplyEdit { server, id, params } => {
                self.server_apply_edit(server.clone(), id.clone(), params.clone(), window, cx)
            }
            language_server::ServerEvent::Diagnostics { path, diagnostics } => {
                self.apply_diagnostics(path, diagnostics, cx)
            }
        }
        cx.notify();
    }

    pub(super) fn apply_diagnostics(
        &mut self,
        path: &Path,
        diagnostics: &[lsp_types::Diagnostic],
        cx: &mut Context<Self>,
    ) {
        let hits = diagnostics
            .iter()
            .map(|diagnostic| SearchMatch {
                path: path.to_path_buf(),
                line: diagnostic.range.start.line as usize + 1,
                column: diagnostic.range.start.character as usize + 1,
                preview: diagnostic
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(160)
                    .collect(),
            })
            .collect();
        self.diagnostics.insert(path.to_path_buf(), hits);
        self.lsp_diagnostics
            .insert(path.to_path_buf(), diagnostics.to_vec());
        let Some(ix) = self.buffer_for_path(path) else {
            return;
        };
        if !self.buffers[ix].folds.is_empty() {
            return;
        }
        let mut sorted = diagnostics.to_vec();
        sorted.sort_by_key(|diagnostic| {
            (
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            )
        });
        self.buffers[ix].editor.update(cx, |editor, cx| {
            let text = editor.text().clone();
            if let Some(set) = editor.diagnostics_mut() {
                set.reset(&text);
                set.extend(sorted);
            }
            cx.notify();
        });
    }
}

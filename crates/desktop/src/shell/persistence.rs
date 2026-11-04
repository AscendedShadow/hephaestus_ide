//! Saving and restoring the session (open files, layout, workspace).

use super::*;

impl IdeShell {
    pub fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = settings::path(cx).map(|path| session::path(&path)) else {
            return;
        };
        let snapshot = session::load(&path);
        self.restore_roots(&snapshot, cx);
        for (ix, file) in snapshot.files.iter().enumerate() {
            if let Ok(document) = Document::open(file) {
                self.open_document(document, window, cx);
                if let Some((line, column)) = snapshot.cursors.get(ix).copied() {
                    self.editor().update(cx, |editor, cx| {
                        editor.set_cursor_position(Position::new(line, column), window, cx)
                    });
                }
            }
        }
        if snapshot.active < self.buffers.len() {
            self.activate(snapshot.active, window, cx);
        }
        if snapshot.groups.len() > 1 {
            self.restore_groups(
                &snapshot.files,
                &snapshot.groups,
                snapshot.focused_group,
                window,
                cx,
            );
        }
        self.recent_paths = snapshot
            .recent
            .into_iter()
            .filter(|path| path.is_file())
            .take(100)
            .collect();
        self.restore_recovery(window, cx);
    }

    pub(super) fn save_session(&self, cx: &App) {
        let Some(path) = settings::path(cx).map(|path| session::path(&path)) else {
            return;
        };
        let snapshot = Session {
            folder: self.workspace.root().map(Path::to_path_buf),
            files: self
                .buffers
                .iter()
                .filter_map(|buffer| buffer.document.path().map(Path::to_path_buf))
                .collect(),
            active: self.buffers[..self.active]
                .iter()
                .filter(|buffer| buffer.document.path().is_some())
                .count(),
            cursors: self
                .buffers
                .iter()
                .filter(|buffer| buffer.document.path().is_some())
                .map(|buffer| {
                    let cursor = buffer.editor.read(cx).cursor_position();
                    (cursor.line, cursor.character)
                })
                .collect(),
            recent: self.recent_paths.clone(),
            groups: self.session_groups(cx),
            focused_group: self.groups.focused,
            ..self.session_roots()
        };
        let _ = session::save(&path, &snapshot);
    }
}

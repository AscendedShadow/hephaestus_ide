//! Workspace/user settings commands (IDE-016).

use super::*;
use crate::settings::layers;

impl IdeShell {
    pub(super) fn open_workspace_settings(
        &mut self,
        _: &OpenWorkspaceSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(root) = self.workspace.root().map(Path::to_path_buf) else {
            self.status = "Open a folder to create workspace settings".into();
            cx.notify();
            return;
        };
        let path = layers::workspace_path(&root);
        if !path.exists() {
            let created = path
                .parent()
                .map(fs::create_dir_all)
                .unwrap_or(Ok(()))
                .and_then(|_| fs::write(&path, "{\n}\n"));
            if let Err(error) = created {
                self.status = format!("Could not create {}: {error}", path.display());
                cx.notify();
                return;
            }
            self.refresh_parent(Some(&path), cx);
        }
        self.open(Some(path), window, cx);
    }

    pub(super) fn open_user_settings(
        &mut self,
        _: &OpenUserSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_settings(&EditSettings, window, cx);
    }

    pub(super) fn show_effective_settings(
        &mut self,
        _: &ShowEffectiveSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(state) = settings::state(cx) else {
            return;
        };
        let text = layers::describe(
            &state.resolved,
            settings::path(cx).as_deref(),
            settings::workspace_path(cx).as_deref(),
        );
        let buffer = self.new_buffer(Document::from_text(&text), window, cx);
        self.active += 1;
        self.buffers
            .insert(self.active.min(self.buffers.len()), buffer);
        self.active = self.active.min(self.buffers.len() - 1);
        self.activate(self.active, window, cx);
        self.set_buffer_language(self.active, "jsonc", cx);
        self.status = "Effective settings — sources are listed at the top".into();
        cx.notify();
    }
}

//! Vim mode: toggling it and routing keys through the Vim layer.

use super::*;

impl IdeShell {
    pub(super) fn toggle_vim_mode(
        &mut self,
        _: &ToggleVimMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_vim_mode(self.vim.is_none(), cx);
    }

    pub(super) fn set_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.vim.is_some() {
            return;
        }
        self.vim = enabled.then(|| {
            let mut vim = VimInput::new(self.buffer().preferences.tab_width);
            vim.reset(self.editor(), cx);
            vim
        });
        self.status = if enabled {
            "Vim keys on"
        } else {
            "Vim keys off"
        }
        .into();
        cx.notify();
    }

    pub(super) fn editor_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor().clone();
        if self.blocked() || !editor.focus_handle(cx).is_focused(window) {
            return;
        }
        self.snippet_key_down(event);
        if event.keystroke.key == "escape" && self.signature.help.is_some() {
            self.dismiss_signature_help();
            cx.notify();
        }
        let Some(vim) = &mut self.vim else {
            return;
        };
        let Some(response) = vim.key_down(&event.keystroke, &editor, window, cx) else {
            return;
        };
        cx.stop_propagation();
        if let Some(message) = response.message {
            self.status = message;
        }
        self.run_vim_commands(response.commands, window, cx);
        cx.notify();
    }

    pub(super) fn run_vim_commands(
        &mut self,
        commands: Vec<VimCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut commands = commands.into_iter().peekable();
        while let Some(command) = commands.next() {
            match command {
                VimCommand::Write | VimCommand::WriteAs(_) => {
                    let then = commands
                        .next_if(|next| matches!(next, VimCommand::Close { .. }))
                        .map(|_| PendingAction::CloseBuffer(self.buffer().id));
                    let target = match command {
                        VimCommand::WriteAs(path) => SaveTarget::Path(self.resolve(&path)),
                        _ => SaveTarget::Current,
                    };
                    self.save(target, then, window, cx);
                }
                VimCommand::WriteAll => {
                    let then = commands
                        .next_if(|next| matches!(next, VimCommand::CloseAll { .. }))
                        .map(|_| PendingAction::CloseWindow);
                    self.save_all(then, window, cx);
                }
                VimCommand::Close { force: true } => self.remove_buffer(self.active, window, cx),
                VimCommand::Close { force: false } => {
                    self.request(PendingAction::CloseBuffer(self.buffer().id), window, cx)
                }
                VimCommand::CloseAll { force: true } => window.remove_window(),
                VimCommand::CloseAll { force: false } => {
                    self.request(PendingAction::CloseWindow, window, cx)
                }
                VimCommand::NextBuffer => self.cycle_tab(1, window, cx),
                VimCommand::PreviousBuffer => self.cycle_tab(-1, window, cx),
                VimCommand::NewBuffer => self.new_file(&NewFile, window, cx),
                VimCommand::Open(path) => {
                    let path = self.resolve(&path);
                    self.open(Some(path), window, cx);
                }
                VimCommand::Undo | VimCommand::Redo => {}
            }
        }
    }
}

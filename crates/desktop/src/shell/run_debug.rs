//! Run configurations, the debugger session and breakpoint actions.

use super::*;

impl IdeShell {
    pub(super) fn config_directory(
        &mut self,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Option<Option<PathBuf>> {
        let directory = directory
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    self.resolve(&path.to_string_lossy())
                }
            })
            .or_else(|| self.project_directory().map(Path::to_path_buf));
        if directory.as_ref().is_some_and(|path| !path.is_dir()) {
            self.status = "Command working directory does not exist".into();
            cx.notify();
            return None;
        }
        Some(directory)
    }

    pub(super) fn launch_config(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(settings) = self.load_settings(cx) else {
            return;
        };
        let Some(config) = settings
            .commands
            .into_iter()
            .nth(index)
            .filter(|config| !config.program.trim().is_empty())
        else {
            self.status = "Configure commands in settings.json (program and args)".into();
            cx.notify();
            return;
        };
        let Some(directory) = self.config_directory(config.working_directory.clone(), cx) else {
            return;
        };
        let result = self.task_terminal.update(cx, |terminal, cx| {
            terminal.run(
                terminal::Options {
                    shell: Some((config.program, config.args)),
                    working_directory: directory.clone(),
                    env: config.env,
                    ..Default::default()
                },
                cx,
            )
        });
        if let Err(error) = result {
            self.status = error;
            cx.notify();
            return;
        }
        self.run_hits.clear();
        self.command_directory = directory;
        self.active_panel = ToolPanel::Run;
        window.focus(&self.task_terminal.focus_handle(cx));
        self.status = format!(
            "Started {}",
            if config.name.is_empty() {
                "command"
            } else {
                &config.name
            }
        );
        cx.notify();
    }

    pub(super) fn run_command(
        &mut self,
        _: &RunCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_commands = self
            .load_settings(cx)
            .map(|settings| settings.commands)
            .unwrap_or_default();
        if self.task_commands.len() > 1 {
            self.show_search(SearchMode::Tasks, window, cx);
        } else {
            self.launch_config(0, window, cx);
        }
    }

    pub(super) fn start_debugger(
        &mut self,
        _: &StartDebugger,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_panel = ToolPanel::Debug;
        if self.debugger.read(cx).is_paused() {
            self.debug_result(|debugger, cx| debugger.resume(cx), cx);
            return;
        }
        if self.debugger.read(cx).is_active() {
            self.status = "The debugger is already running".into();
            cx.notify();
            return;
        }
        self.start_debugging(window, cx);
    }

    pub(super) fn stop_command(&mut self, _: &StopCommand, _: &mut Window, cx: &mut Context<Self>) {
        self.task_terminal
            .update(cx, |terminal, cx| terminal.terminate(cx));
        self.status = "Stopped process".into();
        cx.notify();
    }

    pub(super) fn debug_result(
        &mut self,
        command: impl FnOnce(&mut Debugger, &mut Context<Debugger>) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.debugger.update(cx, command) {
            self.status = error;
        }
        cx.notify();
    }

    pub(super) fn debug_continue(
        &mut self,
        _: &DebugContinue,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debug_result(|debugger, cx| debugger.resume(cx), cx);
    }

    pub(super) fn debug_step_over(
        &mut self,
        _: &DebugStepOver,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debug_result(|debugger, cx| debugger.step(Step::Over, cx), cx);
    }

    pub(super) fn debug_step_into(
        &mut self,
        _: &DebugStepInto,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debug_result(|debugger, cx| debugger.step(Step::Into, cx), cx);
    }

    pub(super) fn debug_step_out(
        &mut self,
        _: &DebugStepOut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debug_result(|debugger, cx| debugger.step(Step::Out, cx), cx);
    }

    pub(super) fn debug_interrupt(
        &mut self,
        _: &DebugInterrupt,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debug_result(|debugger, cx| debugger.pause(cx), cx);
    }

    pub(super) fn debug_stop(&mut self, _: &DebugStop, _: &mut Window, cx: &mut Context<Self>) {
        self.debugger.update(cx, |debugger, cx| debugger.stop(cx));
        self.status = "Stopped debugging".into();
        cx.notify();
    }

    pub(super) fn debug_restart(
        &mut self,
        _: &DebugRestart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_panel = ToolPanel::Debug;
        let result = self
            .debugger
            .update(cx, |debugger, cx| debugger.restart(cx));
        self.status = match result {
            Ok(name) => format!("Restarted {name}"),
            Err(error) => error,
        };
        cx.notify();
    }

    pub(super) fn toggle_breakpoint(
        &mut self,
        _: &ToggleBreakpoint,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.editor().read(cx).cursor_position().line as usize;
        self.toggle_breakpoint_at_row(row, cx);
    }

    pub(super) fn source_line(&self, display_row: usize, cx: &App) -> Option<usize> {
        let buffer = self.buffer();
        let display = buffer.editor.read(cx).text();
        if display_row >= display.lines_len() {
            return None;
        }
        let offset = folding::display_to_source(
            &buffer.folds,
            display.line_start_offset(display_row),
            false,
        );
        Some(buffer.document.text().offset_to_point(offset).row + 1)
    }

    pub(super) fn display_row(&self, source_line: usize, cx: &App) -> usize {
        let buffer = self.buffer();
        let source = buffer.document.text();
        let offset = source.line_start_offset(source_line.saturating_sub(1));
        let display = folding::source_to_display(&buffer.folds, offset);
        buffer.editor.read(cx).text().offset_to_point(display).row
    }

    pub(super) fn toggle_breakpoint_at_row(&mut self, display_row: usize, cx: &mut Context<Self>) {
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            self.status = "Save the file before setting a breakpoint".into();
            cx.notify();
            return;
        };
        let Some(line) = self.source_line(display_row, cx) else {
            return;
        };
        let enabled = self.debugger.update(cx, |debugger, cx| {
            debugger.toggle_breakpoint(path.clone(), line, cx)
        });
        self.status = format!(
            "Breakpoint {} at {}:{line}",
            if enabled { "set" } else { "removed" },
            path.display()
        );
        cx.notify();
    }

    pub(super) fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        match self.project_directory() {
            Some(base) if path.is_relative() => base.join(path),
            _ => path.to_path_buf(),
        }
    }
}

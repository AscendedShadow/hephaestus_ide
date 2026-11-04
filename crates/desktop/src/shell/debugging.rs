//! Debug profiles and attach (IDE-021), breakpoint options (IDE-022), watches (IDE-023)
//! and per-workspace persistence of debugger state.

use std::process::Command as Process;

use crate::{
    debugger::{self, BreakpointOptions, PersistedState},
    settings::{DebugRequest, DebuggerConfig},
};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;

/// Running processes as `(pid, name)`, best effort per platform.
pub(super) fn list_processes() -> Vec<(u32, String)> {
    let mut processes = Vec::new();
    if cfg!(target_os = "linux")
        && let Ok(entries) = fs::read_dir("/proc")
    {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let command = fs::read(entry.path().join("cmdline"))
                .ok()
                .map(|bytes| {
                    bytes
                        .split(|byte| *byte == 0)
                        .filter(|part| !part.is_empty())
                        .map(|part| String::from_utf8_lossy(part).into_owned())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|command| !command.is_empty())
                .or_else(|| {
                    fs::read_to_string(entry.path().join("comm"))
                        .ok()
                        .map(|name| name.trim().to_string())
                });
            if let Some(command) = command {
                processes.push((pid, command));
            }
        }
    } else if cfg!(target_os = "windows") {
        if let Ok(output) = Process::new("tasklist")
            .args(["/fo", "csv", "/nh"])
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let fields: Vec<&str> = line
                    .split("\",\"")
                    .map(|field| field.trim_matches('"'))
                    .collect();
                if let (Some(name), Some(pid)) = (
                    fields.first(),
                    fields.get(1).and_then(|pid| pid.parse().ok()),
                ) {
                    processes.push((pid, name.to_string()));
                }
            }
        }
    } else if let Ok(output) = Process::new("ps").args(["-axo", "pid=,command="]).output() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if let Some((pid, command)) = line.split_once(' ')
                && let Ok(pid) = pid.parse()
            {
                processes.push((pid, command.trim().to_string()));
            }
        }
    }
    processes.sort_by_key(|(pid, _)| std::cmp::Reverse(*pid));
    processes
}

impl IdeShell {
    fn debug_state_path(&self, cx: &App) -> Option<PathBuf> {
        let root = self.workspace.root()?;
        Some(debugger::state_path(&settings::path(cx)?, root))
    }

    /// Load breakpoints, exception choices, watches and the selected profile of the
    /// current workspace.
    pub(super) fn load_debug_state(&mut self, cx: &mut Context<Self>) {
        let state = self
            .debug_state_path(cx)
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<PersistedState>(&bytes).ok())
            .unwrap_or_default();
        self.debug_profile = state.profile.clone();
        if let Some(root) = self.workspace.root().map(Path::to_path_buf) {
            self.debugger
                .update(cx, |debugger, cx| debugger.import_state(&root, &state, cx));
        }
    }

    pub(super) fn save_debug_state(&self, cx: &App) {
        let (Some(path), Some(root)) = (self.debug_state_path(cx), self.workspace.root()) else {
            return;
        };
        let state = self
            .debugger
            .read(cx)
            .export_state(root, self.debug_profile.clone());
        let Ok(json) = serde_json::to_vec_pretty(&state) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) =
            tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
        {
            use std::io::Write as _;
            if file.write_all(&json).is_ok() {
                let _ = file.persist(&path);
            }
        }
    }

    /// The profile Start Debugging uses: the remembered selection, else the first.
    fn selected_profile(&self, profiles: &[DebuggerConfig]) -> Option<DebuggerConfig> {
        self.debug_profile
            .as_ref()
            .and_then(|name| profiles.iter().find(|profile| &profile.name == name))
            .or_else(|| profiles.first())
            .cloned()
    }

    pub(super) fn start_debugging(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.load_settings(cx) else {
            return;
        };
        let profiles = settings.debug_profiles();
        let Some(profile) = self.selected_profile(&profiles) else {
            self.status =
                "Configure \"debug_profiles\" (or \"debugger\") in settings.json to start debugging".into();
            cx.notify();
            return;
        };
        if profile.request == DebugRequest::Attach && profile.process_id.is_none() {
            return self.pick_process(profile, window, cx);
        }
        self.launch_profile(profile, cx);
    }

    pub(super) fn launch_profile(&mut self, profile: DebuggerConfig, cx: &mut Context<Self>) {
        let name = if profile.name.is_empty() {
            "Default".to_string()
        } else {
            profile.name.clone()
        };
        if let Some(task) = profile.pre_launch_task.clone() {
            return self.run_task_then(task, Some(PendingLaunch::Debug(profile)), cx);
        }
        let Some(directory) = self.config_directory(profile.working_directory.clone(), cx) else {
            self.status = format!("Debug profile \"{name}\": the working directory does not exist");
            cx.notify();
            return;
        };
        let result = self.debugger.update(cx, |debugger, cx| {
            debugger.start(profile.clone(), directory, cx)
        });
        self.status = match result {
            Ok(title) => match profile.request {
                DebugRequest::Attach => format!(
                    "Attached to {} ({name})",
                    profile.process_id.map_or(title, |pid| pid.to_string())
                ),
                DebugRequest::Launch => format!("Debugging {title} ({name})"),
            },
            Err(error) => format!("Debug profile \"{name}\": {error}"),
        };
        cx.notify();
    }

    fn pick_process(
        &mut self,
        profile: DebuggerConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = list_processes()
            .into_iter()
            .map(|(pid, command)| PickerItem {
                label: format!("{pid}  {}", command.chars().take(100).collect::<String>()),
                detail: String::new(),
                filter: format!("{pid} {command}"),
                depth: 0,
                action: PickerAction::AttachProcess(pid),
            })
            .collect();
        self.attach_profile = Some(profile);
        self.show_picker(
            PickerKind::Locations,
            "Attach to process (type a name or PID)".into(),
            items,
            window,
            cx,
        );
    }

    pub(super) fn attach_to(&mut self, pid: u32, _: &mut Window, cx: &mut Context<Self>) {
        let Some(mut profile) = self.attach_profile.take() else {
            return;
        };
        profile.process_id = Some(pid);
        self.launch_profile(profile, cx);
    }

    pub(super) fn select_debug_profile(
        &mut self,
        _: &SelectDebugProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(settings) = self.load_settings(cx) else {
            return;
        };
        let profiles = settings.debug_profiles();
        if profiles.is_empty() {
            self.status = "No debug profiles are configured".into();
            cx.notify();
            return;
        }
        let selected = self.selected_profile(&profiles).map(|profile| profile.name);
        let items = profiles
            .iter()
            .map(|profile| PickerItem {
                label: profile.name.clone(),
                detail: format!(
                    "{}{}",
                    match profile.request {
                        DebugRequest::Launch => format!("launch {}", profile.program),
                        DebugRequest::Attach => "attach".to_string(),
                    },
                    if selected.as_ref() == Some(&profile.name) {
                        " · selected"
                    } else {
                        ""
                    }
                ),
                filter: profile.name.clone(),
                depth: 0,
                action: PickerAction::DebugProfile(profile.name.clone()),
            })
            .collect();
        self.show_picker(
            PickerKind::Locations,
            "Select debug profile".into(),
            items,
            window,
            cx,
        );
    }

    pub(super) fn choose_debug_profile(&mut self, name: String, cx: &mut Context<Self>) {
        // Switching profiles stops a running session so adapters do not leak.
        if self.debugger.read(cx).is_active() {
            self.debugger.update(cx, |debugger, cx| debugger.stop(cx));
        }
        self.status = format!("Debug profile: {name}");
        self.debug_profile = Some(name);
        self.save_debug_state(cx);
        cx.notify();
    }

    fn cursor_breakpoint_target(&mut self, cx: &mut Context<Self>) -> Option<(PathBuf, usize)> {
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            self.status = "Save the file before setting a breakpoint".into();
            cx.notify();
            return None;
        };
        let row = self.editor().read(cx).cursor_position().line as usize;
        Some((path, self.source_line(row, cx)?))
    }

    pub(super) fn toggle_breakpoint_enabled(
        &mut self,
        _: &ToggleBreakpointEnabled,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, line)) = self.cursor_breakpoint_target(cx) else {
            return;
        };
        let result = self.debugger.update(cx, |debugger, cx| {
            debugger.toggle_breakpoint_enabled(path, line, cx)
        });
        self.status = match result {
            Some(true) => format!("Breakpoint enabled at line {line}"),
            Some(false) => format!("Breakpoint disabled at line {line}"),
            None => "No breakpoint on this line".into(),
        };
        cx.notify();
    }

    pub(super) fn remove_all_breakpoints(
        &mut self,
        _: &RemoveAllBreakpoints,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debugger
            .update(cx, |debugger, cx| debugger.remove_all_breakpoints(cx));
        self.status = "Removed all breakpoints".into();
        cx.notify();
    }

    pub(super) fn edit_breakpoint(
        &mut self,
        _: &EditBreakpoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some((path, line)) = self.cursor_breakpoint_target(cx) else {
            return;
        };
        let options = self.debugger.read(cx).options_at(&path, line);
        let field = |placeholder: &'static str,
                     value: Option<String>,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value.unwrap_or_default())
                    .validate(|text, _| !text.contains('\n'))
            })
        };
        let condition = field(
            "Condition, e.g. i > 10",
            options.condition.clone(),
            window,
            cx,
        );
        let hits = field(
            "Hit count, e.g. 5 or >= 3",
            options.hit_condition.clone(),
            window,
            cx,
        );
        let log = field(
            "Log message (logpoint), e.g. x = {x}",
            options.log_message.clone(),
            window,
            cx,
        );
        let shell = cx.entity().downgrade();
        let enabled = options.enabled;
        window.open_dialog(cx, {
            let (condition, hits, log) = (condition.clone(), hits.clone(), log.clone());
            move |dialog, _, _| {
                let (shell, condition, hits, log, path) = (
                    shell.clone(),
                    condition.clone(),
                    hits.clone(),
                    log.clone(),
                    path.clone(),
                );
                dialog
                    .title(format!("Breakpoint at line {line}"))
                    .w(px(440.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(Input::new(&condition))
                            .child(Input::new(&hits))
                            .child(Input::new(&log)),
                    )
                    .confirm()
                    .on_ok(move |_, _, cx| {
                        let value = |input: &Entity<InputState>, cx: &App| {
                            Some(input.read(cx).value().trim().to_string())
                                .filter(|text| !text.is_empty())
                        };
                        let options = BreakpointOptions {
                            enabled,
                            condition: value(&condition, cx),
                            hit_condition: value(&hits, cx),
                            log_message: value(&log, cx),
                        };
                        let path = path.clone();
                        let _ = shell.update(cx, |shell, cx| {
                            shell.debugger.update(cx, |debugger, cx| {
                                debugger.set_breakpoint_options(path, line, options, cx)
                            });
                            shell.status = format!("Updated the breakpoint at line {line}");
                        });
                        true
                    })
            }
        });
        condition.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn add_watch(&mut self, _: &AddWatch, window: &mut Window, cx: &mut Context<Self>) {
        self.active_panel = ToolPanel::Debug;
        // Seed with the selected text when there is a single-line selection.
        let selected = {
            let editor = self.editor().read(cx);
            let text = editor.text().to_string();
            text.get(editor.selected_byte_range())
                .filter(|text| !text.is_empty() && !text.contains('\n'))
                .map(str::to_string)
        };
        match selected {
            Some(expression) => self
                .debugger
                .update(cx, |debugger, cx| debugger.add_watch(expression, cx)),
            None => self
                .debugger
                .update(cx, |debugger, cx| debugger.focus_watch_input(window, cx)),
        }
        cx.notify();
    }

    /// Keep breakpoints attached to their lines when lines are inserted or removed.
    pub(super) fn shift_breakpoints_for_edit(
        &mut self,
        ix: usize,
        old: &str,
        cx: &mut Context<Self>,
    ) {
        let buffer = &self.buffers[ix];
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            return;
        };
        if self.debugger.read(cx).lines(&path).is_none() {
            return;
        }
        let new = buffer.document.text().to_string();
        let prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = old[prefix..]
            .bytes()
            .rev()
            .zip(new[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let removed = old[prefix..old.len() - suffix].matches('\n').count();
        let added = new[prefix..new.len() - suffix].matches('\n').count();
        if removed == added {
            return;
        }
        let line = old[..prefix].matches('\n').count() + 1;
        self.debugger.update(cx, |debugger, cx| {
            debugger.shift_breakpoints(&path, line, removed, added, cx)
        });
    }
}

/// A launch waiting for its pre-launch task (IDE-019).
#[derive(Clone, Debug)]
pub(super) enum PendingLaunch {
    Debug(DebuggerConfig),
}

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

use dap::{Client, Event, Scope, StackFrame, Variable};
use gpui::{
    Action, AnyElement, Context, Div, Entity, EventEmitter, FontWeight, Hsla, IntoElement, Render,
    ScrollHandle, Stateful, Task, Window, div, prelude::*, px,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
};
use serde_json::{Value, json};

use crate::{
    assets::AppIcon,
    commands::{
        DebugContinue, DebugInterrupt, DebugRestart, DebugStepInto, DebugStepOut, DebugStepOver,
        DebugStop, StartDebugger,
    },
    settings::DebuggerConfig,
    theme, ui,
};

mod features;
mod launch;
mod view;

pub use features::{BreakpointOptions, PersistedState, state_path};
use launch::*;
use view::{row, section_header};

#[cfg(test)]
#[path = "debugger_tests.rs"]
mod tests;

const MEMORY_PAGE: usize = 256;
const MEMORY_ROW: usize = 16;
const CONSOLE_LIMIT: usize = 5000;
const ROW_HEIGHT: f32 = 22.;
const ROW_GROUP: &str = "debug-row";

pub enum DebuggerEvent {
    /// Breakpoints, exception filters or watches changed (persist them).
    StateChanged,
    Reveal {
        path: PathBuf,
        line: usize,
    },
    Status(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Starting,
    Running,
    Paused(String),
    Exited(String),
}

impl State {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "Not running".into(),
            Self::Starting => "Starting…".into(),
            Self::Running => "Running".into(),
            Self::Paused(reason) => format!("Paused · {reason}"),
            Self::Exited(reason) => reason.clone(),
        }
    }

    fn color(&self) -> Hsla {
        match self {
            Self::Idle | Self::Exited(_) => theme::subtle().into(),
            Self::Starting | Self::Running => {
                theme::git_change(ide_core::git::Change::Added).into()
            }
            Self::Paused(_) => theme::execution_line().into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Over,
    Into,
    Out,
}

impl Step {
    fn command(self) -> &'static str {
        match self {
            Self::Over => "next",
            Self::Into => "stepIn",
            Self::Out => "stepOut",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Detail {
    #[default]
    Console,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    Output,
    Error,
    Input,
    Result,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConsoleLine {
    kind: LineKind,
    text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MemoryView {
    label: String,
    reference: String,
    offset: i64,
    start: Option<u64>,
    bytes: Vec<u8>,
    unreadable: bool,
    loading: bool,
    error: Option<String>,
}

struct Session {
    client: Client,
    configured: bool,
    _events: Task<()>,
}

pub struct Debugger {
    breakpoints: HashMap<PathBuf, BTreeSet<usize>>,
    options: HashMap<(PathBuf, usize), BreakpointOptions>,
    capabilities: Value,
    exception_filters: Vec<features::ExceptionFilter>,
    exception_choices: HashMap<String, bool>,
    watches: Vec<features::Watch>,
    watch_generation: u64,
    watch_input: Entity<InputState>,
    /// Terminate the debuggee on stop (launch sessions, or attach with
    /// `terminate_on_stop`).
    terminate_on_stop: bool,
    rejected: HashSet<(PathBuf, usize)>,
    session: Option<Session>,
    last_launch: Option<(DebuggerConfig, Option<PathBuf>)>,
    state: State,
    title: String,
    thread_id: Option<i64>,
    frames: Vec<StackFrame>,
    frame: usize,
    scopes: Vec<(Scope, Option<Vec<Variable>>)>,
    collapsed: HashSet<String>,
    children: HashMap<i64, Vec<Variable>>,
    expanded: HashSet<i64>,
    generation: u64,
    memory: Option<MemoryView>,
    detail: Detail,
    console: Vec<ConsoleLine>,
    console_scroll: ScrollHandle,
    console_input: Entity<InputState>,
    memory_input: Entity<InputState>,
    split: Entity<ResizableState>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<DebuggerEvent> for Debugger {}

impl Debugger {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let console_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Evaluate expression…")
                .multi_line(false)
        });
        let memory_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Address or expression, e.g. &buffer")
                .multi_line(false)
        });
        let watch_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Add watch expression…")
                .multi_line(false)
        });
        let subscriptions = vec![
            cx.subscribe_in(&watch_input, window, |this, input, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let expression = input.read(cx).text().to_string();
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    this.add_watch(expression.trim().to_string(), cx);
                }
            }),
            cx.subscribe_in(&console_input, window, |this, input, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let expression = input.read(cx).text().to_string();
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    this.evaluate(expression.trim().to_string(), cx);
                }
            }),
            cx.subscribe_in(&memory_input, window, |this, input, event, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let expression = input.read(cx).text().to_string();
                    this.inspect_memory(expression.trim().to_string(), cx);
                }
            }),
        ];
        Self {
            breakpoints: HashMap::new(),
            options: HashMap::new(),
            capabilities: Value::Null,
            exception_filters: Vec::new(),
            exception_choices: HashMap::new(),
            watches: Vec::new(),
            watch_generation: 0,
            watch_input,
            terminate_on_stop: true,
            rejected: HashSet::new(),
            session: None,
            last_launch: None,
            state: State::Idle,
            title: String::new(),
            thread_id: None,
            frames: Vec::new(),
            frame: 0,
            scopes: Vec::new(),
            collapsed: HashSet::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            generation: 0,
            memory: None,
            detail: Detail::default(),
            console: Vec::new(),
            console_scroll: ScrollHandle::new(),
            console_input,
            memory_input,
            split: cx.new(|_| ResizableState::default()),
            _subscriptions: subscriptions,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.state, State::Paused(_))
    }

    pub fn lines(&self, path: &Path) -> Option<&BTreeSet<usize>> {
        self.breakpoints.get(path)
    }

    pub fn is_rejected(&self, path: &Path, line: usize) -> bool {
        self.rejected.contains(&(path.to_path_buf(), line))
    }

    pub fn execution_line(&self, path: &Path) -> Option<usize> {
        if !self.is_paused() {
            return None;
        }
        let frame = self.frames.get(self.frame)?;
        (frame.path()? == path).then_some(frame.line.max(1) as usize)
    }

    pub fn toggle_breakpoint(
        &mut self,
        path: PathBuf,
        line: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let lines = self.breakpoints.entry(path.clone()).or_default();
        let enabled = lines.insert(line);
        if !enabled {
            lines.remove(&line);
            self.options.remove(&(path.clone(), line));
        }
        cx.emit(DebuggerEvent::StateChanged);
        if lines.is_empty() {
            self.breakpoints.remove(&path);
        }
        self.rejected.remove(&(path.clone(), line));
        self.sync_breakpoints(&path, cx);
        cx.notify();
        enabled
    }

    /// Follow a file or folder rename: breakpoints under `from` move to `to`.
    pub fn move_breakpoints(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let moved: Vec<PathBuf> = self
            .breakpoints
            .keys()
            .filter(|path| path.starts_with(from))
            .cloned()
            .collect();
        for old in moved {
            let Ok(suffix) = old.strip_prefix(from) else {
                continue;
            };
            let new = if suffix.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(suffix)
            };
            if let Some(lines) = self.breakpoints.remove(&old) {
                self.breakpoints.insert(new.clone(), lines);
            }
            self.rejected.retain(|(path, _)| path != &old);
            self.sync_breakpoints(&old, cx);
            self.sync_breakpoints(&new, cx);
        }
        cx.notify();
    }

    fn remove_breakpoints(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = match path {
            Some(path) => vec![path.to_path_buf()],
            None => self.breakpoints.keys().cloned().collect(),
        };
        for path in paths {
            self.breakpoints.remove(&path);
            self.rejected.retain(|(rejected, _)| rejected != &path);
            self.sync_breakpoints(&path, cx);
        }
        cx.notify();
    }

    fn sync_breakpoints(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref().filter(|session| session.configured) else {
            return;
        };
        let client = session.client.clone();
        let (lines, breakpoints) = self.source_breakpoints(path);
        let request = client.set_breakpoints(&plain_path(path), &breakpoints);
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| this.verified(path, lines, result, cx));
        })
        .detach();
    }

    fn verified(
        &mut self,
        path: PathBuf,
        lines: Vec<usize>,
        result: Result<Vec<dap::Breakpoint>, dap::Error>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(breakpoints) => {
                for (line, breakpoint) in lines.into_iter().zip(breakpoints) {
                    // The adapter may move a breakpoint to the nearest valid line.
                    let actual = breakpoint
                        .line
                        .and_then(|line| usize::try_from(line).ok())
                        .filter(|actual| *actual != line && *actual > 0)
                        .filter(|actual| {
                            !self
                                .breakpoints
                                .get(&path)
                                .is_some_and(|lines| lines.contains(actual))
                        });
                    let line = match actual {
                        Some(actual) => {
                            if let Some(lines) = self.breakpoints.get_mut(&path) {
                                lines.remove(&line);
                                lines.insert(actual);
                            }
                            if let Some(options) = self.options.remove(&(path.clone(), line)) {
                                self.options.insert((path.clone(), actual), options);
                            }
                            cx.emit(DebuggerEvent::StateChanged);
                            actual
                        }
                        None => line,
                    };
                    if breakpoint.verified {
                        self.rejected.remove(&(path.clone(), line));
                    } else {
                        self.rejected.insert((path.clone(), line));
                    }
                }
            }
            Err(error) => self.log(LineKind::Error, format!("Breakpoints: {error}")),
        }
        cx.notify();
    }

    pub fn start(
        &mut self,
        config: DebuggerConfig,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let plan = plan(&config)?;
        self.shutdown();
        let client = Client::start(&dap::Options {
            program: plan.adapter.clone(),
            args: plan.adapter_args.clone(),
            env: Default::default(),
            working_directory: directory.clone(),
        })
        .map_err(|error| format!("Could not start {}: {error}", plan.adapter))?;
        self.title = if config.name.is_empty() {
            stem(&plan.program)
        } else {
            config.name.clone()
        };
        self.console.clear();
        self.log(
            LineKind::Info,
            format!(
                "Launching {} with {}",
                plan.program,
                [plan.adapter.as_str()]
                    .into_iter()
                    .chain(plan.adapter_args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        self.state = State::Starting;
        self.rejected.clear();
        self.clear_pause();
        let events = client.events();
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |this, cx| this.handle(event, cx)).is_err() {
                    break;
                }
            }
        });
        let handshake = client.initialize(&plan.adapter_id);
        let launch_client = client.clone();
        let attach = config.request == crate::settings::DebugRequest::Attach;
        self.terminate_on_stop = !attach || config.terminate_on_stop;
        let arguments = if attach {
            attach_arguments(&plan, &config, directory.as_deref())
        } else {
            launch_arguments(&plan, &config, directory.as_deref())
        };
        let command = if attach { "attach" } else { "launch" };
        cx.spawn(async move |this, cx| {
            let result = match handshake.await {
                Ok(capabilities) => {
                    let _ = this.update(cx, |this, _| this.adopt_capabilities(capabilities));
                    launch_client.request(command, arguments).await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    this.log(LineKind::Error, format!("Launch failed: {error}"));
                    let log = launch_client.log();
                    if !log.trim().is_empty() {
                        this.log(LineKind::Error, log.trim_end().to_string());
                    }
                    this.state = State::Exited("Launch failed".into());
                    this.session = None;
                    cx.emit(DebuggerEvent::Status(format!("Debugger: {error}")));
                    cx.notify();
                });
            }
        })
        .detach();
        self.session = Some(Session {
            client,
            configured: false,
            _events: pump,
        });
        self.last_launch = Some((config, directory));
        cx.notify();
        Ok(self.title.clone())
    }

    pub fn restart(&mut self, cx: &mut Context<Self>) -> Result<String, String> {
        let (config, directory) = self
            .last_launch
            .clone()
            .ok_or_else(|| "Start the debugger first".to_string())?;
        self.start(config, directory, cx)
    }

    fn shutdown(&mut self) {
        if let Some(session) = self.session.take() {
            drop(session.client.disconnect_with(self.terminate_on_stop));
        }
        self.mark_watches_stale();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.shutdown();
        self.clear_pause();
        self.state = State::Exited("Stopped".into());
        self.log(LineKind::Info, "Debugging session stopped".into());
        cx.notify();
    }

    fn clear_pause(&mut self) {
        self.generation += 1;
        self.frames.clear();
        self.frame = 0;
        self.scopes.clear();
        self.children.clear();
        self.expanded.clear();
    }

    fn client(&self) -> Option<&Client> {
        self.session.as_ref().map(|session| &session.client)
    }

    fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Initialized => self.configure(cx),
            Event::Stopped {
                reason,
                description,
                thread_id,
            } => {
                self.state = State::Paused(description.unwrap_or(reason));
                if thread_id.is_some() {
                    self.thread_id = thread_id;
                }
                self.clear_pause();
                self.load_frames(cx);
            }
            Event::Continued => {
                self.state = State::Running;
                self.clear_pause();
                self.mark_watches_stale();
            }
            Event::Breakpoint { reason, breakpoint } => {
                if let (Some(path), Some(line)) = (
                    breakpoint
                        .source
                        .as_ref()
                        .and_then(|source| source.path.clone()),
                    breakpoint.line.and_then(|line| usize::try_from(line).ok()),
                ) {
                    let path = same_file(&path);
                    if reason != "removed" {
                        if breakpoint.verified {
                            self.rejected.remove(&(path, line));
                        } else {
                            self.rejected.insert((path, line));
                        }
                    }
                }
            }
            Event::Output { category, output } => {
                if category != "telemetry" {
                    let kind = if category == "stderr" {
                        LineKind::Error
                    } else if category == "console" || category == "important" {
                        LineKind::Info
                    } else {
                        LineKind::Output
                    };
                    for line in output.trim_end_matches(['\r', '\n']).lines() {
                        self.log(kind, line.trim_end_matches('\r').to_string());
                    }
                }
            }
            Event::Exited(code) => {
                self.log(LineKind::Info, format!("Process exited with code {code}"));
                self.state = State::Exited(format!("Exited ({code})"));
            }
            Event::Terminated => {
                if !matches!(self.state, State::Exited(_)) {
                    self.state = State::Exited("Terminated".into());
                }
                self.clear_pause();
                self.shutdown();
                cx.emit(DebuggerEvent::Status("Debugging session ended".into()));
            }
            Event::Closed(reason) => {
                if let Some(session) = self.session.take() {
                    let log = session.client.log();
                    if !log.trim().is_empty() {
                        self.log(LineKind::Error, log.trim_end().to_string());
                    }
                    if !matches!(self.state, State::Exited(_)) {
                        self.log(LineKind::Error, reason.clone());
                        self.state = State::Exited("Adapter exited".into());
                    }
                    self.clear_pause();
                }
            }
        }
        cx.notify();
    }

    fn configure(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        session.configured = true;
        let client = session.client.clone();
        let paths: Vec<PathBuf> = self.breakpoints.keys().cloned().collect();
        let mut files: Vec<(PathBuf, Vec<usize>)> = Vec::new();
        let mut requests = Vec::new();
        for path in paths {
            let (lines, breakpoints) = self.source_breakpoints(&path);
            requests.push(client.set_breakpoints(&plain_path(&path), &breakpoints));
            files.push((path, lines));
        }
        let exceptions = client.request(
            "setExceptionBreakpoints",
            json!({"filters": self.enabled_filters()}),
        );
        cx.spawn(async move |this, cx| {
            let mut results = Vec::new();
            for request in requests {
                results.push(request.await);
            }
            let _ = exceptions.await;
            let done = client.request("configurationDone", Value::Null).await;
            let _ = this.update(cx, |this, cx| {
                for ((path, lines), result) in files.into_iter().zip(results) {
                    this.verified(path, lines, result, cx);
                }
                match done {
                    Ok(_) => {
                        if this.state == State::Starting {
                            this.state = State::Running;
                        }
                    }
                    Err(error) => this.log(LineKind::Error, format!("Configuration: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_frames(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client().cloned() else {
            return;
        };
        let generation = self.generation;
        let known = self.thread_id;
        cx.spawn(async move |this, cx| {
            let thread_id = match known {
                Some(id) => Some(id),
                None => client
                    .threads()
                    .await
                    .ok()
                    .and_then(|threads| threads.first().map(|thread| thread.id)),
            };
            let Some(thread_id) = thread_id else {
                return;
            };
            let frames = client.stack_trace(thread_id).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.thread_id = Some(thread_id);
                match frames {
                    Ok(frames) => {
                        this.frames = frames
                            .into_iter()
                            .map(|mut frame| {
                                if let Some(source) = frame.source.as_mut()
                                    && let Some(path) = source.path.as_mut()
                                {
                                    *path = same_file(path);
                                }
                                frame
                            })
                            .collect();
                        let first = this
                            .frames
                            .iter()
                            .position(|frame| frame.path().is_some_and(|path| path.is_file()))
                            .unwrap_or(0);
                        this.select_frame(first, cx);
                    }
                    Err(error) => this.log(LineKind::Error, format!("Call stack: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_frame(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(frame) = self.frames.get(ix).cloned() else {
            return;
        };
        self.frame = ix;
        self.refresh_watches(cx);
        self.scopes.clear();
        self.children.clear();
        self.expanded.clear();
        if let Some(path) = frame.path() {
            cx.emit(DebuggerEvent::Reveal {
                path: path.clone(),
                line: frame.line.max(1) as usize,
            });
        }
        let Some(client) = self.client().cloned() else {
            return;
        };
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let scopes = match client.scopes(frame.id).await {
                Ok(scopes) => scopes,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.log(LineKind::Error, format!("Scopes: {error}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let mut loaded = Vec::new();
            for scope in scopes {
                let variables = if scope.expensive {
                    None
                } else {
                    Some(
                        client
                            .variables(scope.variables_reference)
                            .await
                            .unwrap_or_default(),
                    )
                };
                loaded.push((scope, variables));
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.frame == ix {
                    this.scopes = loaded;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_scope(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((scope, variables)) = self.scopes.get(ix) else {
            return;
        };
        let name = scope.name.clone();
        if variables.is_none() {
            self.collapsed.remove(&name);
            let reference = scope.variables_reference;
            self.fetch_variables(
                reference,
                move |this, variables| {
                    if let Some(entry) = this.scopes.get_mut(ix) {
                        entry.1 = Some(variables);
                    }
                },
                cx,
            );
        } else if !self.collapsed.remove(&name) {
            self.collapsed.insert(name);
        }
        cx.notify();
    }

    fn toggle_variable(&mut self, reference: i64, cx: &mut Context<Self>) {
        if !self.expanded.remove(&reference) {
            self.expanded.insert(reference);
            if !self.children.contains_key(&reference) {
                self.fetch_variables(
                    reference,
                    move |this, variables| {
                        this.children.insert(reference, variables);
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn fetch_variables(
        &mut self,
        reference: i64,
        store: impl FnOnce(&mut Self, Vec<Variable>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client().cloned() else {
            return;
        };
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = client.variables(reference).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(variables) => store(this, variables),
                    Err(error) => this.log(LineKind::Error, format!("Variables: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn frame_id(&self) -> Option<i64> {
        self.is_paused()
            .then(|| self.frames.get(self.frame).map(|frame| frame.id))
            .flatten()
    }

    fn evaluate(&mut self, expression: String, cx: &mut Context<Self>) {
        if expression.is_empty() {
            return;
        }
        self.log(LineKind::Input, expression.clone());
        let Some(client) = self.client().cloned() else {
            self.log(
                LineKind::Error,
                "Start the debugger to evaluate expressions".into(),
            );
            cx.notify();
            return;
        };
        let request = client.evaluate(&expression, self.frame_id(), "repl");
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(evaluation) => {
                        let text = match evaluation.type_name.filter(|name| !name.is_empty()) {
                            Some(type_name) => format!("{}  : {type_name}", evaluation.result),
                            None => evaluation.result,
                        };
                        for line in text.lines() {
                            this.log(LineKind::Result, line.to_string());
                        }
                    }
                    Err(error) => this.log(LineKind::Error, error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn inspect_memory(&mut self, expression: String, cx: &mut Context<Self>) {
        if expression.is_empty() {
            return;
        }
        self.detail = Detail::Memory;
        let Some(client) = self.client().cloned() else {
            self.memory = Some(MemoryView {
                label: expression,
                error: Some("Start the debugger to read memory".into()),
                ..Default::default()
            });
            cx.notify();
            return;
        };
        if dap::parse_address(&expression).is_some() {
            self.show_memory(expression.clone(), expression, cx);
            return;
        }
        let request = client.evaluate(&expression, self.frame_id(), "watch");
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(evaluation) => {
                    let reference = evaluation.memory_reference.clone().or_else(|| {
                        pointer_reference(&Variable {
                            value: evaluation.result.clone(),
                            ..Default::default()
                        })
                    });
                    match reference {
                        Some(reference) => this.show_memory(expression, reference, cx),
                        None => {
                            this.memory = Some(MemoryView {
                                label: expression,
                                error: Some(format!(
                                    "{} has no memory address — try &{}",
                                    evaluation.result, evaluation.result
                                )),
                                ..Default::default()
                            });
                            cx.notify();
                        }
                    }
                }
                Err(error) => {
                    this.memory = Some(MemoryView {
                        label: expression,
                        error: Some(error.to_string()),
                        ..Default::default()
                    });
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn show_memory(&mut self, label: String, reference: String, cx: &mut Context<Self>) {
        self.detail = Detail::Memory;
        self.memory = Some(MemoryView {
            label,
            reference,
            ..Default::default()
        });
        self.load_memory(cx);
    }

    fn page_memory(&mut self, direction: i64, cx: &mut Context<Self>) {
        if let Some(memory) = self.memory.as_mut() {
            memory.offset += direction * MEMORY_PAGE as i64;
            self.load_memory(cx);
        }
    }

    fn load_memory(&mut self, cx: &mut Context<Self>) {
        let Some(memory) = self.memory.as_mut() else {
            return;
        };
        let Some(client) = self.session.as_ref().map(|session| session.client.clone()) else {
            memory.error = Some("Start the debugger to read memory".into());
            cx.notify();
            return;
        };
        memory.loading = true;
        memory.error = None;
        let reference = memory.reference.clone();
        let offset = memory.offset;
        cx.spawn(async move |this, cx| {
            let result = client.read_memory(&reference, offset, MEMORY_PAGE).await;
            let _ = this.update(cx, |this, cx| {
                let Some(memory) = this
                    .memory
                    .as_mut()
                    .filter(|memory| memory.reference == reference && memory.offset == offset)
                else {
                    return;
                };
                memory.loading = false;
                match result {
                    Ok(read) => {
                        memory.start = read.start();
                        memory.bytes = read.bytes();
                        memory.unreadable = read.unreadable_bytes.unwrap_or(0) > 0;
                    }
                    Err(error) => {
                        memory.bytes.clear();
                        memory.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        self.run_thread_command("continue", cx)
    }

    pub fn step(&mut self, step: Step, cx: &mut Context<Self>) -> Result<(), String> {
        self.run_thread_command(step.command(), cx)
    }

    fn run_thread_command(&mut self, command: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let client = self
            .client()
            .cloned()
            .ok_or_else(|| "Start the debugger first".to_string())?;
        if !self.is_paused() {
            return Err("The program is not paused".into());
        }
        let thread_id = self
            .thread_id
            .ok_or_else(|| "No thread is stopped".to_string())?;
        let request = client.step(command, thread_id);
        self.state = State::Running;
        self.clear_pause();
        cx.spawn(async move |this, cx| {
            if let Err(error) = request.await {
                let _ = this.update(cx, |this, cx| {
                    this.log(LineKind::Error, error.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
        Ok(())
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let client = self
            .client()
            .cloned()
            .ok_or_else(|| "Start the debugger first".to_string())?;
        let known = self.thread_id;
        cx.spawn(async move |this, cx| {
            let thread_id = match known {
                Some(id) => Some(id),
                None => client
                    .threads()
                    .await
                    .ok()
                    .and_then(|threads| threads.first().map(|thread| thread.id)),
            };
            let result = match thread_id {
                Some(thread_id) => client
                    .request("pause", json!({"threadId": thread_id}))
                    .await
                    .map(|_| ()),
                None => Err(dap::Error("No running thread".into())),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    this.log(LineKind::Error, format!("Pause: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
        Ok(())
    }

    fn log(&mut self, kind: LineKind, text: String) {
        self.console.push(ConsoleLine { kind, text });
        if self.console.len() > CONSOLE_LIMIT {
            let excess = self.console.len() - CONSOLE_LIMIT;
            self.console.drain(..excess);
        }
        self.console_scroll.scroll_to_bottom();
    }
}

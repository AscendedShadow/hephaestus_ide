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

#[cfg(test)]
#[path = "debugger_tests.rs"]
mod tests;

const MEMORY_PAGE: usize = 256;
const MEMORY_ROW: usize = 16;
const CONSOLE_LIMIT: usize = 5000;
const ROW_HEIGHT: f32 = 22.;
const ROW_GROUP: &str = "debug-row";

pub enum DebuggerEvent {
    Reveal { path: PathBuf, line: usize },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub adapter: String,
    pub adapter_args: Vec<String>,
    pub adapter_id: String,
    pub program: String,
    pub args: Vec<String>,
}

impl Plan {
    fn lldb(&self) -> bool {
        self.adapter_id.contains("lldb")
    }

    fn gdb(&self) -> bool {
        self.adapter_id.contains("gdb")
    }
}

fn stem(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn legacy_target(args: &[String]) -> Option<(String, Vec<String>)> {
    if let Some(ix) = args.iter().position(|arg| arg == "--args") {
        let mut rest = args[ix + 1..].iter().cloned();
        return Some((rest.next()?, rest.collect()));
    }
    let ix = args.iter().position(|arg| !arg.starts_with('-'))?;
    let rest = &args[ix + 1..];
    let program_args = match rest.iter().position(|arg| arg == "--") {
        Some(separator) => rest[separator + 1..].to_vec(),
        None => Vec::new(),
    };
    Some((args[ix].clone(), program_args))
}

fn sibling(program: &str, name: &str) -> String {
    let path = Path::new(program);
    match path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
            .to_string_lossy()
            .into_owned(),
        None => name.into(),
    }
}

fn default_adapter() -> (String, Vec<String>) {
    if cfg!(target_os = "linux") {
        ("gdb".into(), vec!["--interpreter=dap".into()])
    } else {
        ("lldb-dap".into(), Vec::new())
    }
}

pub fn plan(config: &DebuggerConfig) -> Result<Plan, String> {
    let missing =
        || "Set \"program\" in the debugger settings to the executable to debug".to_string();
    let (adapter, adapter_args, program, args) = if !config.adapter.trim().is_empty() {
        if config.program.trim().is_empty() {
            return Err(missing());
        }
        (
            config.adapter.clone(),
            config.adapter_args.clone(),
            config.program.clone(),
            config.args.clone(),
        )
    } else {
        let name = stem(&config.program);
        if config.program.trim().is_empty() {
            return Err(missing());
        } else if name.contains("dap") || name.contains("vscode") {
            let (program, args) = config
                .args
                .split_first()
                .map(|(program, args)| (program.clone(), args.to_vec()))
                .ok_or_else(missing)?;
            (config.program.clone(), Vec::new(), program, args)
        } else if name.contains("gdb") {
            let (program, args) = legacy_target(&config.args).ok_or_else(missing)?;
            (
                config.program.clone(),
                vec!["--interpreter=dap".into()],
                program,
                args,
            )
        } else if name.contains("lldb") {
            let (program, args) = legacy_target(&config.args).ok_or_else(missing)?;
            (
                sibling(&config.program, "lldb-dap"),
                Vec::new(),
                program,
                args,
            )
        } else {
            let (adapter, adapter_args) = default_adapter();
            (
                adapter,
                adapter_args,
                config.program.clone(),
                config.args.clone(),
            )
        }
    };
    let adapter_id = match stem(&adapter) {
        name if name.contains("gdb") => "gdb".into(),
        name if name.contains("lldb") => "lldb-dap".into(),
        name => name,
    };
    Ok(Plan {
        adapter,
        adapter_args,
        adapter_id,
        program,
        args,
    })
}

pub fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

fn same_file(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn launch_arguments(plan: &Plan, config: &DebuggerConfig, directory: Option<&Path>) -> Value {
    let program = match directory {
        Some(directory) if Path::new(&plan.program).is_relative() => {
            plain_path(&directory.join(&plan.program))
        }
        _ => PathBuf::from(&plan.program),
    };
    let mut arguments = json!({
        "name": if config.name.is_empty() { "Debug" } else { config.name.as_str() },
        "type": plan.adapter_id,
        "request": "launch",
        "program": program,
        "args": plan.args,
        "stopOnEntry": config.stop_on_entry,
    });
    if let Some(directory) = directory {
        arguments["cwd"] = json!(plain_path(directory));
    }
    if !config.env.is_empty() {
        arguments["env"] = if plan.lldb() {
            let mut pairs: Vec<String> = config
                .env
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            pairs.sort();
            json!(pairs)
        } else {
            json!(config.env)
        };
    }
    if plan.gdb() && config.stop_on_entry {
        arguments["stopAtBeginningOfMainSubprogram"] = json!(true);
    }
    arguments
}

pub fn hex_rows(start: Option<u64>, offset: i64, bytes: &[u8]) -> Vec<(String, String, String)> {
    bytes
        .chunks(MEMORY_ROW)
        .enumerate()
        .map(|(ix, chunk)| {
            let row = (ix * MEMORY_ROW) as i64;
            let address = match start {
                Some(start) => format!("{:016x}", start.wrapping_add(row as u64)),
                None => format!("+{:06x}", offset + row),
            };
            let hex = chunk
                .iter()
                .enumerate()
                .map(|(ix, byte)| {
                    if ix == MEMORY_ROW / 2 {
                        format!(" {byte:02x}")
                    } else {
                        format!("{byte:02x}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let ascii = chunk
                .iter()
                .map(|byte| {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        *byte as char
                    } else {
                        '·'
                    }
                })
                .collect();
            (address, hex, ascii)
        })
        .collect()
}

fn pointer_reference(variable: &Variable) -> Option<String> {
    if let Some(reference) = &variable.memory_reference {
        return Some(reference.clone());
    }
    let token = variable
        .value
        .split_whitespace()
        .find(|token| token.starts_with("0x"))?
        .trim_end_matches([',', ')', '}']);
    dap::parse_address(token).map(|_| token.to_string())
}

struct Session {
    client: Client,
    configured: bool,
    _events: Task<()>,
}

pub struct Debugger {
    breakpoints: HashMap<PathBuf, BTreeSet<usize>>,
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
        let subscriptions = vec![
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
        }
        if lines.is_empty() {
            self.breakpoints.remove(&path);
        }
        self.rejected.remove(&(path.clone(), line));
        self.sync_breakpoints(&path, cx);
        cx.notify();
        enabled
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
        let lines: Vec<usize> = self
            .breakpoints
            .get(path)
            .map(|lines| lines.iter().copied().collect())
            .unwrap_or_default();
        let request = session.client.set_breakpoints(&plain_path(path), &lines);
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
        let arguments = launch_arguments(&plan, &config, directory.as_deref());
        cx.spawn(async move |this, cx| {
            let result = match handshake.await {
                Ok(_) => launch_client.request("launch", arguments).await,
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
            drop(session.client.disconnect());
        }
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
        let files: Vec<(PathBuf, Vec<usize>)> = self
            .breakpoints
            .iter()
            .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
            .collect();
        let requests: Vec<_> = files
            .iter()
            .map(|(path, lines)| client.set_breakpoints(&plain_path(path), lines))
            .collect();
        let exceptions = client.request("setExceptionBreakpoints", json!({"filters": []}));
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

fn section_header(title: &'static str, count: Option<usize>) -> Div {
    div()
        .h(px(26.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1p5()
        .px_3()
        .child(ui::caption(title))
        .when_some(count, |header, count| header.child(ui::count_badge(count)))
}

fn row(id: impl Into<gpui::ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group(ROW_GROUP)
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1p5()
        .px_2()
        .mx_1()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .when(selected, |row| row.bg(theme::accent_wash()))
        .when(!selected, |row| row.hover(|style| style.bg(theme::hover())))
}

fn breakpoint_dot(rejected: bool) -> Div {
    let color = theme::breakpoint();
    div()
        .size(px(9.))
        .flex_shrink_0()
        .rounded_full()
        .when(rejected, |dot| dot.border_1().border_color(color))
        .when(!rejected, |dot| dot.bg(color))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Debugger {
    fn tool_button(
        &self,
        id: &'static str,
        icon: AppIcon,
        tooltip: &'static str,
        action: &dyn Action,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let boxed = action.boxed_clone();
        Button::new(id)
            .ghost()
            .small()
            .icon(Icon::new(icon).size(px(15.)))
            .tooltip_with_action(tooltip, action, None)
            .disabled(!enabled)
            .on_click(cx.listener(move |_, _, window, cx| {
                window.dispatch_action(boxed.boxed_clone(), cx);
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.is_active();
        let paused = self.is_paused();
        let running = matches!(self.state, State::Running | State::Starting);
        let continue_button = if active {
            self.tool_button(
                "debug-continue",
                AppIcon::Play,
                "Continue",
                &DebugContinue,
                paused,
                cx,
            )
        } else {
            self.tool_button(
                "debug-start",
                AppIcon::Play,
                "Start Debugging",
                &StartDebugger,
                true,
                cx,
            )
        };
        div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .p_0p5()
                    .rounded_md()
                    .bg(theme::panel())
                    .border_1()
                    .border_color(theme::border())
                    .child(continue_button.when(!active || paused, |button| {
                        button.text_color(theme::git_change(ide_core::git::Change::Added))
                    }))
                    .child(self.tool_button(
                        "debug-pause",
                        AppIcon::Pause,
                        "Pause",
                        &DebugInterrupt,
                        active && running,
                        cx,
                    ))
                    .child(div().w(px(1.)).h(px(16.)).mx_0p5().bg(theme::border()))
                    .child(self.tool_button(
                        "debug-next",
                        AppIcon::StepOver,
                        "Step Over",
                        &DebugStepOver,
                        paused,
                        cx,
                    ))
                    .child(self.tool_button(
                        "debug-step",
                        AppIcon::StepInto,
                        "Step Into",
                        &DebugStepInto,
                        paused,
                        cx,
                    ))
                    .child(self.tool_button(
                        "debug-step-out",
                        AppIcon::StepOut,
                        "Step Out",
                        &DebugStepOut,
                        paused,
                        cx,
                    ))
                    .child(div().w(px(1.)).h(px(16.)).mx_0p5().bg(theme::border()))
                    .child(self.tool_button(
                        "debug-restart",
                        AppIcon::Restart,
                        "Restart",
                        &DebugRestart,
                        self.last_launch.is_some(),
                        cx,
                    ))
                    .child(
                        self.tool_button(
                            "debug-stop",
                            AppIcon::Stop,
                            "Stop",
                            &DebugStop,
                            active,
                            cx,
                        )
                        .when(active, |button| button.text_color(theme::breakpoint())),
                    ),
            )
            .child(
                div()
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .rounded_full()
                    .bg(theme::hover())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().size(px(7.)).rounded_full().bg(self.state.color()))
                    .child(self.state.label()),
            )
            .when(!self.title.is_empty(), |bar| {
                bar.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(self.title.clone()),
                )
            })
    }

    fn render_call_stack(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(section_header("CALL STACK", None))
            .when(self.frames.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(theme::subtle())
                        .child(if self.is_paused() {
                            "Loading…"
                        } else {
                            "Not paused"
                        }),
                )
            })
            .children(self.frames.iter().enumerate().map(|(ix, frame)| {
                let selected = ix == self.frame;
                let location = frame
                    .path()
                    .map(|path| format!("{}:{}", file_name(path), frame.line))
                    .or_else(|| frame.instruction_pointer_reference.clone())
                    .unwrap_or_default();
                let has_source = frame.path().is_some_and(|path| path.is_file());
                row(("debug-frame", ix), selected)
                    .child(
                        div()
                            .w(px(12.))
                            .flex_shrink_0()
                            .text_color(theme::execution_line())
                            .when(ix == 0, |slot| slot.child("➜")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::mono_family())
                            .text_color(if has_source {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(frame.name.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::subtle())
                            .child(location),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_frame(ix, cx)))
            }))
    }

    fn render_breakpoints(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut entries: Vec<(&PathBuf, usize)> = self
            .breakpoints
            .iter()
            .flat_map(|(path, lines)| lines.iter().map(move |line| (path, *line)))
            .collect();
        entries.sort();
        div()
            .flex()
            .flex_col()
            .pb_2()
            .child(
                section_header("BREAKPOINTS", Some(entries.len()))
                    .child(div().flex_1())
                    .when(!entries.is_empty(), |header| {
                        header.child(
                            Button::new("debug-clear-breakpoints")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Delete)
                                .tooltip("Remove all breakpoints")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.remove_breakpoints(None, cx)),
                                ),
                        )
                    }),
            )
            .when(entries.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(theme::subtle())
                        .child("Click left of a line number to add one"),
                )
            })
            .children(entries.into_iter().enumerate().map(|(ix, (path, line))| {
                let rejected = self.is_rejected(path, line);
                let reveal = path.clone();
                let remove = path.clone();
                row(("debug-breakpoint", ix), false)
                    .child(breakpoint_dot(rejected))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::text())
                            .child(file_name(path)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::mono_family())
                            .text_color(theme::subtle())
                            .child(line.to_string()),
                    )
                    .child(
                        div()
                            .invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new(("debug-remove-breakpoint", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip("Remove breakpoint")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_breakpoint(remove.clone(), line, cx);
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(DebuggerEvent::Reveal {
                            path: reveal.clone(),
                            line,
                        })
                    }))
            }))
    }

    fn render_variable(
        &self,
        variable: &Variable,
        depth: usize,
        key: &mut usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let reference = variable.variables_reference;
        let expandable = reference > 0;
        let expanded = expandable && self.expanded.contains(&reference);
        let memory = pointer_reference(variable);
        let label = variable
            .evaluate_name
            .clone()
            .unwrap_or_else(|| variable.name.clone());
        *key += 1;
        let ix = *key;
        rows.push(
            row(("debug-variable", ix), false)
                .pl(px(8. + depth as f32 * 14.))
                .font_family(theme::mono_family())
                .child(
                    div()
                        .w(px(12.))
                        .flex_shrink_0()
                        .text_color(theme::subtle())
                        .when(expandable, |slot| {
                            slot.child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(12.)),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme::syntax(crate::syntax::Token::Name))
                        .child(variable.name.clone()),
                )
                .child(div().flex_shrink_0().text_color(theme::subtle()).child("="))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::text())
                        .child(variable.value.clone()),
                )
                .when_some(
                    variable.type_name.clone().filter(|name| !name.is_empty()),
                    |row, type_name| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .max_w(px(160.))
                                .truncate()
                                .text_color(theme::subtle())
                                .child(type_name),
                        )
                    },
                )
                .child(
                    div()
                        .w(px(22.))
                        .flex_shrink_0()
                        .when_some(memory, |slot, memory| {
                            slot.invisible()
                                .group_hover(ROW_GROUP, |style| style.visible())
                                .child(
                                    Button::new(("debug-variable-memory", ix))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(AppIcon::Memory))
                                        .tooltip("View memory")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.show_memory(label.clone(), memory.clone(), cx);
                                        })),
                                )
                        }),
                )
                .when(expandable, |row| {
                    row.on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_variable(reference, cx)),
                    )
                })
                .into_any_element(),
        );
        if expanded {
            match self.children.get(&reference) {
                Some(children) => {
                    for child in children {
                        self.render_variable(child, depth + 1, key, rows, cx);
                    }
                }
                None => {
                    *key += 1;
                    rows.push(
                        div()
                            .pl(px(30. + depth as f32 * 14.))
                            .h(px(ROW_HEIGHT))
                            .text_xs()
                            .text_color(theme::subtle())
                            .child("Loading…")
                            .into_any_element(),
                    );
                }
            }
        }
    }

    fn render_variables(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        let mut key = 0;
        for (ix, (scope, variables)) in self.scopes.iter().enumerate() {
            let collapsed = variables.is_none() || self.collapsed.contains(&scope.name);
            rows.push(
                row(("debug-scope", ix), false)
                    .mt_1()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.))
                        .text_color(theme::subtle()),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::muted())
                            .child(scope.name.clone()),
                    )
                    .when_some(variables.as_ref(), |row, variables| {
                        row.child(ui::count_badge(variables.len()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_scope(ix, cx)))
                    .into_any_element(),
            );
            if let Some(variables) = variables.as_ref().filter(|_| !collapsed) {
                if variables.is_empty() {
                    rows.push(
                        div()
                            .pl(px(30.))
                            .h(px(ROW_HEIGHT))
                            .text_xs()
                            .text_color(theme::subtle())
                            .child("No variables")
                            .into_any_element(),
                    );
                }
                for variable in variables {
                    self.render_variable(variable, 1, &mut key, &mut rows, cx);
                }
            }
        }
        let body =
            if !self.is_active() {
                ui::empty_state(
                    Icon::new(AppIcon::Bug).size(px(28.)),
                    "Press F5 to start debugging",
                )
                .child(div().text_xs().text_color(theme::subtle()).child(
                    "Set \"debugger\" in settings.json — click the gutter to add breakpoints",
                ))
                .into_any_element()
            } else if !self.is_paused() {
                ui::empty_state(
                    Icon::new(AppIcon::Play).size(px(24.)),
                    "Variables appear when the program pauses",
                )
                .into_any_element()
            } else if rows.is_empty() {
                div()
                    .p_3()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child("Loading variables…")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .pb_2()
                    .children(rows)
                    .into_any_element()
            };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(section_header("VARIABLES", None))
            .child(
                div()
                    .id("debug-variables")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }

    fn render_detail_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab =
            |id: &'static str, label: &'static str, detail: Detail, cx: &mut Context<Self>| {
                let active = self.detail == detail;
                div()
                    .id(id)
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|tab| {
                        if active {
                            tab.bg(theme::hover()).text_color(theme::text())
                        } else {
                            tab.text_color(theme::muted())
                                .hover(|style| style.text_color(theme::text()))
                        }
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail = detail;
                        cx.notify();
                    }))
            };
        div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .child(tab("debug-tab-console", "Console", Detail::Console, cx))
            .child(tab("debug-tab-memory", "Memory", Detail::Memory, cx))
    }

    fn render_console(&self) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("debug-console")
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .track_scroll(&self.console_scroll)
                    .overflow_y_scroll()
                    .font_family(theme::mono_family())
                    .text_xs()
                    .children(self.console.iter().map(|line| {
                        let (prefix, color): (&str, Hsla) = match line.kind {
                            LineKind::Output => ("", theme::text().into()),
                            LineKind::Error => ("", theme::error().into()),
                            LineKind::Input => ("› ", theme::accent().into()),
                            LineKind::Result => {
                                ("  ", theme::syntax(crate::syntax::Token::String).into())
                            }
                            LineKind::Info => ("", theme::subtle().into()),
                        };
                        div()
                            .whitespace_nowrap()
                            .text_color(color)
                            .child(format!("{prefix}{}", line.text))
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .text_color(theme::accent())
                            .font_family(theme::mono_family())
                            .text_xs()
                            .child("›"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.console_input).small().appearance(false)),
                    ),
            )
    }

    fn render_memory(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let paging = self
            .memory
            .as_ref()
            .is_some_and(|memory| !memory.reference.is_empty() && self.is_active());
        let header = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .pb_1p5()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.memory_input).small()),
            )
            .child(
                Button::new("debug-memory-go")
                    .small()
                    .label("Go")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let expression = this.memory_input.read(cx).text().to_string();
                        this.inspect_memory(expression.trim().to_string(), cx);
                    })),
            )
            .child(
                Button::new("debug-memory-previous")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronUp)
                    .tooltip("Previous 256 bytes")
                    .disabled(!paging)
                    .on_click(cx.listener(|this, _, _, cx| this.page_memory(-1, cx))),
            )
            .child(
                Button::new("debug-memory-next")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronDown)
                    .tooltip("Next 256 bytes")
                    .disabled(!paging)
                    .on_click(cx.listener(|this, _, _, cx| this.page_memory(1, cx))),
            );
        let body = match &self.memory {
            None => ui::empty_state(
                Icon::new(AppIcon::Memory).size(px(24.)),
                "Inspect memory at an address or expression",
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child("Or hover a variable and click its memory icon"),
            )
            .into_any_element(),
            Some(memory) => {
                let summary = match memory.start {
                    Some(start) => format!("{} @ 0x{:x}", memory.label, start),
                    None => memory.label.clone(),
                };
                div()
                    .flex()
                    .flex_col()
                    .px_3()
                    .pb_2()
                    .font_family(theme::mono_family())
                    .text_xs()
                    .child(
                        div()
                            .pb_1()
                            .text_color(theme::muted())
                            .truncate()
                            .child(summary),
                    )
                    .when_some(memory.error.clone(), |body, error| {
                        body.child(div().text_color(theme::error()).child(error))
                    })
                    .when(memory.loading, |body| {
                        body.child(div().text_color(theme::subtle()).child("Reading…"))
                    })
                    .when(memory.unreadable && !memory.loading, |body| {
                        body.child(
                            div()
                                .text_color(theme::subtle())
                                .child("Some bytes in this range are unreadable"),
                        )
                    })
                    .children(
                        hex_rows(memory.start, memory.offset, &memory.bytes)
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (address, hex, ascii))| {
                                div()
                                    .flex()
                                    .gap_3()
                                    .px_1()
                                    .rounded_sm()
                                    .whitespace_nowrap()
                                    .when(ix % 2 == 1, |row| row.bg(theme::panel()))
                                    .child(div().text_color(theme::subtle()).child(address))
                                    .child(div().text_color(theme::text()).child(hex))
                                    .child(
                                        div()
                                            .text_color(theme::syntax(crate::syntax::Token::String))
                                            .child(ascii),
                                    )
                            }),
                    )
                    .into_any_element()
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("debug-memory")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }
}

impl Render for Debugger {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let detail = match self.detail {
            Detail::Console => self.render_console().into_any_element(),
            Detail::Memory => self.render_memory(cx).into_any_element(),
        };
        div()
            .debug_selector(|| "debug-panel".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("debug-split")
                        .with_state(&self.split)
                        .child(
                            resizable_panel()
                                .size(px(260.))
                                .size_range(px(160.)..px(520.))
                                .child(
                                    div()
                                        .id("debug-stack")
                                        .size_full()
                                        .bg(theme::panel())
                                        .overflow_y_scrollbar()
                                        .child(self.render_call_stack(cx))
                                        .child(self.render_breakpoints(cx)),
                                ),
                        )
                        .child(resizable_panel().child(self.render_variables(cx)))
                        .child(
                            resizable_panel()
                                .size(px(380.))
                                .size_range(px(220.)..px(900.))
                                .child(
                                    div()
                                        .size_full()
                                        .flex()
                                        .flex_col()
                                        .border_l_1()
                                        .border_color(theme::border())
                                        .child(self.render_detail_tabs(cx))
                                        .child(detail),
                                ),
                        ),
                ),
            )
    }
}

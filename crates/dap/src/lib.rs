mod types;

#[cfg(test)]
#[path = "dap_tests.rs"]
mod tests;

pub use types::*;

use std::{
    collections::HashMap,
    fmt,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    thread,
    time::Duration,
};

use async_channel::{Receiver, Sender, unbounded};
use lsp::transport;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const STDERR_LIMIT: usize = 4096;
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Initialized,
    Stopped {
        reason: String,
        description: Option<String>,
        thread_id: Option<i64>,
    },
    Continued,
    Output {
        category: String,
        output: String,
    },
    Exited(i64),
    Terminated,
    Closed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

type Pending = Arc<Mutex<HashMap<i64, Sender<Result<Value, Error>>>>>;

struct Inner {
    outgoing: Sender<String>,
    pending: Pending,
    events: Receiver<Event>,
    stderr: Arc<Mutex<String>>,
    child: Mutex<Option<Child>>,
    next_seq: AtomicI64,
    stopped: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    pub fn start(options: &Options) -> io::Result<Self> {
        let mut command = Command::new(&options.program);
        command
            .args(&options.args)
            .envs(&options.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = options
            .working_directory
            .as_ref()
            .filter(|path| path.is_dir())
        {
            command.current_dir(directory);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let log = Arc::new(Mutex::new(String::new()));
        let client = Self::connect(BufReader::new(stdout), stdin, log.clone());
        if let Ok(mut held) = client.inner.child.lock() {
            *held = Some(child);
        }
        drain_stderr(stderr, log);
        Ok(client)
    }

    fn connect(
        stdout: impl io::BufRead + Send + 'static,
        stdin: impl Write + Send + 'static,
        stderr: Arc<Mutex<String>>,
    ) -> Self {
        let (outgoing, payloads) = unbounded();
        let (events, incoming) = unbounded();
        let pending: Pending = Arc::default();
        let stopped = Arc::new(AtomicBool::new(false));
        write_loop(payloads, stdin);
        read_loop(
            stdout,
            pending.clone(),
            events,
            outgoing.clone(),
            stopped.clone(),
        );
        Self {
            inner: Arc::new(Inner {
                outgoing,
                pending,
                events: incoming,
                stderr,
                child: Mutex::new(None),
                next_seq: AtomicI64::new(1),
                stopped,
            }),
        }
    }

    pub fn events(&self) -> Receiver<Event> {
        self.inner.events.clone()
    }

    pub fn is_running(&self) -> bool {
        !self.inner.stopped.load(Ordering::Relaxed)
    }

    pub fn log(&self) -> String {
        self.inner
            .stderr
            .lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }

    pub fn request(
        &self,
        command: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<Value, Error>> + use<> {
        let seq = self.inner.next_seq.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = async_channel::bounded(1);
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.insert(seq, sender);
        }
        let sent = self.send(json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        }));
        async move {
            sent?;
            receiver
                .recv()
                .await
                .map_err(|_| Error::new("the debug adapter stopped"))?
        }
    }

    fn fetch<T: DeserializeOwned>(
        &self,
        command: &str,
        arguments: Value,
        field: &'static str,
    ) -> impl Future<Output = Result<T, Error>> + use<T> {
        let request = self.request(command, arguments);
        async move {
            let body = request.await?;
            let value = if field.is_empty() {
                body
            } else {
                body.get(field).cloned().unwrap_or(Value::Null)
            };
            serde_json::from_value(value).map_err(|error| Error::new(error.to_string()))
        }
    }

    pub fn initialize(
        &self,
        adapter_id: &str,
    ) -> impl Future<Output = Result<Value, Error>> + use<> {
        self.request(
            "initialize",
            json!({
                "clientID": "hephaestus",
                "clientName": "Hephaestus",
                "adapterID": adapter_id,
                "locale": "en-US",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                "supportsVariableType": true,
                "supportsVariablePaging": false,
                "supportsRunInTerminalRequest": false,
                "supportsMemoryReferences": true,
                "supportsProgressReporting": false,
                "supportsInvalidatedEvent": false,
                "supportsMemoryEvent": false,
            }),
        )
    }

    pub fn set_breakpoints(
        &self,
        path: &Path,
        lines: &[usize],
    ) -> impl Future<Output = Result<Vec<Breakpoint>, Error>> + use<> {
        let breakpoints: Vec<Value> = lines.iter().map(|line| json!({"line": line})).collect();
        self.fetch(
            "setBreakpoints",
            json!({
                "source": {
                    "name": path.file_name().map(|name| name.to_string_lossy().into_owned()),
                    "path": path,
                },
                "breakpoints": breakpoints,
            }),
            "breakpoints",
        )
    }

    pub fn threads(&self) -> impl Future<Output = Result<Vec<Thread>, Error>> + use<> {
        self.fetch("threads", Value::Null, "threads")
    }

    pub fn stack_trace(
        &self,
        thread_id: i64,
    ) -> impl Future<Output = Result<Vec<StackFrame>, Error>> + use<> {
        self.fetch(
            "stackTrace",
            json!({"threadId": thread_id, "startFrame": 0, "levels": 64}),
            "stackFrames",
        )
    }

    pub fn scopes(&self, frame_id: i64) -> impl Future<Output = Result<Vec<Scope>, Error>> + use<> {
        self.fetch("scopes", json!({"frameId": frame_id}), "scopes")
    }

    pub fn variables(
        &self,
        reference: i64,
    ) -> impl Future<Output = Result<Vec<Variable>, Error>> + use<> {
        self.fetch(
            "variables",
            json!({"variablesReference": reference}),
            "variables",
        )
    }

    pub fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<i64>,
        context: &str,
    ) -> impl Future<Output = Result<Evaluation, Error>> + use<> {
        let mut arguments = json!({"expression": expression, "context": context});
        if let Some(frame_id) = frame_id {
            arguments["frameId"] = json!(frame_id);
        }
        self.fetch("evaluate", arguments, "")
    }

    pub fn read_memory(
        &self,
        reference: &str,
        offset: i64,
        count: usize,
    ) -> impl Future<Output = Result<Memory, Error>> + use<> {
        self.fetch(
            "readMemory",
            json!({"memoryReference": reference, "offset": offset, "count": count}),
            "",
        )
    }

    pub fn step(
        &self,
        command: &str,
        thread_id: i64,
    ) -> impl Future<Output = Result<Value, Error>> + use<> {
        self.request(command, json!({"threadId": thread_id}))
    }

    pub fn disconnect(&self) -> impl Future<Output = Result<Value, Error>> + use<> {
        self.request(
            "disconnect",
            json!({"restart": false, "terminateDebuggee": true}),
        )
    }

    fn send(&self, message: Value) -> Result<(), Error> {
        if !self.is_running() {
            return Err(Error::new("the debug adapter stopped"));
        }
        self.inner
            .outgoing
            .try_send(message.to_string())
            .map_err(|_| Error::new("the debug adapter stopped"))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.outgoing.close();
        let Some(mut child) = self.child.lock().ok().and_then(|mut child| child.take()) else {
            return;
        };
        thread::spawn(move || {
            thread::sleep(SHUTDOWN_GRACE);
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

fn write_loop(payloads: Receiver<String>, mut stdin: impl Write + Send + 'static) {
    thread::spawn(move || {
        while let Ok(payload) = payloads.recv_blocking() {
            if transport::write_message(&mut stdin, &payload).is_err() {
                break;
            }
        }
    });
}

fn read_loop(
    mut stdout: impl io::BufRead + Send + 'static,
    pending: Pending,
    events: Sender<Event>,
    outgoing: Sender<String>,
    stopped: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let reason = loop {
            match transport::read_message(&mut stdout) {
                Ok(Some(payload)) => handle(&payload, &pending, &events, &outgoing),
                Ok(None) => break "the debug adapter exited".to_string(),
                Err(error) => break error.to_string(),
            }
        };
        stopped.store(true, Ordering::Relaxed);
        if let Ok(mut pending) = pending.lock() {
            pending.clear();
        }
        let _ = events.try_send(Event::Closed(reason));
        events.close();
    });
}

fn drain_stderr(stderr: impl Read + Send + 'static, log: Arc<Mutex<String>>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match io::BufRead::read_line(&mut reader, &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(mut log) = log.lock() {
                        log.push_str(&line);
                        if log.len() > STDERR_LIMIT {
                            let cut = log.len() - STDERR_LIMIT;
                            let cut = (cut..log.len())
                                .find(|ix| log.is_char_boundary(*ix))
                                .unwrap_or(log.len());
                            log.replace_range(..cut, "");
                        }
                    }
                }
            }
        }
    });
}

fn handle(payload: &str, pending: &Pending, events: &Sender<Event>, outgoing: &Sender<String>) {
    let Ok(message) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    match message.get("type").and_then(Value::as_str) {
        Some("response") => {
            let Some(seq) = message.get("request_seq").and_then(Value::as_i64) else {
                return;
            };
            let outcome = if message.get("success").and_then(Value::as_bool) == Some(true) {
                Ok(message.get("body").cloned().unwrap_or(Value::Null))
            } else {
                Err(Error::new(failure_message(&message)))
            };
            let sender = pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&seq));
            if let Some(sender) = sender {
                let _ = sender.try_send(outcome);
            }
        }
        Some("event") => {
            if let Some(event) = event(&message) {
                let _ = events.try_send(event);
            }
        }
        Some("request") => {
            let command = message.get("command").and_then(Value::as_str).unwrap_or("");
            let response = json!({
                "seq": 0,
                "type": "response",
                "request_seq": message.get("seq").cloned().unwrap_or(Value::Null),
                "command": command,
                "success": false,
                "message": format!("{command} is not supported"),
            });
            let _ = outgoing.try_send(response.to_string());
        }
        _ => {}
    }
}

fn failure_message(message: &Value) -> String {
    message
        .pointer("/body/error/format")
        .and_then(Value::as_str)
        .or_else(|| message.get("message").and_then(Value::as_str))
        .unwrap_or("the debug adapter reported an error")
        .to_string()
}

fn event(message: &Value) -> Option<Event> {
    let body = message.get("body").cloned().unwrap_or(Value::Null);
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
    Some(match message.get("event")?.as_str()? {
        "initialized" => Event::Initialized,
        "stopped" => Event::Stopped {
            reason: text("reason").unwrap_or_else(|| "pause".into()),
            description: text("description").or_else(|| text("text")),
            thread_id: body.get("threadId").and_then(Value::as_i64),
        },
        "continued" => Event::Continued,
        "output" => Event::Output {
            category: text("category").unwrap_or_else(|| "console".into()),
            output: text("output")?,
        },
        "exited" => Event::Exited(body.get("exitCode").and_then(Value::as_i64).unwrap_or(0)),
        "terminated" => Event::Terminated,
        _ => return None,
    })
}

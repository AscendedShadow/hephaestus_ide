pub mod transport;
mod uri;

#[cfg(test)]
#[path = "lsp_tests.rs"]
mod tests;

pub use uri::{path_to_uri, uri_to_path};

use std::{
    collections::HashMap,
    fmt,
    io::{self, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    thread,
    time::Duration,
};

use async_channel::{Receiver, Sender, unbounded};
use lsp_types::{
    InitializeParams, InitializeResult, ServerCapabilities, notification::Notification,
    request::Request,
};
use serde_json::{Value, json};

const STDERR_LIMIT: usize = 4096;
const SHUTDOWN_GRACE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub root: PathBuf,
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    Log(String),
    Stopped(String),
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
    capabilities: Mutex<Option<ServerCapabilities>>,
    stderr: Arc<Mutex<String>>,
    child: Mutex<Option<Child>>,
    next_id: AtomicI64,
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
        if options.root.is_dir() {
            command.current_dir(&options.root);
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
        let client = Self::connect(BufReader::new(stdout), stdin, options, log.clone());
        if let Ok(mut held) = client.inner.child.lock() {
            *held = Some(child);
        }
        drain_stderr(stderr, log);
        Ok(client)
    }

    fn connect(
        stdout: impl io::BufRead + Send + 'static,
        stdin: impl Write + Send + 'static,
        options: &Options,
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
            options.clone(),
            stopped.clone(),
        );
        Self {
            inner: Arc::new(Inner {
                outgoing,
                pending,
                events: incoming,
                capabilities: Mutex::new(None),
                stderr,
                child: Mutex::new(None),
                next_id: AtomicI64::new(1),
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

    pub fn capabilities(&self) -> Option<ServerCapabilities> {
        self.inner.capabilities.lock().ok()?.clone()
    }

    pub fn log(&self) -> String {
        self.inner
            .stderr
            .lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }

    pub fn request<R: Request>(
        &self,
        params: R::Params,
    ) -> impl Future<Output = Result<R::Result, Error>> + use<R> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = async_channel::bounded(1);
        let sent = match serde_json::to_value(&params) {
            Ok(params) => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.insert(id, sender);
                }
                self.send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": R::METHOD,
                    "params": params,
                }))
            }
            Err(error) => Err(Error::new(error.to_string())),
        };
        async move {
            sent?;
            let value = receiver
                .recv()
                .await
                .map_err(|_| Error::new("the language server stopped"))??;
            serde_json::from_value(value).map_err(|error| Error::new(error.to_string()))
        }
    }

    pub fn notify<N: Notification>(&self, params: N::Params) {
        let Ok(params) = serde_json::to_value(&params) else {
            return;
        };
        let _ = self.send(json!({
            "jsonrpc": "2.0",
            "method": N::METHOD,
            "params": params,
        }));
    }

    pub fn initialize(
        &self,
        options: &Options,
    ) -> impl Future<Output = Result<ServerCapabilities, Error>> + use<> {
        let root = uri::path_to_uri(&options.root);
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            initialization_options: options.initialization_options.clone(),
            capabilities: capabilities(),
            workspace_folders: root.map(|uri| {
                vec![lsp_types::WorkspaceFolder {
                    name: options
                        .root
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    uri,
                }]
            }),
            ..Default::default()
        };
        let request = self.request::<lsp_types::request::Initialize>(params);
        let client = self.clone();
        let settings = options.settings.clone();
        async move {
            let InitializeResult { capabilities, .. } = request.await?;
            if let Ok(mut stored) = client.inner.capabilities.lock() {
                *stored = Some(capabilities.clone());
            }
            client.notify::<lsp_types::notification::Initialized>(lsp_types::InitializedParams {});
            if let Some(settings) = settings {
                client.notify::<lsp_types::notification::DidChangeConfiguration>(
                    lsp_types::DidChangeConfigurationParams { settings },
                );
            }
            Ok(capabilities)
        }
    }

    fn send(&self, message: Value) -> Result<(), Error> {
        if !self.is_running() {
            return Err(Error::new("the language server stopped"));
        }
        self.inner
            .outgoing
            .try_send(message.to_string())
            .map_err(|_| Error::new("the language server stopped"))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.outgoing.try_send(
            json!({"jsonrpc": "2.0", "id": 0, "method": "shutdown", "params": null}).to_string(),
        );
        let _ = self
            .outgoing
            .try_send(json!({"jsonrpc": "2.0", "method": "exit"}).to_string());
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
    options: Options,
    stopped: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        loop {
            match transport::read_message(&mut stdout) {
                Ok(Some(payload)) => handle(&payload, &pending, &events, &outgoing, &options),
                Ok(None) => break,
                Err(error) => {
                    let _ = events.try_send(Event::Stopped(error.to_string()));
                    break;
                }
            }
        }
        stopped.store(true, Ordering::Relaxed);
        if let Ok(mut pending) = pending.lock() {
            pending.clear();
        }
        let _ = events.try_send(Event::Stopped("the language server exited".into()));
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

fn handle(
    payload: &str,
    pending: &Pending,
    events: &Sender<Event>,
    outgoing: &Sender<String>,
    options: &Options,
) {
    let Ok(message) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    let method = message.get("method").and_then(Value::as_str);
    match (message.get("id"), method) {
        (Some(id), Some(method)) => {
            let response = match reply(method, message.get("params"), options) {
                Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                None => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("{method} is not supported")},
                }),
            };
            let _ = outgoing.try_send(response.to_string());
        }
        (Some(id), None) => {
            let Some(id) = id.as_i64() else {
                return;
            };
            let outcome = match message.get("error") {
                Some(error) => Err(Error::new(
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("the language server reported an error"),
                )),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let sender = pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&id));
            if let Some(sender) = sender {
                let _ = sender.try_send(outcome);
            }
        }
        (None, Some(method)) => {
            if let Some(event) = notification(method, message.get("params")) {
                let _ = events.try_send(event);
            }
        }
        (None, None) => {}
    }
}

fn notification(method: &str, params: Option<&Value>) -> Option<Event> {
    let params = params?.clone();
    match method {
        lsp_types::notification::PublishDiagnostics::METHOD => {
            let params: lsp_types::PublishDiagnosticsParams =
                serde_json::from_value(params).ok()?;
            Some(Event::Diagnostics {
                path: uri::uri_to_path(&params.uri)?,
                diagnostics: params.diagnostics,
            })
        }
        lsp_types::notification::LogMessage::METHOD
        | lsp_types::notification::ShowMessage::METHOD => {
            Some(Event::Log(params.get("message")?.as_str()?.to_string()))
        }
        _ => None,
    }
}

fn reply(method: &str, params: Option<&Value>, options: &Options) -> Option<Value> {
    match method {
        "workspace/configuration" => {
            let settings = options.settings.clone().unwrap_or(Value::Null);
            let items = params
                .and_then(|params| params.get("items"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Some(Value::Array(
                items
                    .iter()
                    .map(|item| match item.get("section").and_then(Value::as_str) {
                        Some(section) => section_of(&settings, section).unwrap_or(Value::Null),
                        None => settings.clone(),
                    })
                    .collect(),
            ))
        }
        "workspace/workspaceFolders" => {
            let uri = uri::path_to_uri(&options.root)?;
            let name = options
                .root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some(json!([{ "uri": uri, "name": name }]))
        }
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "workspace/semanticTokens/refresh"
        | "workspace/inlayHint/refresh"
        | "workspace/diagnostic/refresh"
        | "workspace/codeLens/refresh" => Some(Value::Null),
        _ => None,
    }
}

fn section_of(settings: &Value, section: &str) -> Option<Value> {
    let mut value = settings;
    for key in section.split('.') {
        value = value.get(key)?;
    }
    Some(value.clone())
}

fn capabilities() -> lsp_types::ClientCapabilities {
    use lsp_types::*;
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            workspace_folders: Some(true),
            configuration: Some(true),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                did_save: Some(true),
                ..Default::default()
            }),
            hover: Some(HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                ..Default::default()
            }),
            definition: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    snippet_support: Some(false),
                    documentation_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                    resolve_support: Some(CompletionItemCapabilityResolveSupport {
                        properties: vec!["documentation".into(), "detail".into()],
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities::default()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

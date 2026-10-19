use std::{
    io::BufRead,
    path::PathBuf,
    sync::mpsc,
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Unpark(thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                assert!(Instant::now() < deadline, "the future never completed");
                thread::park_timeout(Duration::from_millis(20));
            }
        }
    }
}

fn respond(output: &mut impl Write, id: Option<Value>, result: Value) {
    let message = json!({"jsonrpc": "2.0", "id": id, "result": result});
    transport::write_message(output, &message.to_string()).unwrap();
}

fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    replies: mpsc::Sender<Value>,
    file: PathBuf,
) {
    while let Ok(Some(payload)) = transport::read_message(&mut input) {
        let message: Value = serde_json::from_str(&payload).unwrap();
        let id = message.get("id").cloned();
        match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                respond(
                    &mut output,
                    id,
                    json!({"capabilities": {"hoverProvider": true}}),
                );
                let request = json!({
                    "jsonrpc": "2.0",
                    "id": 500,
                    "method": "workspace/configuration",
                    "params": {"items": [{"section": "example.check"}, {"section": "missing"}]},
                });
                transport::write_message(&mut output, &request.to_string()).unwrap();
                let diagnostics = json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": {
                        "uri": path_to_uri(&file).unwrap(),
                        "diagnostics": [{
                            "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 6}},
                            "severity": 1,
                            "message": "cannot find value",
                        }],
                    },
                });
                transport::write_message(&mut output, &diagnostics.to_string()).unwrap();
            }
            Some("textDocument/hover") => respond(
                &mut output,
                id,
                json!({"contents": {"kind": "markdown", "value": "fn helper() -> i32"}}),
            ),
            Some("textDocument/definition") => {
                let message = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32603, "message": "no definition"},
                });
                transport::write_message(&mut output, &message.to_string()).unwrap();
            }
            Some("exit") => break,
            Some(_) => {
                if id.is_some() {
                    respond(&mut output, id, Value::Null);
                }
            }
            None => {
                let _ = replies.send(message);
            }
        }
    }
}

fn connect() -> (Client, Options, mpsc::Receiver<Value>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "").unwrap();
    let (server_input, client_output) = io::pipe().unwrap();
    let (client_input, server_output) = io::pipe().unwrap();
    let options = Options {
        program: "example-language-server".into(),
        root: root.clone(),
        settings: Some(json!({"example": {"check": {"enabled": true}}})),
        ..Default::default()
    };
    let (replies, received) = mpsc::channel();
    thread::spawn(move || serve(BufReader::new(server_input), server_output, replies, file));
    let client = Client::connect(
        BufReader::new(client_input),
        client_output,
        &options,
        Arc::default(),
    );
    (client, options, received, directory)
}

#[test]
fn a_session_initializes_answers_requests_and_reports_diagnostics() {
    let (client, options, replies, _directory) = connect();
    let capabilities = block_on(client.initialize(&options)).unwrap();
    assert_eq!(capabilities.hover_provider, Some(true.into()));
    assert_eq!(client.capabilities(), Some(capabilities));

    let hover = block_on(client.request::<lsp_types::request::HoverRequest>(
        lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: path_to_uri(&options.root.join("main.rs")).unwrap(),
                },
                position: lsp_types::Position::new(1, 2),
            },
            work_done_progress_params: Default::default(),
        },
    ))
    .unwrap()
    .unwrap();
    assert!(matches!(
        hover.contents,
        lsp_types::HoverContents::Markup(markup) if markup.value == "fn helper() -> i32"
    ));

    let failed = block_on(client.request::<lsp_types::request::GotoDefinition>(
        lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: path_to_uri(&options.root.join("main.rs")).unwrap(),
                },
                position: lsp_types::Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        },
    ));
    assert_eq!(failed, Err(Error("no definition".into())));

    let configuration = replies.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(configuration["id"], json!(500));
    assert_eq!(configuration["result"], json!([{"enabled": true}, null]));

    let event = block_on(client.events().recv()).unwrap();
    assert_eq!(
        event,
        Event::Diagnostics {
            path: options.root.join("main.rs"),
            diagnostics: vec![lsp_types::Diagnostic {
                range: lsp_types::Range::new(
                    lsp_types::Position::new(1, 2),
                    lsp_types::Position::new(1, 6)
                ),
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                message: "cannot find value".into(),
                ..Default::default()
            }],
        }
    );
}

#[test]
fn requests_fail_once_the_server_is_gone() {
    let (client, options, _replies, _directory) = connect();
    block_on(client.initialize(&options)).unwrap();
    client.notify::<lsp_types::notification::Exit>(());
    let event = block_on(async {
        loop {
            match client.events().recv().await {
                Ok(Event::Stopped(reason)) => return Some(reason),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    });
    assert_eq!(event.as_deref(), Some("the language server exited"));
    assert!(!client.is_running());
    let shutdown = block_on(client.request::<lsp_types::request::Shutdown>(()));
    assert_eq!(shutdown, Err(Error("the language server stopped".into())));
}

#[test]
fn messages_are_framed_with_their_byte_length() {
    let mut payload = Vec::new();
    transport::write_message(&mut payload, "{\"joy\":\"café 👋\"}").unwrap();
    let text = String::from_utf8(payload.clone()).unwrap();
    assert!(text.starts_with("Content-Length: 20\r\n\r\n"), "{text}");

    let mut reader = BufReader::new(payload.as_slice());
    assert_eq!(
        transport::read_message(&mut reader).unwrap().as_deref(),
        Some("{\"joy\":\"café 👋\"}")
    );
    assert_eq!(transport::read_message(&mut reader).unwrap(), None);

    let mut split = BufReader::new(
        "content-length: 2\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n{}".as_bytes(),
    );
    assert_eq!(
        transport::read_message(&mut split).unwrap().as_deref(),
        Some("{}")
    );
}

#[test]
fn paths_round_trip_through_file_uris() {
    let directory = tempfile::tempdir().unwrap();
    let directory = directory.path().canonicalize().unwrap().join("space dir");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("café.rs");
    std::fs::write(&path, "").unwrap();
    let uri = path_to_uri(&path).unwrap();
    let text = uri.as_str();
    assert!(text.starts_with("file:///"), "{text}");
    assert!(text.ends_with("/space%20dir/caf%C3%A9.rs"), "{text}");
    assert!(!text.contains('\\'), "{text}");
    assert_eq!(uri_to_path(&uri).unwrap(), path);
}

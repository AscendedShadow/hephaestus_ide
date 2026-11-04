//! An in-process, scriptable language server for workflow tests.

use std::{
    io::{self, BufReader, Write as _},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use gpui::VisualTestContext;
use serde_json::{Value, json};

use super::*;
use crate::settings::{ActiveLanguageServers, LanguageServer};

pub(super) type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + 'static>;

pub(super) struct MockServer {
    /// Notifications from the client and its responses to server requests.
    pub received: Arc<Mutex<Vec<Value>>>,
    output: Arc<Mutex<io::PipeWriter>>,
}

impl MockServer {
    /// Send a server-to-client request (e.g. `workspace/applyEdit`).
    pub fn request(&self, id: i64, method: &str, params: Value) {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut output = self.output.lock().unwrap();
        lsp::transport::write_message(&mut *output, &message.to_string()).unwrap();
        output.flush().unwrap();
    }

    /// Parameters of every notification or request the client sent with `method`.
    pub fn notifications(&self, method: &str) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message["method"] == method)
            .map(|message| message["params"].clone())
            .collect()
    }

    pub fn response(&self, id: i64) -> Option<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .find(|message| message["id"] == id && message.get("method").is_none())
            .cloned()
    }
}

/// Run the test executor until `condition` holds (the server answers from a real thread).
#[track_caller]
pub(super) fn wait_until(
    cx: &mut VisualTestContext,
    mut condition: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "condition not reached in time");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Install a mock server for `language` in the shell's workspace `root`.
pub(super) fn install(
    shell: &Entity<IdeShell>,
    language: &'static str,
    root: &Path,
    capabilities: Value,
    handler: Handler,
    cx: &mut VisualTestContext,
) -> MockServer {
    cx.executor().allow_parking();
    let (server_input, client_output) = io::pipe().unwrap();
    let (client_input, server_output) = io::pipe().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::new(Mutex::new(server_output));
    {
        let received = received.clone();
        let output = output.clone();
        thread::spawn(move || {
            let mut input = BufReader::new(server_input);
            while let Ok(Some(payload)) = lsp::transport::read_message(&mut input) {
                let message: Value = serde_json::from_str(&payload).unwrap();
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                received.lock().unwrap().push(message.clone());
                if let (Some(id), Some(method)) = (message.get("id").cloned(), method) {
                    {
                        let result = if method == "initialize" {
                            json!({"capabilities": capabilities})
                        } else {
                            handler(&method, &message["params"])
                        };
                        let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
                        let mut output = output.lock().unwrap();
                        if lsp::transport::write_message(&mut *output, &reply.to_string()).is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
    }
    let config = LanguageServer {
        languages: vec![language.to_string()],
        program: "mock-language-server".into(),
        ..Default::default()
    };
    let options = lsp::Options {
        program: config.program.clone(),
        root: root.to_path_buf(),
        ..Default::default()
    };
    let client =
        lsp::Client::connect_streams(BufReader::new(client_input), client_output, &options);
    cx.update(|_, cx| cx.set_global(ActiveLanguageServers(vec![config.clone()])));
    shell.update(cx, |shell, cx| {
        shell.servers.update(cx, |servers, cx| {
            servers.configure_for_test(vec![config.clone()], root.to_path_buf());
            servers.adopt(config.program.clone(), client, options, cx);
        });
    });
    wait_until(cx, |cx| {
        shell.read_with(cx, |shell, cx| {
            shell
                .servers
                .read(cx)
                .server_capabilities(language)
                .is_some()
        })
    });
    // Open documents are synchronized once the server is ready.
    shell.update(cx, |shell, cx| {
        for ix in 0..shell.buffers.len() {
            shell.sync_lsp_buffer(ix, cx);
        }
    });
    MockServer { received, output }
}

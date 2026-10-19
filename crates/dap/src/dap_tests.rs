use std::{
    io::BufRead,
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

fn write(output: &mut impl Write, message: Value) {
    transport::write_message(output, &message.to_string()).unwrap();
}

fn respond(output: &mut impl Write, request: &Value, body: Value) {
    write(
        output,
        json!({
            "seq": 0,
            "type": "response",
            "request_seq": request["seq"],
            "command": request["command"],
            "success": true,
            "body": body,
        }),
    );
}

fn serve(mut input: impl BufRead, mut output: impl Write, replies: mpsc::Sender<Value>) {
    while let Ok(Some(payload)) = transport::read_message(&mut input) {
        let message: Value = serde_json::from_str(&payload).unwrap();
        if message["type"] == "response" {
            let _ = replies.send(message);
            continue;
        }
        match message["command"].as_str().unwrap() {
            "initialize" => {
                respond(
                    &mut output,
                    &message,
                    json!({"supportsReadMemoryRequest": true}),
                );
                write(
                    &mut output,
                    json!({"seq": 0, "type": "event", "event": "initialized"}),
                );
                write(
                    &mut output,
                    json!({"seq": 0, "type": "request", "seq": 900, "command": "runInTerminal", "arguments": {}}),
                );
            }
            "setBreakpoints" => {
                let lines: Vec<Value> = message["arguments"]["breakpoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|breakpoint| json!({"verified": true, "line": breakpoint["line"]}))
                    .collect();
                respond(&mut output, &message, json!({"breakpoints": lines}));
            }
            "configurationDone" => {
                respond(&mut output, &message, Value::Null);
                write(
                    &mut output,
                    json!({"seq": 0, "type": "event", "event": "stopped", "body": {"reason": "breakpoint", "threadId": 7}}),
                );
            }
            "stackTrace" => respond(
                &mut output,
                &message,
                json!({"stackFrames": [{
                    "id": 1000,
                    "name": "main",
                    "line": 3,
                    "column": 5,
                    "source": {"name": "main.c", "path": "/src/main.c"},
                }]}),
            ),
            "variables" => respond(
                &mut output,
                &message,
                json!({"variables": [{
                    "name": "count",
                    "value": "42",
                    "type": "int",
                    "variablesReference": 0,
                    "memoryReference": "0x1000",
                }]}),
            ),
            "readMemory" => respond(
                &mut output,
                &message,
                json!({"address": "0x1000", "data": "AAECAw=="}),
            ),
            "evaluate" => {
                write(
                    &mut output,
                    json!({
                        "seq": 0,
                        "type": "response",
                        "request_seq": message["seq"],
                        "command": "evaluate",
                        "success": false,
                        "message": "evaluate failed",
                        "body": {"error": {"id": 1, "format": "no symbol \"nope\""}},
                    }),
                );
            }
            "disconnect" => {
                respond(&mut output, &message, Value::Null);
                break;
            }
            _ => respond(&mut output, &message, Value::Null),
        }
    }
}

fn connect() -> (Client, mpsc::Receiver<Value>) {
    let (server_input, client_output) = io::pipe().unwrap();
    let (client_input, server_output) = io::pipe().unwrap();
    let (replies, received) = mpsc::channel();
    thread::spawn(move || serve(BufReader::new(server_input), server_output, replies));
    let client = Client::connect(BufReader::new(client_input), client_output, Arc::default());
    (client, received)
}

fn next_event(client: &Client) -> Event {
    block_on(client.events().recv()).unwrap()
}

#[test]
fn a_session_stops_at_a_breakpoint_and_reads_variables_and_memory() {
    let (client, replies) = connect();
    let capabilities = block_on(client.initialize("lldb-dap")).unwrap();
    assert_eq!(capabilities["supportsReadMemoryRequest"], json!(true));
    assert_eq!(next_event(&client), Event::Initialized);

    let refused = replies.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(refused["request_seq"], json!(900));
    assert_eq!(refused["success"], json!(false));

    let breakpoints = block_on(client.set_breakpoints(Path::new("/src/main.c"), &[3, 9])).unwrap();
    assert_eq!(
        breakpoints
            .iter()
            .map(|breakpoint| (breakpoint.verified, breakpoint.line))
            .collect::<Vec<_>>(),
        vec![(true, Some(3)), (true, Some(9))]
    );

    block_on(client.request("configurationDone", Value::Null)).unwrap();
    assert_eq!(
        next_event(&client),
        Event::Stopped {
            reason: "breakpoint".into(),
            description: None,
            thread_id: Some(7),
        }
    );

    let frames = block_on(client.stack_trace(7)).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].name, "main");
    assert_eq!(frames[0].line, 3);
    assert_eq!(frames[0].path(), Some(&PathBuf::from("/src/main.c")));

    let variables = block_on(client.variables(1)).unwrap();
    assert_eq!(variables[0].name, "count");
    assert_eq!(variables[0].type_name.as_deref(), Some("int"));
    assert_eq!(variables[0].memory_reference.as_deref(), Some("0x1000"));

    let memory = block_on(client.read_memory("0x1000", 0, 4)).unwrap();
    assert_eq!(memory.start(), Some(0x1000));
    assert_eq!(memory.bytes(), vec![0, 1, 2, 3]);

    let failed = block_on(client.evaluate("nope", Some(1000), "repl"));
    assert_eq!(failed, Err(Error("no symbol \"nope\"".into())));

    block_on(client.disconnect()).unwrap();
    assert!(matches!(next_event(&client), Event::Closed(_)));
    assert!(!client.is_running());
    assert_eq!(
        block_on(client.threads()),
        Err(Error("the debug adapter stopped".into()))
    );
}

#[test]
fn base64_decodes_padded_and_unpadded_data() {
    assert_eq!(decode_base64("aGVsbG8="), b"hello");
    assert_eq!(decode_base64("aGVsbG8"), b"hello");
    assert_eq!(decode_base64("/+8="), vec![0xff, 0xef]);
    assert_eq!(decode_base64(""), Vec::<u8>::new());
}

#[test]
fn addresses_parse_from_hex_or_decimal() {
    assert_eq!(parse_address("0x7ffe10"), Some(0x7ffe10));
    assert_eq!(parse_address(" 4096 "), Some(4096));
    assert_eq!(parse_address("main"), None);
}

use std::{
    io::{self, BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
};

use async_channel::{Receiver, Sender, bounded, unbounded};
use serde_json::{Value, json};

use crate::settings::RunConfig;

pub struct Client {
    child: Child,
    outgoing: Sender<Value>,
    sequence: i64,
    pub events: Receiver<Value>,
}

impl Client {
    pub fn start(config: &RunConfig, root: &Path) -> io::Result<Self> {
        let mut child = Command::new(&config.program)
            .args(&config.args)
            .current_dir(config.working_directory.as_deref().unwrap_or(root))
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Language server has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Language server has no stdout"))?;
        let (sender, events) = unbounded();
        let (outgoing, queued) = bounded::<Value>(64);
        thread::spawn(move || {
            while let Ok(message) = queued.recv_blocking() {
                let Ok(bytes) = serde_json::to_vec(&message) else {
                    continue;
                };
                if write!(stdin, "Content-Length: {}\r\n\r\n", bytes.len()).is_err()
                    || stdin.write_all(&bytes).is_err()
                    || stdin.flush().is_err()
                {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(message)) = read_message(&mut reader) {
                if sender.send_blocking(message).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            outgoing,
            sequence: 1,
            events,
        };
        client.write(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "processId": std::process::id(), "rootUri": uri(root),
                "capabilities": {"textDocument": {"publishDiagnostics": {}, "synchronization": {},
                    "completion": {"completionItem": {"snippetSupport": false}}}},
                "clientInfo": {"name": "Hephaestus"}
            }
        }))?;
        Ok(client)
    }

    pub fn initialized(&mut self) -> io::Result<()> {
        self.write(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))
    }

    pub fn respond(&mut self, id: Value, result: Value) -> io::Result<()> {
        self.write(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }

    pub fn open(&mut self, path: &Path, text: &str, version: i64) -> io::Result<()> {
        self.write(
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
                "textDocument":{"uri":uri(path),"languageId":"rust","version":version,"text":text}
            }}),
        )
    }

    pub fn change(&mut self, path: &Path, text: &str, version: i64) -> io::Result<()> {
        self.write(
            json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
                "textDocument":{"uri":uri(path),"version":version},"contentChanges":[{"text":text}]
            }}),
        )
    }

    pub fn close(&mut self, path: &Path) -> io::Result<()> {
        self.write(
            json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{
                "textDocument":{"uri":uri(path)}
            }}),
        )
    }

    pub fn saved(&mut self, path: &Path) -> io::Result<()> {
        self.write(
            json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{
                "textDocument":{"uri":uri(path)}
            }}),
        )
    }

    pub fn definition(&mut self, path: &Path, line: u32, character: u32) -> io::Result<i64> {
        self.sequence += 1;
        let id = self.sequence;
        self.write(
            json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
                "textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}
            }}),
        )?;
        Ok(id)
    }

    pub fn hover(&mut self, path: &Path, line: u32, character: u32) -> io::Result<i64> {
        self.sequence += 1;
        let id = self.sequence;
        self.write(
            json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
                "textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}
            }}),
        )?;
        Ok(id)
    }

    pub fn completion(&mut self, path: &Path, line: u32, character: u32) -> io::Result<i64> {
        self.sequence += 1;
        let id = self.sequence;
        self.write(
            json!({"jsonrpc":"2.0","id":id,"method":"textDocument/completion","params":{
                "textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}
            }}),
        )?;
        Ok(id)
    }

    fn write(&mut self, message: Value) -> io::Result<()> {
        self.outgoing.try_send(message).map_err(io::Error::other)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(target_os = "windows")]
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let path = path.replace('\\', "/");
    let path = if cfg!(target_os = "windows") {
        format!("/{path}")
    } else {
        path
    };
    let mut encoded = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn path_from_uri(uri: &str) -> Option<std::path::PathBuf> {
    let source = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let mut source = source.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let hi = (source.next()? as char).to_digit(16)?;
            let lo = (source.next()? as char).to_digit(16)?;
            bytes.push((hi * 16 + lo) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    #[cfg(target_os = "windows")]
    let path = path.strip_prefix('/').unwrap_or(&path).replace('/', "\\");
    let path: std::path::PathBuf = path.into();
    Some(path.canonicalize().unwrap_or(path))
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.trim().strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or_else(|| io::Error::other("Missing LSP Content-Length"))?;
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::other("LSP message too large"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_framed_messages_and_round_trips_uri() {
        let data = b"Content-Length: 7\r\n\r\n{\"a\":1}";
        assert_eq!(read_message(&mut &data[..]).unwrap().unwrap()["a"], 1);
        let path = std::path::Path::new(if cfg!(windows) {
            "C:\\my files\\é.rs"
        } else {
            "/my files/é.rs"
        });
        assert_eq!(path_from_uri(&uri(path)).as_deref(), Some(path));
    }
}

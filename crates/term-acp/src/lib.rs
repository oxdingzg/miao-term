//! A minimal Agent Client Protocol (ACP) client (ADR 0040, M7 A2).
//!
//! ACP is JSON-RPC 2.0 over the agent subprocess's stdio, one message per
//! newline. This crate launches an agent, sends `initialize`, `session/new`,
//! `session/prompt` and `session/cancel`, answers the agent's `fs/*` and
//! `session/request_permission` requests, and forwards `session/update`
//! notifications and responses to a [`Handler`]. It has no GPU and no host
//! dependency, so it is unit-tested against a fake agent.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use serde_json::{json, Value};

/// What the client does when the agent talks to it.
pub trait Handler: Send + Sync + 'static {
    /// A `session/update` notification's `update` object.
    fn on_update(&self, session: &str, update: &Value) {
        let _ = (session, update);
    }
    /// A response to one of our requests.
    fn on_response(&self, id: u64, result: &Value, error: Option<&Value>) {
        let _ = (id, result, error);
    }
    /// The transport failed or the agent sent unparsable data.
    fn on_error(&self, message: &str) {
        let _ = message;
    }
    /// `fs/read_text_file`: the file's text, or `None` to refuse.
    fn read_text_file(&self, path: &str) -> Option<String> {
        let _ = path;
        None
    }
    /// `fs/write_text_file`.
    fn write_text_file(&self, path: &str, content: &str) -> bool {
        let _ = (path, content);
        false
    }
    /// `session/request_permission`: whether to allow the requested action.
    fn request_permission(&self, request: &Value) -> bool {
        let _ = request;
        false
    }
}

/// A connection to one ACP agent.
pub struct Client {
    out: mpsc::Sender<String>,
    next_id: AtomicU64,
    handler: Arc<dyn Handler>,
    child: Option<Child>,
}

impl Client {
    /// Launch `command` (with `args`/`env`, in `cwd`) and speak ACP to it.
    pub fn spawn(
        command: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        env: &[(String, String)],
        handler: Arc<dyn Handler>,
    ) -> std::io::Result<Client> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut client = Client::over(stdout, stdin, handler);
        client.child = Some(child);
        Ok(client)
    }

    /// Speak ACP over any reader/writer pair (tests, or a remote transport).
    pub fn over<R, W>(reader: R, mut writer: W, handler: Arc<dyn Handler>) -> Client
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let (out, rx) = mpsc::channel::<String>();
        // The writer thread owns the agent's stdin.
        std::thread::spawn(move || {
            for line in rx {
                if writer.write_all(line.as_bytes()).is_err() {
                    break;
                }
                if writer.write_all(b"\n").is_err() || writer.flush().is_err() {
                    break;
                }
            }
        });
        let reply = Writer(out.clone());
        let h = handler.clone();
        std::thread::spawn(move || reader_loop(Box::new(reader), h, reply));
        Client {
            out,
            next_id: AtomicU64::new(1),
            handler,
            child: None,
        }
    }

    /// Send a request and return its id (the response arrives at
    /// [`Handler::on_response`]).
    pub fn request(&self, method: &str, params: Value) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    /// Send a notification (no response).
    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Negotiate the protocol version and capabilities.
    pub fn initialize(&self, name: &str, version: &str) -> u64 {
        self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": false,
                },
                "clientInfo": { "name": name, "version": version },
            }),
        )
    }

    /// Start a session rooted at `cwd` (an absolute path).
    pub fn new_session(&self, cwd: &str) -> u64 {
        self.request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))
    }

    /// Send a prompt to a session.
    pub fn prompt(&self, session: &str, text: &str) -> u64 {
        self.request(
            "session/prompt",
            json!({
                "sessionId": session,
                "prompt": [{ "type": "text", "text": text }],
            }),
        )
    }

    /// Ask the agent to stop the current turn.
    pub fn cancel(&self, session: &str) {
        self.notify("session/cancel", json!({ "sessionId": session }));
    }

    pub fn handler(&self) -> &Arc<dyn Handler> {
        &self.handler
    }

    fn send(&self, message: Value) {
        // serde_json never emits a bare newline, so a message is one line.
        let _ = self.out.send(message.to_string());
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Clone)]
struct Writer(mpsc::Sender<String>);

impl Writer {
    fn raw(&self, v: Value) {
        let _ = self.0.send(v.to_string());
    }
    fn result(&self, id: &Value, result: Value) {
        self.raw(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }
    fn error(&self, id: &Value, code: i64, message: &str) {
        self.raw(
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        );
    }
}

fn reader_loop(mut reader: Box<dyn Read + Send>, handler: Arc<dyn Handler>, writer: Writer) {
    let mut buf = String::new();
    let mut bytes = [0u8; 8192];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                buf.push_str(&String::from_utf8_lossy(&bytes[..n]));
                while let Some(pos) = buf.find('\n') {
                    let line: String = buf.drain(..=pos).collect();
                    let line = line.trim();
                    if !line.is_empty() {
                        dispatch(line, &handler, &writer);
                    }
                }
            }
            Err(e) => {
                handler.on_error(&format!("read failed: {e}"));
                break;
            }
        }
    }
}

fn dispatch(line: &str, handler: &Arc<dyn Handler>, writer: &Writer) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        handler.on_error("invalid JSON from the agent");
        return;
    };
    let method = v.get("method").and_then(Value::as_str);
    match (method, v.get("id")) {
        // A request from the agent: answer it.
        (Some(method), Some(id)) => {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            match method {
                "fs/read_text_file" => {
                    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
                    match handler.read_text_file(path) {
                        Some(content) => writer.result(id, json!({ "content": content })),
                        None => writer.error(id, -32000, "file not readable"),
                    }
                }
                "fs/write_text_file" => {
                    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
                    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
                    if handler.write_text_file(path, content) {
                        writer.result(id, json!({}));
                    } else {
                        writer.error(id, -32000, "file not writable");
                    }
                }
                "session/request_permission" => {
                    // ACP wants an outcome; we only ever select or cancel.
                    let outcome = if handler.request_permission(&params) {
                        json!({ "outcome": "selected" })
                    } else {
                        json!({ "outcome": "cancelled" })
                    };
                    writer.result(id, json!({ "outcome": outcome }));
                }
                _ => writer.error(id, -32601, "method not found"),
            }
        }
        // A notification: forward streamed updates.
        (Some("session/update"), None) => {
            if let Some(update) = v.pointer("/params/update") {
                let session = v
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                handler.on_update(session, update);
            }
        }
        (Some(_), None) => {}
        // A response to one of our requests.
        (None, Some(id)) => {
            if let Some(id) = id.as_u64() {
                let result = v.get("result").cloned().unwrap_or(Value::Null);
                let error = v.get("error").cloned();
                handler.on_response(id, &result, error.as_ref());
            }
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::time::Duration;

    #[derive(Default)]
    struct Recorder {
        tx: std::sync::Mutex<Option<mpsc::Sender<String>>>,
    }

    impl Recorder {
        fn new() -> (Arc<Self>, mpsc::Receiver<String>) {
            let (tx, rx) = mpsc::channel();
            let rec = Arc::new(Recorder {
                tx: std::sync::Mutex::new(Some(tx)),
            });
            (rec, rx)
        }
        fn send(&self, message: String) {
            if let Some(tx) = &*self.tx.lock().unwrap() {
                let _ = tx.send(message);
            }
        }
    }

    impl Handler for Recorder {
        fn on_update(&self, session: &str, update: &Value) {
            self.send(format!(
                "update:{session}:{}",
                update
                    .get("sessionUpdate")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ));
        }
        fn on_response(&self, id: u64, result: &Value, _error: Option<&Value>) {
            self.send(format!(
                "response:{id}:{}",
                result
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ));
        }
        fn read_text_file(&self, path: &str) -> Option<String> {
            (path == "/tmp/x").then(|| "hello".to_string())
        }
    }

    // Loopback TCP provides the same bidirectional byte stream on every OS.
    fn streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (agent, _) = listener.accept().unwrap();
        (client, agent)
    }

    #[test]
    fn initialize_and_updates_round_trip() {
        let (client_stream, agent_stream) = streams();
        let (rec, rx) = Recorder::new();
        let client = Client::over(client_stream.try_clone().unwrap(), client_stream, rec);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(agent_stream.try_clone().unwrap());
            let mut writer = agent_stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let v: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(v["method"], "initialize");
            let id = v["id"].clone();
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": 1 } })
            )
            .unwrap();
            writeln!(
                writer,
                "{}",
                json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "s1",
                        "update": { "sessionUpdate": "agent_message_chunk" }
                    }
                })
            )
            .unwrap();
        });
        client.initialize("mtty", "0.0.16");
        let mut got = Vec::new();
        for _ in 0..2 {
            got.push(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        assert!(got.iter().any(|m| m == "response:1:1"), "{got:?}");
        assert!(
            got.iter().any(|m| m == "update:s1:agent_message_chunk"),
            "{got:?}"
        );
    }

    #[test]
    fn answers_an_fs_read_request() {
        let (client_stream, agent_stream) = streams();
        let (rec, _rx) = Recorder::new();
        let _client = Client::over(client_stream.try_clone().unwrap(), client_stream, rec);
        let mut writer = agent_stream.try_clone().unwrap();
        let mut reader = BufReader::new(agent_stream);
        writeln!(
            writer,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "fs/read_text_file",
                "params": { "path": "/tmp/x" }
            })
        )
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"]["content"], "hello");
    }
}

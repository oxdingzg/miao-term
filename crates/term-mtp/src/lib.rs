//! `miao-term-mtp` — the MTP control plane.
//!
//! A newline-delimited JSON server over a Unix socket (Windows named pipe is a
//! later transport). The envelope matches the existing `miaotty` MTP contract so
//! `miaotty-cli`, plugins and agent hooks keep working unchanged.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{prelude::*, ListenerOptions};
use interprocess::TryClone;

/// Maximum size of a single newline-delimited request. Lines longer than this
/// are treated as hostile and the connection is dropped.
const MAX_LINE: usize = 1 << 20;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTO_VERSION: i64 = 1;
/// Largest file the control plane will read or write in one request.
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

pub const HOST_CAPS: &[&str] = &[
    "core.basic",
    "app.view.write",
    "file.read",
    "file.write",
    "agent.state.read",
    "agent.state.write",
    "history.read",
    "history.write",
];

#[derive(Debug, Deserialize)]
pub struct Request {
    pub v: i64,
    pub id: i64,
    pub kind: String,
    pub ns: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub v: i64,
    pub id: i64,
    pub kind: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl Response {
    fn ok(id: i64, revision: i64, result: Value) -> Self {
        Self {
            v: PROTO_VERSION,
            id,
            kind: "res",
            ok: true,
            result: Some(result),
            error: None,
            revision: Some(revision),
        }
    }

    fn err(id: i64, revision: i64, code: &str, message: impl Into<String>) -> Self {
        Self {
            v: PROTO_VERSION,
            id,
            kind: "res",
            ok: false,
            result: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.into(),
                retryable: false,
            }),
            revision: Some(revision),
        }
    }
}

/// Per-user runtime directory for the control-plane socket.
///
/// Prefers `$XDG_RUNTIME_DIR` (Linux, already mode 0700 and user-owned) and
/// otherwise falls back to `$TMPDIR` (per-user on macOS). This keeps the
/// socket off shared, world-writable locations such as `/tmp` whenever the
/// platform offers a private directory.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return path;
        }
    }
    std::env::temp_dir()
}

/// Default socket path (`$XDG_RUNTIME_DIR/miaotty.sock` or `$TMPDIR/miaotty.sock`),
/// shared with `miaotty-cli`.
pub fn default_socket() -> PathBuf {
    runtime_dir().join("miaotty.sock")
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Shared, in-memory server state: agent states + command history.
#[derive(Default)]
pub struct ServerState {
    revision: AtomicI64,
    seq: AtomicI64,
    states: Mutex<BTreeMap<String, Value>>,
    history: Mutex<BTreeMap<String, Vec<Value>>>,
    panes: Mutex<Vec<Value>>,
    writes: Mutex<Vec<(String, Vec<u8>)>>,
    commands: Mutex<Vec<Command>>,
}

/// UI-side control actions queued by MTP methods.
#[derive(Debug, Clone)]
pub enum Command {
    Focus(String),
    Close(String),
    /// Open a file in the reader.
    View(String),
    /// Open a file in the editor.
    Edit(String),
}

impl ServerState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn bump(&self) -> i64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn revision(&self) -> i64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Replace the advertised pane list (called by the app on tab changes).
    pub fn set_panes(&self, panes: Vec<Value>) {
        *self.panes.lock().unwrap() = panes;
    }

    /// Command history for a pane, oldest → newest.
    pub fn history_for(&self, pane_id: &str) -> Vec<Value> {
        let k = format!("pane:{pane_id}");
        self.history
            .lock()
            .unwrap()
            .get(&k)
            .cloned()
            .unwrap_or_default()
    }

    /// Agent state entry for a pane, if any.
    pub fn agent_for(&self, pane_id: &str) -> Option<Value> {
        let k = format!("pane:{pane_id}");
        self.states.lock().unwrap().get(&k).cloned()
    }

    fn queue_write(&self, pane_id: String, data: Vec<u8>) {
        self.writes.lock().unwrap().push((pane_id, data));
    }

    /// Take queued writes for the UI to feed into the panes.
    pub fn take_writes(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut *self.writes.lock().unwrap())
    }

    fn queue_command(&self, command: Command) {
        self.commands.lock().unwrap().push(command);
    }

    /// Take queued focus/close commands for the UI to apply.
    pub fn take_commands(&self) -> Vec<Command> {
        std::mem::take(&mut *self.commands.lock().unwrap())
    }
}

fn key(pane: Option<&str>, tty: Option<&str>, extra: &str) -> String {
    if let Some(p) = pane.filter(|s| !s.is_empty()) {
        return format!("pane:{p}");
    }
    if let Some(t) = tty.filter(|s| !s.is_empty()) {
        return format!("tty:{t}");
    }
    format!("global:{extra}")
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

fn dispatch(state: &ServerState, req: Request) -> Response {
    let id = req.id;
    let rev = state.revision();
    let params = req.params.clone().unwrap_or(Value::Null);
    match (req.ns.as_str(), req.method.as_str()) {
        ("core", "ping") => Response::ok(
            id,
            rev,
            json!({
                "proto": PROTO_VERSION,
                "app_version": concat!("miaotty/", env!("CARGO_PKG_VERSION")),
                "pid": std::process::id(),
                "caps": HOST_CAPS,
            }),
        ),
        ("core", "health") => Response::ok(id, rev, json!({ "ok": true, "revision": rev })),
        ("pane", "list") => {
            let panes = state.panes.lock().unwrap().clone();
            Response::ok(id, rev, json!({ "panes": panes }))
        }
        ("pane", "send") | ("pane", "run") => {
            let pane = str_field(&params, "pane_id").unwrap_or_default();
            let data = str_field(&params, "data").unwrap_or_default();
            if pane.is_empty() {
                return Response::err(id, rev, "no_pane", "pane_id is required");
            }
            let bytes = if req.method == "run" {
                format!("{data}\r").into_bytes()
            } else {
                data.into_bytes()
            };
            state.queue_write(pane, bytes);
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("pane", "focus") | ("pane", "close") => {
            let pane = str_field(&params, "pane_id").unwrap_or_default();
            if pane.is_empty() {
                return Response::err(id, rev, "no_pane", "pane_id is required");
            }
            let command = if req.method == "focus" {
                Command::Focus(pane)
            } else {
                Command::Close(pane)
            };
            state.queue_command(command);
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("app", "view") | ("app", "edit") => {
            let path = str_field(&params, "path").unwrap_or_default();
            if path.is_empty() {
                return Response::err(id, rev, "no_path", "path is required");
            }
            let command = if req.method == "view" {
                Command::View(path)
            } else {
                Command::Edit(path)
            };
            state.queue_command(command);
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("file", "read") => {
            let path = str_field(&params, "path").unwrap_or_default();
            if path.is_empty() {
                return Response::err(id, rev, "no_path", "path is required");
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let truncated = bytes.len() > MAX_FILE_BYTES;
                    let slice = &bytes[..bytes.len().min(MAX_FILE_BYTES)];
                    Response::ok(
                        id,
                        rev,
                        json!({
                            "data": String::from_utf8_lossy(slice),
                            "bytes": bytes.len(),
                            "truncated": truncated,
                        }),
                    )
                }
                Err(e) => Response::err(id, rev, "io_error", e.to_string()),
            }
        }
        ("file", "write") => {
            let path = str_field(&params, "path").unwrap_or_default();
            let data = str_field(&params, "data").unwrap_or_default();
            if path.is_empty() {
                return Response::err(id, rev, "no_path", "path is required");
            }
            match std::fs::write(&path, data.as_bytes()) {
                Ok(()) => Response::ok(id, rev, json!({ "ok": true, "bytes": data.len() })),
                Err(e) => Response::err(id, rev, "io_error", e.to_string()),
            }
        }
        ("agent", "state.set") => {
            let pane = str_field(&params, "pane_id");
            let tty = str_field(&params, "tty");
            let agent = str_field(&params, "agent").unwrap_or_default();
            let k = key(pane.as_deref(), tty.as_deref(), &agent);
            let seq = state.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let mut entry = params.clone();
            if let Value::Object(ref mut m) = entry {
                m.insert("seq".into(), json!(seq));
            }
            let revision = state.bump();
            state.states.lock().unwrap().insert(k, entry);
            Response::ok(id, revision, json!({ "revision": revision }))
        }
        ("agent", "state.list") => {
            let states: Vec<Value> = state.states.lock().unwrap().values().cloned().collect();
            Response::ok(id, rev, json!({ "revision": rev, "states": states }))
        }
        ("history", "add") => {
            let pane = str_field(&params, "pane_id");
            let tty = str_field(&params, "tty");
            let k = key(pane.as_deref(), tty.as_deref(), "history");
            let seq = state.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let mut entry = params.clone();
            if let Value::Object(ref mut m) = entry {
                m.insert("seq".into(), json!(seq));
                m.entry("ts").or_insert(json!(now_ms()));
            }
            let revision = state.bump();
            let mut history = state.history.lock().unwrap();
            let list = history.entry(k).or_default();
            list.push(entry);
            if list.len() > 1000 {
                let excess = list.len() - 1000;
                list.drain(..excess);
            }
            Response::ok(id, revision, json!({ "revision": revision }))
        }
        ("history", "list") => {
            let pane = str_field(&params, "pane_id");
            let tty = str_field(&params, "tty");
            let k = key(pane.as_deref(), tty.as_deref(), "history");
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let history = state.history.lock().unwrap();
            let list = history.get(&k).cloned().unwrap_or_default();
            let list = match limit {
                Some(n) if list.len() > n => list[list.len() - n..].to_vec(),
                _ => list,
            };
            Response::ok(id, rev, json!({ "revision": rev, "entries": list }))
        }
        _ => Response::err(
            id,
            rev,
            "bad_request",
            format!("unknown method {}.{}", req.ns, req.method),
        ),
    }
}

/// Read one request line, bounded to [`MAX_LINE`]. Returns `Ok(false)` at EOF
/// or when a client sends an oversized line, in which case the connection is
/// dropped so a single client cannot make us allocate unbounded memory.
fn read_request(reader: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    let n = reader
        .by_ref()
        .take(MAX_LINE as u64)
        .read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(false);
    }
    if buf.len() >= MAX_LINE && buf.last() != Some(&b'\n') {
        return Ok(false);
    }
    Ok(true)
}

fn write_response(writer: &mut impl Write, response: &Response) -> bool {
    match serde_json::to_string(response) {
        Ok(mut encoded) => {
            encoded.push('\n');
            writer.write_all(encoded.as_bytes()).is_ok() && writer.flush().is_ok()
        }
        Err(_) => false,
    }
}

fn handle(stream: LocalSocketStream, state: Arc<ServerState>) {
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    let mut writer = stream;
    let mut buf = Vec::new();
    while let Ok(true) = read_request(&mut reader, &mut buf) {
        let response = match std::str::from_utf8(&buf) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Request>(line) {
                    Ok(req) => dispatch(&state, req),
                    Err(e) => Response::err(0, state.revision(), "bad_request", e.to_string()),
                }
            }
            Err(_) => Response::err(
                0,
                state.revision(),
                "bad_request",
                "request must be valid UTF-8",
            ),
        };
        if !write_response(&mut writer, &response) {
            break;
        }
    }
}

/// A minimal MTP client (cross-platform via `interprocess`).
pub mod client {
    use super::{ErrorInfo, PROTO_VERSION};
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;

    #[cfg(unix)]
    use interprocess::local_socket::GenericFilePath;
    #[cfg(windows)]
    use interprocess::local_socket::GenericNamespaced;
    use interprocess::local_socket::{prelude::*, ConnectOptions};
    use interprocess::TryClone;
    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Deserialize)]
    struct Reply {
        ok: bool,
        result: Option<Value>,
        error: Option<ErrorInfo>,
    }

    pub struct Client {
        reader: BufReader<LocalSocketStream>,
        writer: LocalSocketStream,
        next_id: i64,
    }

    /// Connect to the host at `path` (Unix socket path; ignored on Windows,
    /// which uses the fixed `miaotty` named pipe).
    pub fn connect(path: &Path) -> std::io::Result<Client> {
        #[cfg(unix)]
        let name = path.to_fs_name::<GenericFilePath>()?;
        #[cfg(windows)]
        let name = "miaotty".to_ns_name::<GenericNamespaced>()?;
        let stream = ConnectOptions::new().name(name).connect_sync()?;
        let writer = stream.try_clone()?;
        Ok(Client {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
        })
    }

    impl Client {
        /// Send a request and return the result value (or an error).
        pub fn call(&mut self, ns: &str, method: &str, params: Value) -> std::io::Result<Value> {
            let id = self.next_id;
            self.next_id += 1;
            let req = serde_json::json!({
                "v": PROTO_VERSION, "id": id, "kind": "req",
                "ns": ns, "method": method, "params": params,
            });
            let mut line = req.to_string();
            line.push('\n');
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;

            let mut buf = String::new();
            self.reader.read_line(&mut buf)?;
            let reply: Reply = serde_json::from_str(buf.trim())
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            if reply.ok {
                Ok(reply.result.unwrap_or(Value::Null))
            } else {
                let msg = reply
                    .error
                    .map(|e| format!("[{}] {}", e.code, e.message))
                    .unwrap_or_else(|| "error".to_string());
                Err(std::io::Error::other(msg))
            }
        }
    }
}

/// Bind `path` and serve connections on a background thread.
pub fn serve(path: &Path, state: Arc<ServerState>) -> std::io::Result<()> {
    #[cfg(unix)]
    let name = {
        let _ = std::fs::remove_file(path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        path.to_fs_name::<GenericFilePath>()?
    };
    #[cfg(windows)]
    let name = "miaotty".to_ns_name::<GenericNamespaced>()?;

    let listener = ListenerOptions::new().name(name).create_sync()?;

    // Restrict the endpoint to its owner. `pane.run`/`pane.send` execute
    // commands in the user's shell, so other local users must not connect.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || handle(stream, state));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_a_normal_request() {
        let mut reader = BufReader::new(Cursor::new(b"{\"v\":1}\n".to_vec()));
        let mut buf = Vec::new();
        assert!(matches!(read_request(&mut reader, &mut buf), Ok(true)));
        assert_eq!(buf, b"{\"v\":1}\n");
    }

    fn request(ns: &str, method: &str, params: Value) -> Request {
        Request {
            v: PROTO_VERSION,
            id: 1,
            kind: "req".to_string(),
            ns: ns.to_string(),
            method: method.to_string(),
            params: Some(params),
        }
    }

    #[test]
    fn file_read_and_write_round_trip() {
        let state = ServerState::new();
        let path = std::env::temp_dir().join(format!("miaotty-mtp-file-{}", std::process::id()));
        let path_str = path.to_string_lossy().to_string();

        let write = dispatch(
            &state,
            request(
                "file",
                "write",
                json!({ "path": path_str, "data": "hello" }),
            ),
        );
        assert!(write.ok, "{:?}", write.error);

        let read = dispatch(&state, request("file", "read", json!({ "path": path_str })));
        assert!(read.ok);
        let result = read.result.unwrap();
        assert_eq!(result["data"], "hello");
        assert_eq!(result["truncated"], false);

        let missing = dispatch(
            &state,
            request("file", "read", json!({ "path": path_str + ".nope" })),
        );
        assert!(!missing.ok);
        assert_eq!(missing.error.unwrap().code, "io_error");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn app_view_and_edit_queue_commands() {
        let state = ServerState::new();
        assert!(dispatch(&state, request("app", "view", json!({ "path": "/tmp/a" }))).ok);
        assert!(dispatch(&state, request("app", "edit", json!({ "path": "/tmp/b" }))).ok);
        let commands = state.take_commands();
        assert!(matches!(
            commands.as_slice(),
            [Command::View(a), Command::Edit(b)] if a == "/tmp/a" && b == "/tmp/b"
        ));
        let bad = dispatch(&state, request("app", "view", json!({})));
        assert!(!bad.ok);
        assert_eq!(bad.error.unwrap().code, "no_path");
    }

    #[test]
    fn rejects_an_oversized_request() {
        let mut input = vec![b'a'; MAX_LINE + 10];
        input.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(input));
        let mut buf = Vec::new();
        assert!(matches!(read_request(&mut reader, &mut buf), Ok(false)));
    }
}

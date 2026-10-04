//! `mtty-mtp` — the MTP control plane.
//!
//! A newline-delimited JSON server over a Unix socket (Windows named pipe is a
//! later transport). The envelope matches the existing MTP contract so
//! `mtty-cli` (formerly `miaotty-cli`), plugins and agent hooks keep working
//! unchanged.

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
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTO_VERSION: i64 = 1;
/// Largest file the control plane will read or write in one request.
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

pub const HOST_CAPS: &[&str] = &[
    "core.basic",
    "app.view.write",
    "pane.read",
    "file.read",
    "file.write",
    "agent.state.read",
    "agent.state.write",
    "agent.resume",
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

/// Default socket path (`$XDG_RUNTIME_DIR/mtty.sock` or `$TMPDIR/mtty.sock`),
/// shared with `mtty-cli`.
pub fn default_socket() -> PathBuf {
    runtime_dir().join("mtty.sock")
}

/// The pre-rename socket path (ADR 0032), still served through a link.
pub fn legacy_socket() -> PathBuf {
    runtime_dir().join("miaotty.sock")
}

/// The socket a client should use by default: the new path, or the legacy one
/// when only an older host is running.
pub fn client_socket() -> PathBuf {
    let socket = default_socket();
    let legacy = legacy_socket();
    if !socket.exists() && legacy.exists() {
        legacy
    } else {
        socket
    }
}

/// Point the legacy socket path at `socket`, so older clients reach this host.
/// Only a missing path or an existing symlink is replaced; a real socket left
/// by a running older host is never touched.
#[cfg(unix)]
pub fn link_legacy_socket(socket: &Path, legacy: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(legacy) {
        Ok(meta) if !meta.file_type().is_symlink() => return Ok(false),
        Ok(_) => std::fs::remove_file(legacy)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::os::unix::fs::symlink(socket, legacy)?;
    Ok(true)
}

/// An environment setting by its unprefixed name: `MTTY_<name>`, then the
/// pre-rename `MIAOTTY_<name>` (ADR 0032). Empty values count as unset.
pub fn env(name: &str) -> Option<String> {
    ["MTTY_", "MIAOTTY_"]
        .iter()
        .filter_map(|prefix| std::env::var(format!("{prefix}{name}")).ok())
        .find(|v| !v.is_empty())
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
    /// When set (from `MTTY_MTP_TOKEN`), every request must carry it.
    token: Option<String>,
    /// When set (from `MTTY_MTP_ALLOW`), only these capabilities are allowed.
    allow: Option<Vec<String>>,
    revision: Mutex<i64>,
    revision_cv: Condvar,
    seq: AtomicI64,
    states: Mutex<BTreeMap<String, Value>>,
    history: Mutex<BTreeMap<String, Vec<Value>>>,
    panes: Mutex<Vec<Value>>,
    writes: Mutex<Vec<(String, Vec<u8>)>>,
    commands: Mutex<Vec<Command>>,
    /// Called after work is queued so the host can wake its event loop.
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Event stream subscribers (`core.subscribe`). Dead channels are pruned.
    subscribers: Mutex<Vec<std::sync::mpsc::Sender<Value>>>,
    /// The host is in read-only mode: input to panes is refused.
    read_only: std::sync::atomic::AtomicBool,
    /// Every pane state change in order, so the host sees each transition
    /// even when several arrive between two of its loop iterations.
    transitions: Mutex<Vec<StateChange>>,
    /// Each pane's last finished command output (OSC 133), published by the host.
    outputs: Mutex<BTreeMap<String, Value>>,
}

/// One `agent.state.set` for a pane, as the host should observe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub pane: String,
    pub agent: String,
    pub state: String,
}

/// Unconsumed transitions kept at most (a host that stopped draining must
/// not grow memory without bound).
const MAX_TRANSITIONS: usize = 1024;

/// UI-side control actions queued by MTP methods.
#[derive(Debug, Clone)]
pub enum Command {
    Focus(String),
    Close(String),
    /// Open a file in the reader, optionally at a 1-based line.
    View {
        path: String,
        line: Option<usize>,
    },
    /// Open a file in the editor, optionally at a 1-based line/column.
    Edit {
        path: String,
        line: Option<usize>,
        column: Option<usize>,
    },
    /// Propose an edit to an open editor pane (ADR 0040, A1).
    Propose {
        pane: Option<String>,
        path: Option<String>,
        edits: Vec<ProposedEdit>,
        /// Replace the whole file with this instead of `edits`.
        text: Option<String>,
        label: Option<String>,
    },
    /// Resume a reported agent session in a new tab (ADR 0042, A4).
    ResumeAgent {
        agent: String,
        session: String,
        cwd: Option<String>,
    },
}

/// One replacement in `editor.propose`: characters `start..end` become `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl ServerState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Like [`ServerState::new`] but requires `token` on every request.
    pub fn with_token(token: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            token,
            ..Default::default()
        })
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Like [`ServerState::with_token`] but also applies a capability allowlist.
    pub fn with_config(token: Option<String>, allow: Option<Vec<String>>) -> Arc<Self> {
        Arc::new(Self {
            token,
            allow,
            ..Default::default()
        })
    }

    /// Mirror the host's read-only mode: while on, `pane.send` / `pane.run`
    /// fail with `read_only` instead of typing into a pane.
    pub fn set_read_only(&self, on: bool) {
        self.read_only.store(on, Ordering::SeqCst);
    }

    /// Parse a comma-separated `MTTY_MTP_ALLOW` value into a policy.
    /// Empty/blank input means "no policy" (allow everything).
    pub fn parse_allow(value: Option<String>) -> Option<Vec<String>> {
        let list: Vec<String> = value?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if list.is_empty() {
            None
        } else {
            Some(list)
        }
    }

    /// Whether a capability is permitted by the current policy. `core.basic`
    /// (ping/health) is always allowed so a client can discover the host.
    pub fn allows(&self, cap: &str) -> bool {
        if cap.is_empty() || cap == "core.basic" {
            return true;
        }
        match &self.allow {
            None => true,
            Some(list) => list.iter().any(|c| c == cap),
        }
    }

    /// The capabilities this host will actually accept.
    pub fn allowed_caps(&self) -> Vec<&'static str> {
        HOST_CAPS
            .iter()
            .copied()
            .filter(|c| self.allows(c))
            .collect()
    }

    fn bump(&self) -> i64 {
        let mut rev = self.revision.lock().unwrap();
        *rev += 1;
        self.revision_cv.notify_all();
        *rev
    }

    fn revision(&self) -> i64 {
        *self.revision.lock().unwrap()
    }

    /// Block until the revision is greater than `since`, or `timeout` elapses.
    /// Lets a client follow state changes without busy-polling.
    pub fn wait_for_revision(&self, since: i64, timeout: Duration) -> i64 {
        let rev = self.revision.lock().unwrap();
        if *rev > since {
            return *rev;
        }
        let (rev, _) = self.revision_cv.wait_timeout(rev, timeout).unwrap();
        *rev
    }

    /// Replace the advertised pane list (called by the app on tab changes).
    /// Publish a pane's last command output (`{text, exit, truncated}`).
    pub fn set_output(&self, pane: &str, output: Value) {
        self.outputs
            .lock()
            .unwrap()
            .insert(pane.to_string(), output);
    }

    pub fn set_panes(&self, panes: Vec<Value>) {
        *self.panes.lock().unwrap() = panes.clone();
        self.broadcast(json!({ "topic": "panes", "panes": panes }));
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

    /// Pane state changes since the last call, oldest first.
    pub fn take_transitions(&self) -> Vec<StateChange> {
        std::mem::take(&mut *self.transitions.lock().unwrap())
    }

    /// Agent state entry for a pane, if any.
    pub fn agent_for(&self, pane_id: &str) -> Option<Value> {
        let k = format!("pane:{pane_id}");
        self.states.lock().unwrap().get(&k).cloned()
    }

    /// Agent sessions that reported a session id, newest first (ADR 0042).
    pub fn agent_sessions(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .states
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| {
                e.get("session_id")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
            })
            .map(|(k, e)| {
                let mut e = e.clone();
                if let Value::Object(ref mut m) = e {
                    m.entry("pane").or_insert_with(|| json!(k));
                }
                e
            })
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.get("seq").and_then(Value::as_i64).unwrap_or(0)));
        out
    }

    /// The stored agent entry matching a pane id or a session id (ADR 0042).
    pub fn agent_session(&self, pane: Option<&str>, session: Option<&str>) -> Option<Value> {
        self.states.lock().unwrap().values().find_map(|e| {
            let pane_ok = pane.is_some_and(|p| e.get("pane_id").and_then(Value::as_str) == Some(p));
            let sess_ok =
                session.is_some_and(|s| e.get("session_id").and_then(Value::as_str) == Some(s));
            (pane_ok || sess_ok).then(|| e.clone())
        })
    }

    /// Start an event stream. The returned receiver yields one JSON object per
    /// state change; dropping it unsubscribes.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Value> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Send `event` to every subscriber, dropping those that went away.
    fn broadcast(&self, event: Value) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Register a callback the host uses to wake its event loop; it is invoked
    /// whenever the control plane queues work (`pane.send`, `pane.focus`, …).
    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        *self.waker.lock().unwrap() = Some(waker);
    }

    /// Wake the host, if it registered a waker.
    fn wake(&self) {
        let waker = self.waker.lock().unwrap().clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    fn queue_write(&self, pane_id: String, data: Vec<u8>) {
        self.writes.lock().unwrap().push((pane_id, data));
        self.wake();
    }

    /// Take queued writes for the UI to feed into the panes.
    pub fn take_writes(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut *self.writes.lock().unwrap())
    }

    fn queue_command(&self, command: Command) {
        self.commands.lock().unwrap().push(command);
        self.wake();
    }

    /// Take queued focus/close commands for the UI to apply.
    pub fn take_commands(&self) -> Vec<Command> {
        std::mem::take(&mut *self.commands.lock().unwrap())
    }
}

/// Standard base64 (RFC 4648) with padding.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Decode standard base64; `None` on invalid input.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    // A base64 length of 1 (mod 4) cannot be produced by an encoder.
    if cleaned.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in cleaned {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
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

/// The capability a method requires ("" when the method is unknown).
fn method_cap(ns: &str, method: &str) -> &'static str {
    match (ns, method) {
        ("core", "ping" | "health" | "wait" | "subscribe") => "core.basic",
        // A pane's output may hold secrets: reading it is its own capability.
        ("pane", "output") => "pane.read",
        ("pane", _) | ("app", _) | ("editor", _) => "app.view.write",
        ("file", "read") => "file.read",
        ("file", "write") => "file.write",
        ("agent", "state.list") => "agent.state.read",
        ("agent", "state.set") => "agent.state.write",
        ("agent", "sessions") => "agent.state.read",
        ("agent", "resume") => "agent.resume",
        ("history", "list") => "history.read",
        ("history", "add") => "history.write",
        _ => "",
    }
}

fn dispatch(state: &ServerState, req: Request) -> Response {
    let id = req.id;
    let rev = state.revision();
    let params = req.params.clone().unwrap_or(Value::Null);
    if let Some(expected) = state.token() {
        if params.get("token").and_then(Value::as_str) != Some(expected) {
            return Response::err(id, rev, "unauthorized", "missing or invalid token");
        }
    }
    // Any request may be a wake-up (a forwarded launch pings `core.health`
    // after dropping its inbox file), so repaint once per request.
    state.wake();
    let cap = method_cap(&req.ns, &req.method);
    if !state.allows(cap) {
        return Response::err(
            id,
            rev,
            "forbidden",
            format!("capability not allowed: {cap}"),
        );
    }
    match (req.ns.as_str(), req.method.as_str()) {
        ("core", "ping") => Response::ok(
            id,
            rev,
            json!({
                "proto": PROTO_VERSION,
                "app_version": concat!("mtty/", env!("CARGO_PKG_VERSION")),
                "pid": std::process::id(),
                "caps": HOST_CAPS,
                "allowed": state.allowed_caps(),
            }),
        ),
        ("core", "health") => Response::ok(id, rev, json!({ "ok": true, "revision": rev })),
        ("core", "subscribe") => {
            // The connection is upgraded to an event stream by `handle`.
            Response::ok(id, rev, json!({ "ok": true, "subscribed": true }))
        }
        ("core", "wait") => {
            // Long-poll: return as soon as the revision moves past `since`.
            let since = params.get("since").and_then(Value::as_i64).unwrap_or(0);
            let timeout_ms = params
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(25_000)
                .min(120_000);
            let now = state.wait_for_revision(since, Duration::from_millis(timeout_ms));
            Response::ok(id, now, json!({ "revision": now, "changed": now > since }))
        }
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
            if state.read_only.load(Ordering::SeqCst) {
                return Response::err(id, rev, "read_only", "the terminal is in read-only mode");
            }
            let bytes = if req.method == "run" {
                format!("{data}\r").into_bytes()
            } else {
                data.into_bytes()
            };
            state.queue_write(pane, bytes);
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("pane", "output") => {
            let pane = str_field(&params, "pane_id").unwrap_or_default();
            match state.outputs.lock().unwrap().get(&pane) {
                Some(output) => Response::ok(id, rev, output.clone()),
                None => Response::err(
                    id,
                    rev,
                    "no_output",
                    "no finished command in this pane (needs the shell integration)",
                ),
            }
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
            let line = params
                .get("line")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .map(|n| n as usize);
            let column = params
                .get("column")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let command = if req.method == "view" {
                Command::View { path, line }
            } else {
                Command::Edit { path, line, column }
            };
            state.queue_command(command);
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("editor", "propose") => {
            let pane = str_field(&params, "pane_id");
            let path = str_field(&params, "path");
            if pane.is_none() && path.is_none() {
                return Response::err(id, rev, "no_pane", "pane_id or path is required");
            }
            let label = str_field(&params, "label");
            let text = str_field(&params, "text");
            let edits: Vec<ProposedEdit> = params
                .get("edits")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            let start = e.get("start").and_then(Value::as_u64)? as usize;
                            let end = e
                                .get("end")
                                .and_then(Value::as_u64)
                                .map(|n| n as usize)
                                .unwrap_or(start);
                            let text = e
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            Some(ProposedEdit { start, end, text })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if edits.is_empty() && text.is_none() {
                return Response::err(id, rev, "no_edits", "edits or text is required");
            }
            state.queue_command(Command::Propose {
                pane,
                path,
                edits,
                text,
                label,
            });
            Response::ok(id, rev, json!({ "ok": true }))
        }
        ("file", "read") => {
            let path = str_field(&params, "path").unwrap_or_default();
            if path.is_empty() {
                return Response::err(id, rev, "no_path", "path is required");
            }
            let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let length = params
                .get("length")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(MAX_FILE_BYTES)
                .min(MAX_FILE_BYTES);
            let base64 = str_field(&params, "encoding").as_deref() == Some("base64");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let total = bytes.len();
                    let end = offset.saturating_add(length).min(total);
                    let slice = if offset <= total {
                        &bytes[offset..end]
                    } else {
                        &[][..]
                    };
                    let mut result = json!({
                        "bytes": total,
                        "offset": offset,
                        "returned": slice.len(),
                        "eof": end >= total,
                        "truncated": end < total,
                    });
                    result["data"] = if base64 {
                        json!(base64_encode(slice))
                    } else {
                        json!(String::from_utf8_lossy(slice))
                    };
                    result["encoding"] = json!(if base64 { "base64" } else { "utf8" });
                    Response::ok(id, rev, result)
                }
                Err(e) => Response::err(id, rev, "io_error", e.to_string()),
            }
        }
        ("file", "write") => {
            let path = str_field(&params, "path").unwrap_or_default();
            if path.is_empty() {
                return Response::err(id, rev, "no_path", "path is required");
            }
            let bytes = match (str_field(&params, "data_b64"), str_field(&params, "data")) {
                (Some(b64), _) => match base64_decode(&b64) {
                    Some(bytes) => bytes,
                    None => return Response::err(id, rev, "bad_encoding", "invalid base64"),
                },
                (None, Some(text)) => text.into_bytes(),
                (None, None) => Vec::new(),
            };
            match std::fs::write(&path, &bytes) {
                Ok(()) => Response::ok(id, rev, json!({ "ok": true, "bytes": bytes.len() })),
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
                // When this pane last reported, for the Resume picker (ADR 0042).
                m.insert("ts".into(), json!(now_ms()));
            }
            let revision = state.bump();
            state.states.lock().unwrap().insert(k, entry.clone());
            if let (Some(pane), Some(new_state)) =
                (pane.clone(), params.get("state").and_then(Value::as_str))
            {
                let mut log = state.transitions.lock().unwrap();
                if log.len() >= MAX_TRANSITIONS {
                    log.remove(0);
                }
                log.push(StateChange {
                    pane,
                    agent: agent.clone(),
                    state: new_state.to_string(),
                });
            }
            state.broadcast(json!({
                "topic": "agent.state",
                "pane": pane,
                "agent": agent,
                "state": params.get("state").cloned().unwrap_or(Value::Null),
                "revision": revision,
            }));
            Response::ok(id, revision, json!({ "revision": revision }))
        }
        ("agent", "state.list") => {
            let states: Vec<Value> = state.states.lock().unwrap().values().cloned().collect();
            Response::ok(id, rev, json!({ "revision": rev, "states": states }))
        }
        ("agent", "sessions") => Response::ok(
            id,
            rev,
            json!({ "revision": rev, "sessions": state.agent_sessions() }),
        ),
        ("agent", "resume") => {
            if state.read_only.load(Ordering::SeqCst) {
                return Response::err(id, rev, "read_only", "the terminal is in read-only mode");
            }
            let pane = str_field(&params, "pane_id");
            let session = str_field(&params, "session_id");
            let Some(entry) = state.agent_session(pane.as_deref(), session.as_deref()) else {
                return Response::err(id, rev, "no_session", "no matching agent session");
            };
            let agent = entry
                .get("agent")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let session_id = entry
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let cwd = entry.get("cwd").and_then(Value::as_str).map(str::to_string);
            if agent.is_empty() || session_id.is_empty() {
                return Response::err(
                    id,
                    rev,
                    "no_session",
                    "the agent did not report a session id",
                );
            }
            state.queue_command(Command::ResumeAgent {
                agent,
                session: session_id,
                cwd,
            });
            Response::ok(id, state.bump(), json!({ "ok": true }))
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
            state.broadcast(json!({
                "topic": "history",
                "pane": pane,
                "command": params.get("command").cloned().unwrap_or(Value::Null),
                "revision": revision,
            }));
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

fn handle<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    state: &ServerState,
    pump: Option<Box<dyn Write + Send>>,
) {
    let mut pump = pump;
    let mut buf = Vec::new();
    while let Ok(true) = read_request(reader, &mut buf) {
        let response = match std::str::from_utf8(&buf) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Request>(line) {
                    Ok(req) => {
                        // `core.subscribe` upgrades this connection to a stream.
                        let is_subscribe = req.ns == "core" && req.method == "subscribe";
                        let topics = is_subscribe
                            .then(|| {
                                req.params
                                    .as_ref()
                                    .and_then(|p| p.get("topics").cloned())
                                    .map(|t| topic_list(&t))
                            })
                            .flatten();
                        let response = dispatch(state, req);
                        if let (true, true, Some(w)) = (is_subscribe, response.ok, pump.take()) {
                            spawn_event_pump(w, state.subscribe(), topics);
                        }
                        response
                    }
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
        if !write_response(writer, &response) {
            break;
        }
    }
}

/// A minimal MTP client (cross-platform via `interprocess`).
pub mod client {
    use super::{ErrorInfo, PROTO_VERSION};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::Path;

    #[cfg(unix)]
    use interprocess::local_socket::GenericFilePath;
    #[cfg(windows)]
    use interprocess::local_socket::GenericNamespaced;
    use interprocess::local_socket::{prelude::*, ConnectOptions};
    use interprocess::TryClone;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Deserialize)]
    struct Reply {
        ok: bool,
        result: Option<Value>,
        error: Option<ErrorInfo>,
    }

    pub struct Client {
        reader: BufReader<Box<dyn Read + Send>>,
        writer: Box<dyn Write + Send>,
        next_id: i64,
    }

    /// Connect to the host at `path` (Unix socket path; ignored on Windows,
    /// which uses the fixed `mtty` named pipe, falling back to the pre-rename
    /// `miaotty` pipe of an older host).
    pub fn connect(path: &Path) -> std::io::Result<Client> {
        #[cfg(unix)]
        let name = path.to_fs_name::<GenericFilePath>()?;
        #[cfg(windows)]
        let stream = {
            let _ = path;
            let name = "mtty".to_ns_name::<GenericNamespaced>()?;
            match ConnectOptions::new().name(name).connect_sync() {
                Ok(stream) => stream,
                Err(_) => {
                    let legacy = "miaotty".to_ns_name::<GenericNamespaced>()?;
                    ConnectOptions::new().name(legacy).connect_sync()?
                }
            }
        };
        #[cfg(unix)]
        let stream = ConnectOptions::new().name(name).connect_sync()?;
        let writer = stream.try_clone()?;
        Ok(Client {
            reader: BufReader::new(Box::new(stream)),
            writer: Box::new(writer),
            next_id: 1,
        })
    }

    /// Connect over TCP (`host:port`), for a host serving `remote-listen`.
    pub fn connect_tcp(addr: &str) -> std::io::Result<Client> {
        let stream = std::net::TcpStream::connect(addr)?;
        let writer = stream.try_clone()?;
        Ok(Client {
            reader: BufReader::new(Box::new(stream)),
            writer: Box::new(writer),
            next_id: 1,
        })
    }

    /// Connect to `tcp://host:port` or a local socket path.
    pub fn connect_any(addr: &str) -> std::io::Result<Client> {
        match addr.strip_prefix("tcp://") {
            Some(rest) => connect_tcp(rest),
            None => connect(Path::new(addr)),
        }
    }

    impl Client {
        /// Send a request and return the result value (or an error).
        /// Subscribe to the host's event stream (`topics` empty = all).
        /// Subsequent events arrive via [`Client::next_event`].
        pub fn subscribe(&mut self, topics: &[&str]) -> std::io::Result<()> {
            self.call("core", "subscribe", json!({ "topics": topics }))
                .map(|_| ())
        }

        /// Block until the next event arrives; `None` when the host closed the
        /// stream.
        pub fn next_event(&mut self) -> std::io::Result<Option<Value>> {
            let mut buf = String::new();
            loop {
                buf.clear();
                if self.reader.read_line(&mut buf)? == 0 {
                    return Ok(None);
                }
                let line = buf.trim();
                if line.is_empty() {
                    continue;
                }
                let v: Value =
                    serde_json::from_str(line).map_err(|e| std::io::Error::other(e.to_string()))?;
                match v.get("kind").and_then(Value::as_str) {
                    Some("event") => return Ok(v.get("event").cloned()),
                    // A late reply to a previous call: ignore.
                    _ => continue,
                }
            }
        }

        pub fn call(&mut self, ns: &str, method: &str, params: Value) -> std::io::Result<Value> {
            // A host started with MTTY_MTP_TOKEN requires it on every request.
            let params = match crate::env("MTP_TOKEN") {
                Some(token) => match params {
                    Value::Object(mut map) => {
                        map.insert("token".to_string(), Value::String(token));
                        Value::Object(map)
                    }
                    other => other,
                },
                _ => params,
            };
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

/// Normalise a `topics` parameter (array or comma-separated string).
fn topic_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Forward subscription events to a socket as `kind: "event"` lines.
fn spawn_event_pump(
    mut writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Value>,
    topics: Option<Vec<String>>,
) {
    thread::spawn(move || {
        for event in rx {
            if let Some(only) = &topics {
                if !only.is_empty() {
                    let topic = event.get("topic").and_then(Value::as_str).unwrap_or("");
                    if !only.iter().any(|t| t == topic) {
                        continue;
                    }
                }
            }
            let line = json!({ "v": PROTO_VERSION, "kind": "event", "event": event });
            let ok = writer
                .write_all(line.to_string().as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .and_then(|_| writer.flush())
                .is_ok();
            if !ok {
                break;
            }
        }
    });
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
    let name = {
        let _ = path;
        "mtty".to_ns_name::<GenericNamespaced>()?
    };

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
            thread::spawn(move || {
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                let pump = stream
                    .try_clone()
                    .ok()
                    .map(|s| Box::new(s) as Box<dyn Write + Send>);
                let mut writer = stream;
                handle(&mut reader, &mut writer, &state, pump);
            });
        }
    });
    Ok(())
}

/// Serve the control plane over TCP (for remote access). Requires a token: the
/// control plane runs commands in the user's shell, so it must be authenticated.
pub fn serve_tcp(addr: &str, state: Arc<ServerState>) -> std::io::Result<()> {
    use std::net::TcpListener;
    if state.token().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "remote access requires MTTY_MTP_TOKEN",
        ));
    }
    let listener = TcpListener::bind(addr)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || {
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                let pump = stream
                    .try_clone()
                    .ok()
                    .map(|s| Box::new(s) as Box<dyn Write + Send>);
                let mut writer = stream;
                handle(&mut reader, &mut writer, &state, pump);
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn base64_round_trips() {
        for case in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foobar",
            &[0u8, 255, 1, 2, 3][..],
        ] {
            let encoded = base64_encode(case);
            assert_eq!(base64_decode(&encoded).unwrap(), case, "{encoded}");
        }
        assert!(base64_decode("A").is_none());
        assert!(base64_decode("!!!!").is_none());
    }

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
        let path = std::env::temp_dir().join(format!("mtty-mtp-file-{}", std::process::id()));
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
    fn quick_state_changes_are_all_observed_in_order() {
        let state = ServerState::new();
        for s in ["processing", "idle", "processing", "idle"] {
            let r = dispatch(
                &state,
                request(
                    "agent",
                    "state.set",
                    json!({ "pane_id": "p1", "agent": "codex", "state": s }),
                ),
            );
            assert!(r.ok);
        }
        let seen: Vec<String> = state
            .take_transitions()
            .into_iter()
            .map(|c| c.state)
            .collect();
        assert_eq!(seen, ["processing", "idle", "processing", "idle"]);
        assert!(
            state.take_transitions().is_empty(),
            "each change is taken once"
        );
    }

    #[test]
    fn read_only_mode_refuses_pane_input() {
        let state = ServerState::new();
        let send = || {
            dispatch(
                &state,
                request(
                    "pane",
                    "run",
                    json!({ "pane_id": "p1", "data": "rm -rf x" }),
                ),
            )
        };
        state.set_read_only(true);
        let refused = send();
        assert!(!refused.ok);
        assert_eq!(refused.error.unwrap().code, "read_only");
        assert!(state.take_writes().is_empty());
        state.set_read_only(false);
        assert!(send().ok);
        assert_eq!(state.take_writes().len(), 1);
    }

    #[test]
    fn token_is_required_when_configured() {
        let state = ServerState::with_token(Some("s3cret".to_string()));
        let denied = dispatch(&state, request("core", "ping", json!({})));
        assert!(!denied.ok);
        assert_eq!(denied.error.unwrap().code, "unauthorized");
        let allowed = dispatch(
            &state,
            request("core", "ping", json!({ "token": "s3cret" })),
        );
        assert!(allowed.ok, "{:?}", allowed.error);
        // No token configured: nothing to check.
        assert!(dispatch(&ServerState::new(), request("core", "ping", json!({}))).ok);
    }

    #[test]
    fn capability_policy_gates_methods() {
        let state = ServerState::with_config(
            None,
            ServerState::parse_allow(Some("core.basic, file.read".into())),
        );
        // ping is always allowed, and reports the effective caps.
        let ping = dispatch(&state, request("core", "ping", json!({})));
        assert!(ping.ok);
        assert_eq!(
            ping.result.unwrap()["allowed"],
            json!(["core.basic", "file.read"])
        );
        // file.write is denied.
        let denied = dispatch(
            &state,
            request("file", "write", json!({ "path": "/tmp/x", "data": "hi" })),
        );
        assert!(!denied.ok);
        assert_eq!(denied.error.unwrap().code, "forbidden");
        // core.basic stays allowed even under a tight policy.
        let locked =
            ServerState::with_config(None, ServerState::parse_allow(Some("core.basic".into())));
        assert!(dispatch(&locked, request("core", "health", json!({}))).ok);
        assert!(!dispatch(&locked, request("pane", "list", json!({}))).ok);
        // No policy: everything is allowed (backwards compatible). Use the OS
        // temp dir so this works on Windows too (there is no `/tmp`).
        let open = ServerState::new();
        let tmp = std::env::temp_dir().join(format!("mtty-mtp-allow-{}", std::process::id()));
        let tmp = tmp.to_string_lossy().to_string();
        assert!(
            dispatch(
                &open,
                request("file", "write", json!({ "path": tmp, "data": "z" }))
            )
            .ok
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn legacy_socket_link_never_replaces_a_real_socket() {
        let dir = std::env::temp_dir().join(format!("mtty-mtp-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("mtty.sock");
        let legacy = dir.join("miaotty.sock");
        assert!(link_legacy_socket(&socket, &legacy).unwrap());
        assert_eq!(std::fs::read_link(&legacy).unwrap(), socket);
        // A stale link from an earlier run is refreshed.
        assert!(link_legacy_socket(&dir.join("other.sock"), &legacy).unwrap());
        assert_eq!(std::fs::read_link(&legacy).unwrap(), dir.join("other.sock"));
        // A real file (an older host's socket) is left alone.
        std::fs::remove_file(&legacy).unwrap();
        std::fs::write(&legacy, "").unwrap();
        assert!(!link_legacy_socket(&socket, &legacy).unwrap());
        assert!(!std::fs::symlink_metadata(&legacy)
            .unwrap()
            .file_type()
            .is_symlink());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_allow_blank_is_none() {
        assert_eq!(ServerState::parse_allow(None), None);
        assert_eq!(ServerState::parse_allow(Some("  , ,".into())), None);
        assert_eq!(
            ServerState::parse_allow(Some("a, b".into())),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn file_read_supports_offsets_and_base64() {
        let state = ServerState::new();
        let path = std::env::temp_dir().join(format!("mtty-mtp-bin-{}", std::process::id()));
        let path_str = path.to_string_lossy().to_string();
        // 0x00..0x03 via base64 ("AAECAw==")
        assert!(
            dispatch(
                &state,
                request(
                    "file",
                    "write",
                    json!({ "path": path_str, "data_b64": "AAECAw==" })
                )
            )
            .ok
        );
        let all = dispatch(
            &state,
            request(
                "file",
                "read",
                json!({ "path": path_str, "encoding": "base64" }),
            ),
        );
        assert!(all.ok);
        let result = all.result.unwrap();
        assert_eq!(result["data"], "AAECAw==");
        assert_eq!(result["encoding"], "base64");
        assert_eq!(result["bytes"], 4);
        assert_eq!(result["eof"], true);

        let part = dispatch(
            &state,
            request(
                "file",
                "read",
                json!({ "path": path_str, "offset": 1, "length": 2, "encoding": "base64" }),
            ),
        );
        let result = part.result.unwrap();
        assert_eq!(result["data"], "AQI=");
        assert_eq!(result["returned"], 2);
        assert_eq!(result["eof"], false);
        assert_eq!(result["truncated"], true);

        let bad = dispatch(
            &state,
            request(
                "file",
                "write",
                json!({ "path": path_str, "data_b64": "!!!not base64" }),
            ),
        );
        assert_eq!(bad.error.unwrap().code, "bad_encoding");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn app_view_and_edit_queue_commands() {
        let state = ServerState::new();
        assert!(dispatch(&state, request("app", "view", json!({ "path": "/tmp/a" }))).ok);
        assert!(
            dispatch(
                &state,
                request(
                    "app",
                    "edit",
                    json!({ "path": "/tmp/b", "line": 3, "column": 5 })
                ),
            )
            .ok
        );
        let commands = state.take_commands();
        assert!(matches!(
            commands.as_slice(),
            [
                Command::View { path: a, line: None },
                Command::Edit {
                    path: b,
                    line: Some(3),
                    column: Some(5)
                }
            ] if a == "/tmp/a" && b == "/tmp/b"
        ));
        let bad = dispatch(&state, request("app", "view", json!({})));
        assert!(!bad.ok);
        assert_eq!(bad.error.unwrap().code, "no_path");
    }

    #[test]
    fn editor_propose_queues_edits() {
        let state = ServerState::new();
        assert!(
            dispatch(
                &state,
                request(
                    "editor",
                    "propose",
                    json!({
                        "path": "/tmp/a",
                        "label": "agent",
                        "edits": [{ "start": 4, "end": 7, "text": "TWO" }],
                    }),
                ),
            )
            .ok
        );
        let commands = state.take_commands();
        assert!(matches!(
            commands.as_slice(),
            [Command::Propose { edits, label: Some(l), .. }]
                if edits.len() == 1
                    && edits[0].start == 4
                    && edits[0].end == 7
                    && edits[0].text == "TWO"
                    && l == "agent"
        ));
        let bad = dispatch(
            &state,
            request("editor", "propose", json!({ "path": "/tmp/a" })),
        );
        assert!(!bad.ok);
        assert_eq!(bad.error.unwrap().code, "no_edits");
    }

    #[test]
    fn tcp_requires_a_token() {
        assert!(serve_tcp("127.0.0.1:0", ServerState::new()).is_err());
        let st = ServerState::with_token(Some("t".into()));
        assert!(serve_tcp("127.0.0.1:0", st).is_ok());
    }

    #[test]
    fn rejects_an_oversized_request() {
        let mut input = vec![b'a'; MAX_LINE + 10];
        input.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(input));
        let mut buf = Vec::new();
        assert!(matches!(read_request(&mut reader, &mut buf), Ok(false)));
    }

    #[test]
    fn subscribe_receives_state_events() {
        let st = ServerState::new();
        let rx = st.subscribe();
        dispatch(
            &st,
            request(
                "agent",
                "state.set",
                json!({ "pane_id": "pane0", "agent": "claude", "state": "processing" }),
            ),
        );
        let event = rx.recv_timeout(Duration::from_secs(2)).expect("event");
        assert_eq!(event["topic"], json!("agent.state"));
        assert_eq!(event["pane"], json!("pane0"));
        assert_eq!(event["agent"], json!("claude"));
        assert_eq!(event["state"], json!("processing"));
        assert!(event["revision"].as_i64().unwrap() >= 1);
    }

    #[test]
    fn sessions_list_newest_first_and_resume_queues_a_command() {
        let st = ServerState::new();
        for (pane, agent, session) in [("pane0", "claude", "s-1"), ("pane1", "codex", "s-2")] {
            dispatch(
                &st,
                request(
                    "agent",
                    "state.set",
                    json!({ "pane_id": pane, "agent": agent, "state": "idle",
                            "session_id": session, "cwd": "/tmp" }),
                ),
            );
        }
        // A state without a session id is not offered for resume.
        dispatch(
            &st,
            request(
                "agent",
                "state.set",
                json!({ "pane_id": "pane2", "agent": "miao", "state": "idle" }),
            ),
        );
        let listed = dispatch(&st, request("agent", "sessions", json!({})));
        let sessions = listed.result.expect("sessions result");
        let sessions = sessions["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        // Newest first: pane1 was set last.
        assert_eq!(sessions[0]["session_id"], json!("s-2"));

        // Resume by session id queues a ResumeAgent command for the host.
        let ok = dispatch(
            &st,
            request("agent", "resume", json!({ "session_id": "s-1" })),
        );
        assert!(ok.ok, "{:?}", ok.error);
        let commands = st.take_commands();
        assert!(matches!(
            commands.as_slice(),
            [Command::ResumeAgent { agent, session, cwd }]
                if agent == "claude" && session == "s-1" && cwd.as_deref() == Some("/tmp")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn event_stream_over_the_socket() {
        let path = std::env::temp_dir().join(format!("miao-mtp-test-{}.sock", std::process::id()));
        let st = ServerState::new();
        serve(&path, st.clone()).expect("serve");
        // The listener thread may need a moment to come up.
        let mut client = loop {
            match client::connect(&path) {
                Ok(c) => break c,
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        };
        client.subscribe(&["agent.state"]).expect("subscribe");
        dispatch(
            &st,
            request(
                "agent",
                "state.set",
                json!({ "pane_id": "pane1", "agent": "codex", "state": "idle" }),
            ),
        );
        let event = client
            .next_event()
            .expect("stream")
            .expect("an event arrived");
        assert_eq!(event["topic"], json!("agent.state"));
        assert_eq!(event["pane"], json!("pane1"));
        // Topic filtering: a panes event is not delivered to this subscriber.
        st.set_panes(vec![json!({ "id": "pane1" })]);
        dispatch(
            &st,
            request(
                "agent",
                "state.set",
                json!({ "pane_id": "pane2", "agent": "claude", "state": "error" }),
            ),
        );
        let again = client.next_event().unwrap().unwrap();
        assert_eq!(again["pane"], json!("pane2"), "filtered stream");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn wait_returns_immediately_when_already_changed() {
        let st = ServerState::new();
        st.bump();
        let t0 = std::time::Instant::now();
        assert_eq!(st.wait_for_revision(0, Duration::from_secs(5)), 1);
        assert!(t0.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn wait_wakes_on_a_later_change() {
        let st = ServerState::new();
        let bumping = st.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            bumping.bump();
        });
        let t0 = std::time::Instant::now();
        let rev = st.wait_for_revision(0, Duration::from_secs(5));
        assert_eq!(rev, 1);
        assert!(t0.elapsed() < Duration::from_secs(2), "woke promptly");
        handle.join().unwrap();
    }

    #[test]
    fn wait_times_out_without_changes() {
        let st = ServerState::new();
        let t0 = std::time::Instant::now();
        assert_eq!(st.wait_for_revision(0, Duration::from_millis(120)), 0);
        assert!(t0.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn wait_is_dispatched() {
        let st = ServerState::new();
        let r = dispatch(
            &st,
            request("core", "wait", json!({ "since": 0, "timeout_ms": 10 })),
        );
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["changed"], json!(false));
    }
}

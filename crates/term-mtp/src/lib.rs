//! `miao-term-mtp` — the MTP control plane.
//!
//! A newline-delimited JSON server over a Unix socket (Windows named pipe is a
//! later transport). The envelope matches the existing `miaotty` MTP contract so
//! `miaotty-cli`, plugins and agent hooks keep working unchanged.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTO_VERSION: i64 = 1;
pub const HOST_CAPS: &[&str] = &[
    "core.basic",
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

#[derive(Debug, Serialize)]
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

/// Default socket path (`$TMPDIR/miaotty.sock`), shared with `miaotty-cli`.
pub fn default_socket() -> PathBuf {
    std::env::temp_dir().join("miaotty.sock")
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
        self.history.lock().unwrap().get(&k).cloned().unwrap_or_default()
    }

    /// Agent state entry for a pane, if any.
    pub fn agent_for(&self, pane_id: &str) -> Option<Value> {
        let k = format!("pane:{pane_id}");
        self.states.lock().unwrap().get(&k).cloned()
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
            let limit = params.get("limit").and_then(Value::as_u64).map(|n| n as usize);
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

fn handle(stream: std::os::unix::net::UnixStream, state: Arc<ServerState>) {
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    let mut writer = stream;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatch(&state, req),
            Err(e) => Response::err(0, state.revision(), "bad_request", e.to_string()),
        };
        if let Ok(mut encoded) = serde_json::to_string(&response) {
            encoded.push('\n');
            if writer.write_all(encoded.as_bytes()).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    }
}

/// Bind `path` and serve connections on a background thread.
pub fn serve(path: &Path, state: Arc<ServerState>) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || handle(stream, state));
        }
    });
    Ok(())
}

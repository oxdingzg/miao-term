//! Language servers for mtty's editor pane (ADR 0034, phase E5).
//!
//! [`Lsp`] keeps one server per (language group, workspace root), opens and
//! updates the documents the editor panes show, and turns answers into
//! [`Event`]s for the host. It is free of UI code: positions go in as
//! chars of a rope and come back in the protocol's terms, with the
//! encoding needed to read them ([`position::pos_to_char`]).

pub mod client;
pub mod env;
pub mod position;
pub mod protocol;
pub mod rpc;
pub mod servers;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use miao_term_editor::Rope;
use serde_json::json;

pub use client::{Capabilities, Client, Incoming, Waker};
pub use position::{char_to_pos, pos_to_char, Encoding, LspRange, Pos};
pub use protocol::{CompletionItem, Diagnostic};

/// Files larger than this get no server: each change sends the whole text.
pub const MAX_DOCUMENT_BYTES: usize = 2 << 20;

/// `config.toml`'s `[lsp]`: on unless `enabled = false`; per server key a
/// command and root markers in place of the defaults, or `enabled = false`.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub disabled: bool,
    pub servers: HashMap<String, ServerSettings>,
}

#[derive(Clone, Debug, Default)]
pub struct ServerSettings {
    pub command: Option<Vec<String>>,
    pub root_markers: Option<Vec<String>>,
    pub disabled: bool,
}

/// What the host hears back.
#[derive(Debug)]
pub enum Event {
    /// New diagnostics for a file (read them with [`Lsp::diagnostics`]).
    Diagnostics(PathBuf),
    Hover {
        path: PathBuf,
        at: usize,
        markdown: String,
    },
    Completion {
        path: PathBuf,
        /// The caret the request was made at.
        at: usize,
        ticket: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
        encoding: Encoding,
    },
    Definition {
        targets: Vec<(PathBuf, LspRange)>,
        encoding: Encoding,
    },
    /// A server could not start or stopped; said once per server.
    Failed {
        server: String,
        message: String,
        /// The user configured this server (rather than a default that is
        /// simply not installed).
        configured: bool,
    },
}

struct Running {
    key: String,
    root: PathBuf,
    client: Client,
    failed: bool,
}

struct OpenDoc {
    server: usize,
    version: i32,
    revision: u64,
}

enum Pending {
    Hover {
        path: PathBuf,
        at: usize,
    },
    Completion {
        path: PathBuf,
        at: usize,
        ticket: u64,
    },
    Definition,
}

pub struct Lsp {
    settings: Settings,
    servers: Vec<Running>,
    docs: HashMap<PathBuf, OpenDoc>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    pending: HashMap<(usize, i64), Pending>,
    /// Files whose language has no server here (not installed, disabled).
    skipped: HashSet<PathBuf>,
    waker: Waker,
    next_ticket: u64,
}

impl Lsp {
    pub fn new(settings: Settings, waker: Waker) -> Self {
        Lsp {
            settings,
            servers: Vec::new(),
            docs: HashMap::new(),
            diagnostics: HashMap::new(),
            pending: HashMap::new(),
            skipped: HashSet::new(),
            waker,
            next_ticket: 0,
        }
    }

    /// Open `path` with its server, or send its new text when `revision`
    /// changed. `language` is the editor's name for it ("Rust"…). Cheap
    /// when nothing changed; call for every open editor each frame.
    pub fn sync(&mut self, path: &Path, language: Option<&str>, rope: &Rope, revision: u64) {
        if self.settings.disabled || self.skipped.contains(path) {
            return;
        }
        if let Some(doc) = self.docs.get_mut(path) {
            if doc.revision == revision {
                return;
            }
            doc.revision = revision;
            doc.version += 1;
            let version = doc.version;
            let running = &mut self.servers[doc.server];
            if running.failed {
                return;
            }
            running.client.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": position::path_to_uri(path), "version": version },
                    "contentChanges": [{ "text": rope.to_string() }],
                }),
            );
            return;
        }
        let Some((key, language_id)) = language.and_then(|l| servers::language(l, path)) else {
            self.skipped.insert(path.to_path_buf());
            return;
        };
        let custom = self.settings.servers.get(key).cloned().unwrap_or_default();
        if custom.disabled || rope.len_bytes() > MAX_DOCUMENT_BYTES {
            self.skipped.insert(path.to_path_buf());
            return;
        }
        let Some(spec) = servers::server(key) else {
            return;
        };
        let markers = custom
            .root_markers
            .clone()
            .unwrap_or_else(|| spec.root_markers.iter().map(|m| m.to_string()).collect());
        let root = servers::find_root(path, &markers);
        let index = match self
            .servers
            .iter()
            .position(|s| s.key == key && s.root == root)
        {
            Some(i) => i,
            None => {
                let command = custom
                    .command
                    .clone()
                    .unwrap_or_else(|| spec.command.iter().map(|c| c.to_string()).collect());
                self.servers.push(Running {
                    key: key.to_string(),
                    root: root.clone(),
                    client: Client::start(command, &root, self.waker.clone()),
                    failed: false,
                });
                self.servers.len() - 1
            }
        };
        if self.servers[index].failed {
            self.skipped.insert(path.to_path_buf());
            return;
        }
        self.servers[index].client.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": position::path_to_uri(path),
                    "languageId": language_id,
                    "version": 1,
                    "text": rope.to_string(),
                }
            }),
        );
        self.docs.insert(
            path.to_path_buf(),
            OpenDoc {
                server: index,
                version: 1,
                revision,
            },
        );
    }

    /// Close the documents not in `open` (their panes went away).
    pub fn retain(&mut self, open: &HashSet<PathBuf>) {
        let gone: Vec<PathBuf> = self
            .docs
            .keys()
            .filter(|p| !open.contains(*p))
            .cloned()
            .collect();
        for path in gone {
            if let Some(doc) = self.docs.remove(&path) {
                let running = &mut self.servers[doc.server];
                if !running.failed {
                    running.client.notify(
                        "textDocument/didClose",
                        json!({ "textDocument": { "uri": position::path_to_uri(&path) } }),
                    );
                }
            }
            self.diagnostics.remove(&path);
        }
        self.skipped.retain(|p| open.contains(p));
    }

    /// The file was written: some servers (rust-analyzer's `cargo check`)
    /// report diagnostics on save.
    pub fn saved(&mut self, path: &Path) {
        if let Some(running) = self.server_for(path) {
            running.client.notify(
                "textDocument/didSave",
                json!({ "textDocument": { "uri": position::path_to_uri(path) } }),
            );
        }
    }

    /// Whether a server handles `path`.
    pub fn handles(&self, path: &Path) -> bool {
        self.docs
            .get(path)
            .is_some_and(|d| !self.servers[d.server].failed)
    }

    /// The position encoding of `path`'s server.
    pub fn encoding(&self, path: &Path) -> Encoding {
        self.docs
            .get(path)
            .and_then(|d| self.servers[d.server].client.capabilities())
            .map(|c| c.encoding)
            .unwrap_or_default()
    }

    /// Characters that open completion as they are typed (`.`, `::`…).
    pub fn trigger_characters(&self, path: &Path) -> Vec<String> {
        self.docs
            .get(path)
            .and_then(|d| self.servers[d.server].client.capabilities())
            .map(|c| c.trigger_characters)
            .unwrap_or_default()
    }

    pub fn diagnostics(&self, path: &Path) -> &[Diagnostic] {
        self.diagnostics.get(path).map_or(&[], |d| d.as_slice())
    }

    /// The server for `path`, when it is open and running.
    fn server_for(&mut self, path: &Path) -> Option<&mut Running> {
        let index = self.docs.get(path)?.server;
        Some(&mut self.servers[index]).filter(|r| !r.failed)
    }

    fn position_params(&self, path: &Path, rope: &Rope, at: usize) -> serde_json::Value {
        let pos = char_to_pos(rope, at, self.encoding(path));
        json!({
            "textDocument": { "uri": position::path_to_uri(path) },
            "position": pos.to_json(),
        })
    }

    fn send(
        &mut self,
        path: &Path,
        method: &str,
        params: serde_json::Value,
        pending: Pending,
    ) -> bool {
        let Some(index) = self.docs.get(path).map(|d| d.server) else {
            return false;
        };
        let running = &mut self.servers[index];
        if running.failed {
            return false;
        }
        let id = running.client.request(method, params);
        self.pending.insert((index, id), pending);
        true
    }

    /// Ask for hover information at char `at`.
    pub fn hover(&mut self, path: &Path, rope: &Rope, at: usize) -> bool {
        let params = self.position_params(path, rope, at);
        let pending = Pending::Hover {
            path: path.to_path_buf(),
            at,
        };
        self.send(path, "textDocument/hover", params, pending)
    }

    /// Ask for completions at char `at`; `trigger` is the character typed
    /// that asked for them, if any. The answer carries the returned
    /// ticket (older answers can then be dropped).
    pub fn completion(
        &mut self,
        path: &Path,
        rope: &Rope,
        at: usize,
        trigger: Option<&str>,
    ) -> Option<u64> {
        let mut params = self.position_params(path, rope, at);
        params["context"] = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c }),
            None => json!({ "triggerKind": 1 }),
        };
        self.next_ticket += 1;
        let ticket = self.next_ticket;
        let pending = Pending::Completion {
            path: path.to_path_buf(),
            at,
            ticket,
        };
        self.send(path, "textDocument/completion", params, pending)
            .then_some(ticket)
    }

    /// Ask where the symbol at char `at` is defined.
    pub fn definition(&mut self, path: &Path, rope: &Rope, at: usize) -> bool {
        let params = self.position_params(path, rope, at);
        self.send(path, "textDocument/definition", params, Pending::Definition)
    }

    /// What the servers sent since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for index in 0..self.servers.len() {
            let incoming = self.servers[index].client.poll();
            for item in incoming {
                match item {
                    Incoming::Ready => {}
                    Incoming::Failed(message) => {
                        let running = &mut self.servers[index];
                        if running.failed {
                            continue;
                        }
                        running.failed = true;
                        let key = running.key.clone();
                        events.push(Event::Failed {
                            configured: self
                                .settings
                                .servers
                                .get(&key)
                                .is_some_and(|s| s.command.is_some()),
                            server: key,
                            message,
                        });
                        // Its documents keep no stale diagnostics.
                        let paths: Vec<PathBuf> = self
                            .docs
                            .iter()
                            .filter(|(_, d)| d.server == index)
                            .map(|(p, _)| p.clone())
                            .collect();
                        for p in paths {
                            if self.diagnostics.remove(&p).is_some() {
                                events.push(Event::Diagnostics(p));
                            }
                        }
                        self.pending.retain(|(i, _), _| *i != index);
                    }
                    Incoming::Notification { method, params } => {
                        if method == "textDocument/publishDiagnostics" {
                            if let Some((path, list)) = protocol::diagnostics(&params) {
                                self.diagnostics.insert(path.clone(), list);
                                events.push(Event::Diagnostics(path));
                            }
                        }
                    }
                    Incoming::Response { id, result } => {
                        let Some(pending) = self.pending.remove(&(index, id)) else {
                            continue;
                        };
                        let result = result.unwrap_or(serde_json::Value::Null);
                        let encoding = self.servers[index]
                            .client
                            .capabilities()
                            .map(|c| c.encoding)
                            .unwrap_or_default();
                        match pending {
                            Pending::Hover { path, at } => {
                                if let Some(markdown) = protocol::hover_text(&result) {
                                    events.push(Event::Hover { path, at, markdown });
                                }
                            }
                            Pending::Completion { path, at, ticket } => {
                                let (items, incomplete) = protocol::completion_items(&result);
                                events.push(Event::Completion {
                                    path,
                                    at,
                                    ticket,
                                    items,
                                    incomplete,
                                    encoding,
                                });
                            }
                            Pending::Definition => events.push(Event::Definition {
                                targets: protocol::locations(&result),
                                encoding,
                            }),
                        }
                    }
                }
            }
        }
        events
    }

    /// Ask every server to exit (quitting).
    pub fn shutdown(&mut self) {
        for running in &mut self.servers {
            running.client.shutdown();
        }
    }
}

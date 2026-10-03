//! ACP v1 terminal lifecycle. Commands use pipes rather than an interactive
//! PTY; callers receive bounded output and can wait, kill, and release them.
use super::Handler;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_TERMINALS: usize = 64;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Terminals {
    cwd: Mutex<Option<PathBuf>>,
    next: AtomicU64,
    commands: Mutex<HashMap<String, Arc<Terminal>>>,
    closed: AtomicBool,
    cancelled: Mutex<HashMap<String, u64>>,
}
struct Terminal {
    session: String,
    child: Mutex<Child>,
    output: Mutex<Output>,
    readers: AtomicU64,
    publish: Mutex<()>,
}
struct Output {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}
impl Terminal {
    fn publish(&self, handler: &Arc<dyn Handler>, id: &str) {
        // Serialize snapshots from stdout and stderr so an older snapshot
        // cannot arrive after and replace a newer one in the UI.
        let _publish = self.publish.lock().unwrap();
        let text = self.output.lock().unwrap().text();
        handler.on_terminal_output(&self.session, id, &text);
    }
    fn kill(&self) -> Result<(), String> {
        let mut child = self.child.lock().unwrap();
        #[cfg(unix)]
        {
            // Each command owns a process group, including shell descendants
            // that may still hold the stdout/stderr pipes open.
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            let _ = miao_term_platform::background_command("taskkill")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            child.kill().map_err(|e| e.to_string())?;
        }
        child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Output {
    fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > self.limit {
            self.truncated = true;
            self.bytes.drain(..self.bytes.len() - self.limit);
            // A retained suffix must start on a character boundary.
            while self.bytes.first().is_some_and(|b| b & 0xc0 == 0x80) {
                self.bytes.remove(0);
            }
        }
    }
    fn text(&self) -> String {
        // A running command may have written only part of the last character.
        let end = match std::str::from_utf8(&self.bytes) {
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            _ => self.bytes.len(),
        };
        String::from_utf8_lossy(&self.bytes[..end]).into_owned()
    }
}
impl Terminals {
    pub(crate) fn enabled(&self) -> bool {
        self.cwd.lock().unwrap().is_some()
    }
    pub(crate) fn enable(&self, cwd: &Path) {
        *self.cwd.lock().unwrap() = Some(cwd.to_path_buf());
    }
    pub(crate) fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        for (_, terminal) in self.commands.lock().unwrap().drain() {
            let _ = terminal.kill();
        }
    }
    pub(crate) fn cancel(&self, session: &str) {
        *self
            .cancelled
            .lock()
            .unwrap()
            .entry(session.into())
            .or_default() += 1;
    }
    pub(crate) fn permission(&self, params: &Value, handler: &Arc<dyn Handler>) -> Option<bool> {
        let session = params
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let generation = *self.cancelled.lock().unwrap().get(session).unwrap_or(&0);
        let (tx, rx) = mpsc::channel();
        let handler = handler.clone();
        let request = params.clone();
        std::thread::spawn(move || {
            let _ = tx.send(handler.request_permission(&request));
        });
        loop {
            if self.closed.load(Ordering::SeqCst)
                || *self.cancelled.lock().unwrap().get(session).unwrap_or(&0) != generation
            {
                return None;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(allow) => return Some(allow),
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    pub(crate) fn request(
        &self,
        method: &str,
        params: &Value,
        handler: &Arc<dyn Handler>,
    ) -> Result<Value, String> {
        if !self.enabled() || self.closed.load(Ordering::SeqCst) {
            return Err("terminal capability is unavailable".into());
        }
        let session = string(params, "sessionId")?;
        if method == "terminal/create" {
            return self.create(session, params, handler);
        }
        if !matches!(
            method,
            "terminal/output" | "terminal/wait_for_exit" | "terminal/kill" | "terminal/release"
        ) {
            return Err("unknown terminal method".into());
        }
        let id = string(params, "terminalId")?;
        let terminal = self
            .commands
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .filter(|t| t.session == session)
            .ok_or("unknown terminal for session")?;
        match method {
            "terminal/output" => {
                let status = terminal
                    .child
                    .lock()
                    .unwrap()
                    .try_wait()
                    .map_err(|e| e.to_string())?;
                let output = terminal.output.lock().unwrap();
                let mut result = json!({ "output": output.text(), "truncated": output.truncated });
                if let Some(status) = status {
                    result["exitStatus"] = exit_status(status);
                }
                Ok(result)
            }
            "terminal/wait_for_exit" => {
                let mut exited_at = None;
                loop {
                    let status = terminal
                        .child
                        .lock()
                        .unwrap()
                        .try_wait()
                        .map_err(|e| e.to_string())?;
                    if let Some(status) = status {
                        // Give readers time to drain ordinary output, but a
                        // background descendant may keep inherited pipes open.
                        let since = exited_at.get_or_insert_with(Instant::now);
                        if terminal.readers.load(Ordering::SeqCst) == 0
                            || since.elapsed() >= Duration::from_millis(100)
                        {
                            return Ok(exit_status(status));
                        }
                    }
                    if self.closed.load(Ordering::SeqCst) {
                        return Err("terminal client closed".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "terminal/kill" | "terminal/release" => {
                terminal.kill()?;
                if method == "terminal/release" {
                    self.commands.lock().unwrap().remove(id);
                }
                Ok(json!({}))
            }
            _ => unreachable!(),
        }
    }
    fn create(
        &self,
        session: &str,
        params: &Value,
        handler: &Arc<dyn Handler>,
    ) -> Result<Value, String> {
        let command = string(params, "command")?;
        let args = params
            .get("args")
            .map(|v| v.as_array().ok_or("args must be an array"))
            .transpose()?
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().ok_or("args must contain strings"))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let cwd = match params.get("cwd") {
            Some(Value::String(cwd)) => PathBuf::from(cwd),
            None | Some(Value::Null) => self
                .cwd
                .lock()
                .unwrap()
                .clone()
                .ok_or("missing terminal cwd")?,
            _ => return Err("cwd must be a string".into()),
        };
        if !cwd.is_absolute() {
            return Err("cwd must be absolute".into());
        }
        let limit = match params.get("outputByteLimit") {
            None => 1024 * 1024,
            Some(v) => usize::try_from(v.as_u64().ok_or("outputByteLimit must be nonnegative")?)
                .map_err(|_| "outputByteLimit is too large")?,
        };
        if limit > MAX_OUTPUT_BYTES {
            return Err("outputByteLimit exceeds the 16 MiB client limit".into());
        }
        let mut process = miao_term_platform::background_command(command);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        process
            .args(&args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(env) = params.get("env") {
            for entry in env.as_array().ok_or("env must be an array")? {
                process.env(string(entry, "name")?, string(entry, "value")?);
            }
        }
        let title = format!(
            "Run {}",
            std::iter::once(command)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let request = json!({ "sessionId": session, "toolCall": { "toolCallId": "client-terminal", "title": title, "kind": "execute", "rawInput": params }, "options": [{ "optionId": "allow-once", "kind": "allow_once", "name": "Allow once" }, { "optionId": "reject-once", "kind": "reject_once", "name": "Reject" }] });
        if self.permission(&request, handler) != Some(true) {
            return Err("terminal command denied".into());
        }
        let id = format!("term-{}", self.next.fetch_add(1, Ordering::SeqCst));
        // Serialize spawn/insertion against shutdown so no process escapes Drop.
        let mut commands = self.commands.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            return Err("terminal client closed".into());
        }
        if commands.len() >= MAX_TERMINALS {
            return Err("too many active terminals; release a terminal first".into());
        }
        let mut child = process.spawn().map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let terminal = Arc::new(Terminal {
            session: session.into(),
            child: Mutex::new(child),
            output: Mutex::new(Output {
                bytes: Vec::new(),
                limit,
                truncated: false,
            }),
            readers: AtomicU64::new(2),
            publish: Mutex::new(()),
        });
        commands.insert(id.clone(), terminal.clone());
        capture(stdout, terminal.clone(), handler.clone(), id.clone());
        capture(stderr, terminal, handler.clone(), id.clone());
        Ok(json!({ "terminalId": id }))
    }
}
fn capture(
    reader: impl Read + Send + 'static,
    terminal: Arc<Terminal>,
    handler: Arc<dyn Handler>,
    id: String,
) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0; 8192];
        let mut pending = Vec::new();
        let mut last_publish = Instant::now() - Duration::from_millis(50);
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    if !pending.is_empty() {
                        terminal
                            .output
                            .lock()
                            .unwrap()
                            .append(String::from_utf8_lossy(&pending).as_bytes());
                    }
                    terminal.publish(&handler, &id);
                    break;
                }
                Ok(n) => {
                    pending.extend_from_slice(&buffer[..n]);
                    let mut decoded = Vec::new();
                    loop {
                        match std::str::from_utf8(&pending) {
                            Ok(_) => {
                                decoded.append(&mut pending);
                                break;
                            }
                            Err(error) => {
                                decoded.extend(pending.drain(..error.valid_up_to()));
                                if let Some(length) = error.error_len() {
                                    pending.drain(..length);
                                    decoded.extend_from_slice("\u{fffd}".as_bytes());
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    if decoded.is_empty() {
                        continue;
                    }
                    {
                        let mut output = terminal.output.lock().unwrap();
                        output.append(&decoded);
                    }
                    // Avoid enqueueing a full output snapshot for every 8 KiB
                    // when a command produces output faster than the UI draws.
                    if last_publish.elapsed() >= Duration::from_millis(50) {
                        terminal.publish(&handler, &id);
                        last_publish = Instant::now();
                    }
                }
            }
        }
        terminal.readers.fetch_sub(1, Ordering::SeqCst);
    });
}
fn string<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid {key}"))
}
fn exit_status(status: std::process::ExitStatus) -> Value {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|s| s.to_string())
    };
    #[cfg(not(unix))]
    let signal: Option<String> = None;
    json!({ "exitCode": status.code(), "signal": signal })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Allow;
    impl Handler for Allow {
        fn request_permission(&self, _: &Value) -> bool {
            true
        }
    }
    fn service() -> Arc<Terminals> {
        let service = Arc::new(Terminals::default());
        service.enable(&std::env::temp_dir());
        service
    }
    #[test]
    fn bounded_output_keeps_complete_unicode_characters() {
        let mut output = Output {
            bytes: Vec::new(),
            limit: 4,
            truncated: false,
        };
        output.append("a你好".as_bytes());
        assert_eq!(output.text(), "好");
        assert!(output.truncated);
        output.limit = 0;
        output.append(b"x");
        assert_eq!(output.text(), "");
    }
    #[test]
    fn output_wait_release_and_session_isolation() {
        let service = service();
        let handler: Arc<dyn Handler> = Arc::new(Allow);
        #[cfg(unix)]
        let create = json!({ "sessionId": "s", "command": "sh", "args": ["-c", "printf abcdef"], "outputByteLimit": 3 });
        #[cfg(windows)]
        let create = json!({ "sessionId": "s", "command": "cmd", "args": ["/C", "echo abcdef"], "outputByteLimit": 3 });
        let id = service
            .request("terminal/create", &create, &handler)
            .unwrap()["terminalId"]
            .clone();
        let params = json!({ "sessionId": "s", "terminalId": id });
        assert!(service
            .request(
                "terminal/output",
                &json!({ "sessionId": "other", "terminalId": id }),
                &handler
            )
            .is_err());
        let exit = service
            .request("terminal/wait_for_exit", &params, &handler)
            .unwrap();
        assert_eq!(exit["exitCode"], 0);
        let output = service
            .request("terminal/output", &params, &handler)
            .unwrap();
        assert_eq!(output["truncated"], true);
        assert!(output["output"].as_str().unwrap().len() <= 3);
        assert_eq!(output["exitStatus"]["exitCode"], 0);
        service
            .request("terminal/release", &params, &handler)
            .unwrap();
        assert!(service
            .request("terminal/output", &params, &handler)
            .is_err());
        service.shutdown();
    }
    #[test]
    fn kill_unblocks_wait_and_release_invalidates_id() {
        let service = service();
        let handler: Arc<dyn Handler> = Arc::new(Allow);
        #[cfg(unix)]
        let create = json!({ "sessionId": "s", "command": "sh", "args": ["-c", "exec sleep 30"] });
        #[cfg(windows)]
        let create = json!({ "sessionId": "s", "command": "powershell", "args": ["-NoProfile", "-Command", "Start-Sleep -Seconds 30"] });
        let id = service
            .request("terminal/create", &create, &handler)
            .unwrap()["terminalId"]
            .clone();
        let params = json!({ "sessionId": "s", "terminalId": id });
        let waiter = {
            let service = service.clone();
            let params = params.clone();
            let handler = handler.clone();
            std::thread::spawn(move || service.request("terminal/wait_for_exit", &params, &handler))
        };
        service.request("terminal/kill", &params, &handler).unwrap();
        assert!(waiter.join().unwrap().is_ok());
        assert!(service
            .request("terminal/output", &params, &handler)
            .unwrap()
            .get("exitStatus")
            .is_some());
        service
            .request("terminal/release", &params, &handler)
            .unwrap();
        service.shutdown();
    }
    #[test]
    fn denied_command_never_spawns() {
        struct Deny;
        impl Handler for Deny {}
        let service = service();
        let handler: Arc<dyn Handler> = Arc::new(Deny);
        assert!(service
            .request(
                "terminal/create",
                &json!({ "sessionId": "s", "command": "missing-command" }),
                &handler
            )
            .unwrap_err()
            .contains("denied"));
        assert!(service.commands.lock().unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn exited_shell_does_not_wait_for_background_descendant_pipes() {
        let service = service();
        let handler: Arc<dyn Handler> = Arc::new(Allow);
        let id = service
            .request(
                "terminal/create",
                &json!({ "sessionId": "s", "command": "sh", "args": ["-c", "sleep 30 &"] }),
                &handler,
            )
            .unwrap()["terminalId"]
            .clone();
        let params = json!({ "sessionId": "s", "terminalId": id });
        let terminal = service
            .commands
            .lock()
            .unwrap()
            .get(id.as_str().unwrap())
            .unwrap()
            .clone();
        let (tx, rx) = mpsc::channel();
        let waiter_service = service.clone();
        let waiter_handler = handler.clone();
        let waiter_params = params.clone();
        std::thread::spawn(move || {
            let _ = tx.send(waiter_service.request(
                "terminal/wait_for_exit",
                &waiter_params,
                &waiter_handler,
            ));
        });
        let result = rx.recv_timeout(Duration::from_secs(2));
        // Always clean up the descendant, including when the assertion fails.
        service
            .request("terminal/release", &params, &handler)
            .unwrap();
        assert_eq!(result.unwrap().unwrap()["exitCode"], 0);
        let deadline = Instant::now() + Duration::from_secs(2);
        while terminal.readers.load(Ordering::SeqCst) != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(terminal.readers.load(Ordering::SeqCst), 0);
        service.shutdown();
    }
    #[test]
    fn oversized_output_limit_is_rejected_before_spawn() {
        let service = service();
        let handler: Arc<dyn Handler> = Arc::new(Allow);
        assert!(service.request("terminal/create", &json!({ "sessionId": "s", "command": "missing-command", "outputByteLimit": MAX_OUTPUT_BYTES + 1 }), &handler).unwrap_err().contains("limit"));
        assert!(service.commands.lock().unwrap().is_empty());
    }
}

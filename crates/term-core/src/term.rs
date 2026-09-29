//! Terminal: PTY + VT parser/screen + a reader thread.
//!
//! The screen model is `alacritty_terminal` (see [`crate::aterm`], ADR 0001),
//! hidden behind this type so the app never depends on the parser crate.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::aterm::ATerm;

type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Upper bound on a pending OSC 7 sequence held between chunks. Terminal output
/// is untrusted, so an unterminated sequence must not grow this buffer forever.
const MAX_OSC: usize = 8 * 1024;

/// A running terminal: a child shell on a PTY plus the parsed screen state.
pub struct Terminal {
    screen: ATerm,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn Write + Send>,
    #[allow(dead_code)]
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    rows: u16,
    cols: u16,
    exited: bool,
    cwd: Option<String>,
    title: Option<String>,
    osc_buf: Vec<u8>,
    /// Pending bytes scanned for a ConPTY cursor-position query (DSR, `ESC[6n`),
    /// which must be answered or the Windows shell stalls before it runs anything.
    dsr_buf: Vec<u8>,
}

/// Closing the PTY can block: on Windows `ClosePseudoConsole` waits for the
/// client process to exit, so a running shell would stall teardown for minutes
/// (closing a tab or quitting the app). Kill and reap the child *before* the
/// master is dropped.
impl Drop for Terminal {
    fn drop(&mut self) {
        // Teardown must not block: closing a ConPTY (`ClosePseudoConsole`) waits
        // for the client process to exit, which can stall for minutes on Windows
        // when a shell was running — and closing a tab or quitting the app must
        // never hang. Kill the tree, reap, drain, then close off-thread.
        #[cfg(windows)]
        if let Some(pid) = self.child.process_id() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Drain output the shell already produced (bounded).
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        while std::time::Instant::now() < deadline && self.rx.try_recv().is_ok() {}
        if let Some(master) = self.master.take() {
            std::thread::spawn(move || drop(master));
        }
    }
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

impl Terminal {
    /// Spawn a shell on a new PTY.
    pub fn new(
        shell: Option<String>,
        cols: u16,
        rows: u16,
        scrollback: usize,
        cwd: Option<std::path::PathBuf>,
        extra_env: &[(String, String)],
        waker: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Fallible<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell_path = shell.unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&shell_path);
        // Inherit the current environment (PATH, MIAOTTY_SOCKET, …), then override.
        for (k, v) in std::env::vars() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        for (k, v) in crate::shell::env_for(&shell_path) {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        } else if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let waker_thread = waker.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        // Wake the UI so output (e.g. echo of a keystroke) is
                        // drawn promptly instead of on the next blink tick.
                        waker_thread();
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            screen: ATerm::new(cols, rows, scrollback),
            master: Some(pair.master),
            writer,
            child,
            rx,
            rows,
            cols,
            exited: false,
            cwd: None,
            title: None,
            osc_buf: Vec::new(),
            dsr_buf: Vec::new(),
        })
    }

    /// Drain pending PTY output into the screen. Returns true if anything changed.
    pub fn process_pending(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(bytes) => {
                    self.scan_osc(&bytes);
                    self.screen.process(&bytes);
                    self.answer_dsr(&bytes);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.exited = true;
                    break;
                }
            }
        }
        changed
    }

    /// Send raw bytes (keyboard/paste) to the shell.
    pub fn write(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).is_ok() {
            let _ = self.writer.flush();
        }
    }

    /// Resize both the screen model and the PTY if the size changed.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 || (rows == self.rows && cols == self.cols) {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        self.screen.resize(cols, rows);
        let _ = self.master.as_ref().map(|m| {
            m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
        });
    }

    pub fn screen(&self) -> &ATerm {
        &self.screen
    }

    pub fn screen_mut(&mut self) -> &mut ATerm {
        &mut self.screen
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    /// The working directory reported by the shell via OSC 7, if any.
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// The child process id, if the platform exposes one.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// The window title reported via OSC 0/2, if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Answer ConPTY's cursor-position query (DSR `ESC[6n`) with `ESC[<row>;<col>R`.
    ///
    /// Windows ConPTY emits this on startup and the console app blocks until the
    /// terminal replies; without it the Windows shell never produces output.
    fn answer_dsr(&mut self, bytes: &[u8]) {
        self.dsr_buf.extend_from_slice(bytes);
        while let Some(idx) = find_subslice(&self.dsr_buf, b"\x1b[6n") {
            let (row, col) = self.screen.cursor();
            let reply = format!("\x1b[{};{}R", row + 1, col + 1);
            self.dsr_buf.drain(..idx + 4);
            self.write(reply.as_bytes());
        }
        // Keep a short tail in case the sequence is split across chunks.
        let keep = 3.min(self.dsr_buf.len());
        let drain = self.dsr_buf.len() - keep;
        self.dsr_buf.drain(..drain);
    }

    /// Scan a chunk for OSC 7 (`ESC ] 7 ; file://host/path BEL|ST`) and update `cwd`.
    fn scan_osc(&mut self, bytes: &[u8]) {
        self.osc_buf.extend_from_slice(bytes);
        loop {
            let Some(start) = find_subslice(&self.osc_buf, b"\x1b]") else {
                // Keep a short tail in case the prefix is split across chunks.
                let keep = 2.min(self.osc_buf.len());
                let drain = self.osc_buf.len() - keep;
                self.osc_buf.drain(..drain);
                return;
            };
            let after = start + 2;
            let Some(semi_rel) = self.osc_buf[after..].iter().position(|&b| b == b';') else {
                if self.osc_buf.len() - start > MAX_OSC {
                    self.osc_buf.clear();
                } else {
                    self.osc_buf.drain(..start);
                }
                return;
            };
            let semi = after + semi_rel;
            let code = self.osc_buf[after..semi].to_vec();
            match find_terminator(&self.osc_buf[semi + 1..]) {
                Some((end, term_len)) => {
                    if let Ok(payload) =
                        std::str::from_utf8(&self.osc_buf[semi + 1..semi + 1 + end])
                    {
                        match code.as_slice() {
                            b"7" => {
                                if let Some(path) = parse_osc7(payload) {
                                    self.cwd = Some(path);
                                }
                            }
                            b"0" | b"2" => self.title = Some(payload.to_string()),
                            _ => {}
                        }
                    }
                    self.osc_buf.drain(..semi + 1 + end + term_len);
                }
                None => {
                    // No terminator yet: keep the pending sequence bounded.
                    if self.osc_buf.len() - start > MAX_OSC {
                        self.osc_buf.clear();
                    } else {
                        self.osc_buf.drain(..start);
                    }
                    return;
                }
            }
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Returns (payload_len, terminator_len) for BEL or ESC `\`.
fn find_terminator(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x07 {
            return Some((i, 1));
        }
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

fn parse_osc7(payload: &str) -> Option<String> {
    let rest = payload.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    Some(rest[slash..].to_string())
}

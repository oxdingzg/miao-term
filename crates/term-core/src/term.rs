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

/// Nominal cell height (px) used only to advance the cursor below a Sixel image.
/// The host owns real font metrics; this keeps the following prompt below the
/// image without threading the cell size through the engine.
const NOMINAL_CELL_H: u32 = 16;

/// Maximum host metadata payload retained as a title or working directory.
const MAX_OSC: usize = 8 * 1024;

const PTY_READ_BYTES: usize = 8192;
// Backpressure bounds queued output to 256 KiB per pane, plus one reader chunk.
const PTY_QUEUE_CHUNKS: usize = 32;
// Yield between chunks so a continuously writing child cannot monopolize the UI.
const PTY_DRAIN_BYTES: usize = 64 * 1024;
const PTY_DRAIN_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

/// A running terminal: a child shell on a PTY plus the parsed screen state.
pub struct Terminal {
    screen: ATerm,
    /// The PTY master, for a shell session; `None` for a plain byte pipe
    /// (serial, Telnet, raw TCP — ADR 0037).
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn Write + Send>,
    /// The child shell, for a PTY session; `None` for a byte pipe.
    #[allow(dead_code)]
    child: Option<Box<dyn Child + Send + Sync>>,
    rx: Receiver<Vec<u8>>,
    waker: std::sync::Arc<dyn Fn() + Send + Sync>,
    rows: u16,
    cols: u16,
    exited: bool,
    cwd: Option<String>,
    cwd_checked: std::time::Instant,
    cwd_reported: bool,
    title: Option<String>,
    scanner: miao_term_graphics::Scanner,
    graphics: crate::graphics::GraphicsLayer,
    graphics_enabled: bool,
    scrollback: usize,
    cell_px: (u16, u16),
    /// Output bytes of the running command, between OSC 133 `C` and `D`.
    capture: Option<Vec<u8>>,
    /// The last finished command's output (OSC 133 semantic prompts).
    last_output: Option<CommandOutput>,
}

/// A finished command's output, as plain text, with its exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub text: String,
    pub exit: Option<i32>,
    /// The output was longer than the capture limit and was cut.
    pub truncated: bool,
}

/// The most output kept for one command.
const MAX_CAPTURE: usize = 2 * 1024 * 1024;

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
        if let Some(pid) = self.child.as_ref().and_then(|c| c.process_id()) {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
        // Inherit the current environment (PATH, MTTY_SOCKET, …), then override.
        for (k, v) in std::env::vars() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some((k, v)) = default_colour_env(|name| std::env::var_os(name).is_some()) {
            cmd.env(k, v);
        }
        #[cfg(unix)]
        if ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .all(|name| std::env::var(name).map_or(true, |value| value.is_empty()))
        {
            // Finder and minimal launch environments omit the locale. Bash
            // otherwise interprets UTF-8 bytes as Meta keys (including Tab).
            cmd.env(
                "LC_CTYPE",
                if cfg!(target_os = "macos") {
                    "UTF-8"
                } else {
                    "C.UTF-8"
                },
            );
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        // The environment the shell will see, for values the shim restores.
        let seen = |name: &str| {
            extra_env
                .iter()
                .rev()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .or_else(|| std::env::var(name).ok())
        };
        let integration = crate::shell::integration(&shell_path, &seen);
        cmd.args(&integration.args);
        for (k, v) in integration.env {
            cmd.env(k, v);
        }
        let cwd = cwd.or_else(|| std::env::current_dir().ok());
        if let Some(dir) = &cwd {
            cmd.cwd(dir);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(Self::pipe(
            cols,
            rows,
            scrollback,
            reader,
            writer,
            waker,
            Some(pair.master),
            Some(child),
            cwd.map(|p| p.to_string_lossy().into_owned()),
        ))
    }

    /// Build a terminal over a byte stream: a reader thread forwards chunks
    /// into `rx` and wakes the UI, exactly as for a PTY. `master`/`child` are
    /// `None` for serial, Telnet and raw TCP sessions (ADR 0037).
    // A private constructor: the arguments are the terminal's own fields.
    #[allow(clippy::too_many_arguments)]
    fn pipe(
        cols: u16,
        rows: u16,
        scrollback: usize,
        mut reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        waker: std::sync::Arc<dyn Fn() + Send + Sync>,
        master: Option<Box<dyn MasterPty + Send>>,
        child: Option<Box<dyn Child + Send + Sync>>,
        cwd: Option<String>,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(PTY_QUEUE_CHUNKS);
        let waker_thread = waker.clone();
        thread::spawn(move || {
            let mut buf = [0u8; PTY_READ_BYTES];
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
        Self {
            screen: ATerm::new(cols, rows, scrollback),
            master,
            writer,
            child,
            rx,
            waker,
            rows,
            cols,
            exited: false,
            cwd,
            cwd_checked: std::time::Instant::now(),
            cwd_reported: false,
            title: None,
            scanner: miao_term_graphics::Scanner::new(),
            graphics: crate::graphics::GraphicsLayer::new(),
            graphics_enabled: true,
            scrollback,
            capture: None,
            last_output: None,
            cell_px: (0, 0),
        }
    }

    /// Run a terminal over a plain byte pipe with no child process behind it:
    /// a serial port, a Telnet connection or a raw TCP socket (ADR 0037).
    /// `exited()` becomes true when the pipe closes.
    pub fn from_pipe(
        cols: u16,
        rows: u16,
        scrollback: usize,
        reader: impl Read + Send + 'static,
        writer: Box<dyn Write + Send>,
        waker: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::pipe(
            cols,
            rows,
            scrollback,
            Box::new(reader),
            writer,
            waker,
            None,
            None,
            None,
        )
    }

    /// Drain pending PTY output into the screen. Returns true if anything changed.
    pub fn process_pending(&mut self) -> bool {
        let mut changed = false;
        // Shell hooks may be absent or replaced by user startup files. Query
        // the local shell itself at a bounded cadence, without spawning tools.
        if !self.cwd_reported && self.cwd_checked.elapsed() >= std::time::Duration::from_millis(500)
        {
            self.cwd_checked = std::time::Instant::now();
            if let Some(path) = self.pid().and_then(process_cwd) {
                if self.cwd.as_deref() != Some(path.as_str()) {
                    self.cwd = Some(path);
                    changed = true;
                }
            }
        }
        let deadline = std::time::Instant::now() + PTY_DRAIN_BUDGET;
        let mut drained = 0;
        loop {
            match self.rx.try_recv() {
                Ok(bytes) => {
                    drained += bytes.len();
                    changed |= self.feed(&bytes);
                    if drained >= PTY_DRAIN_BYTES || std::time::Instant::now() >= deadline {
                        // A queued continuation is essential: the reader can be
                        // blocked on a full queue and unable to send a new wake.
                        (self.waker)();
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.exited = true;
                    break;
                }
            }
        }
        self.graphics.sync_total(self.screen.total_lines());
        changed
    }

    /// Feed one chunk through the shared control scanner: borrowed text to VT,
    /// ordered metadata/DSR notifications and graphics to their host handlers.
    fn feed(&mut self, bytes: &[u8]) -> bool {
        let mut changed = false;
        let mut line_feeds = 0usize;
        // Move the scanner out temporarily so callbacks can update the screen,
        // metadata, images, and PTY writer without an intermediate segment Vec.
        let mut scanner = std::mem::take(&mut self.scanner);
        scanner.feed_with(bytes, |event| {
            match event {
                miao_term_graphics::StreamEvent::Text(text) => {
                    if !text.is_empty() {
                        // Image-anchor accounting has no purpose in text-only
                        // panes. Avoid an additional full-byte scan in that case.
                        if !self.graphics.images.is_empty() {
                            line_feeds += count_line_feeds(text);
                        }
                        self.screen.process(text);
                        if let Some(buf) = self.capture.as_mut() {
                            let room = MAX_CAPTURE.saturating_sub(buf.len());
                            buf.extend_from_slice(&text[..text.len().min(room + 1)]);
                        }
                        changed = true;
                    }
                }
                miao_term_graphics::StreamEvent::Graphics(graphic) => {
                    if self.graphics_enabled {
                        changed |= self.handle_graphic(graphic);
                    }
                }
                miao_term_graphics::StreamEvent::Osc(payload) => self.observe_osc(payload),
                miao_term_graphics::StreamEvent::CursorReport => {
                    // Respond at the query's position in the stream, after any
                    // preceding text has updated the cursor (including ConPTY).
                    let (row, col) = self.screen.cursor();
                    self.write(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
                }
            }
        });
        self.scanner = scanner;
        // Keep image anchors aligned with content. Up to the ring cap this is
        // exact (the buffer grows); past it the grid rotates without growing, so
        // approximate with the line feeds seen in this chunk.
        let total = self.screen.total_lines();
        let cap = self.scrollback + self.rows as usize;
        if total < cap {
            self.graphics.sync_total(total);
        } else {
            if line_feeds > 0 {
                self.graphics.shift(line_feeds);
            }
            self.graphics.sync_total(total);
        }
        changed
    }

    fn handle_graphic(&mut self, g: miao_term_graphics::Graphic) -> bool {
        use miao_term_graphics as gfx;
        // Align existing anchors before adding to the layer.
        self.graphics.sync_total(self.screen.total_lines());
        let (line, col) = {
            let (r, c) = self.screen.cursor();
            (r as i32, c)
        };
        match g {
            gfx::Graphic::Sixel { transparent, data } => {
                let Some(img) = gfx::sixel::decode(&data, transparent, self.graphics.max_pixels)
                else {
                    return false;
                };
                let rows = img.height.div_ceil(self.cell_h()).max(1);
                self.graphics.place(img, line, col, None, None, 0, 0, 0);
                for _ in 0..rows {
                    self.screen.process(b"\r\n");
                }
                true
            }
            gfx::Graphic::Kitty(cmd) => {
                let move_now = cmd.move_cursor && matches!(cmd.action, 'T' | 'p');
                let rows_hint = cmd.rows;
                let before = self.graphics.images.last().map(|image| image.id);
                let view_offset = self.screen.scroll_offset() as i32;
                // `a=p` places at an explicit cell (viewport row y, column x).
                let (anchor, place_col) = match (cmd.action, cmd.cell_x, cmd.cell_y) {
                    ('p', Some(x), Some(y)) => (y as i32 - view_offset, x),
                    _ => (line, col),
                };
                let changed = self.graphics.kitty(cmd, anchor, place_col, view_offset);
                if move_now
                    && self
                        .graphics
                        .images
                        .last()
                        .is_some_and(|image| Some(image.id) != before)
                {
                    let rows = rows_hint
                        .map(u32::from)
                        .or_else(|| {
                            self.graphics
                                .images
                                .last()
                                .map(|i| i.image.height.div_ceil(self.cell_h()))
                        })
                        .unwrap_or(1)
                        .max(1);
                    for _ in 0..rows {
                        self.screen.process(b"\r\n");
                    }
                }
                changed
            }
            gfx::Graphic::Iterm2 { data, .. } => {
                let Some(img) = gfx::decode_iterm2(&data, self.graphics.max_pixels) else {
                    return false;
                };
                self.graphics.place(img, line, col, None, None, 0, 0, 0);
                true
            }
        }
    }

    /// Enable/disable inline graphics processing (config `graphics`).
    pub fn set_graphics_enabled(&mut self, on: bool) {
        self.graphics_enabled = on;
    }

    /// Report the cell size in physical pixels (used for `TIOCSWINSZ` so image
    /// tools size themselves to the grid, and to advance below Sixel images).
    pub fn set_cell_size(&mut self, w: u16, h: u16) {
        self.cell_px = (w, h);
    }

    fn cell_h(&self) -> u32 {
        let h = self.cell_px.1 as u32;
        if h == 0 {
            NOMINAL_CELL_H
        } else {
            h
        }
    }

    /// Feed bytes as if they came from the PTY (tests only).
    #[cfg(test)]
    pub(crate) fn feed_for_test(&mut self, bytes: &[u8]) -> bool {
        self.feed(bytes)
    }

    /// Decoded inline images (for the renderer).
    pub fn graphics(&self) -> &crate::graphics::GraphicsLayer {
        &self.graphics
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
        let (pw, ph) = (
            cols.saturating_mul(self.cell_px.0),
            rows.saturating_mul(self.cell_px.1),
        );
        let _ = self.master.as_ref().map(|m| {
            m.resize(PtySize {
                rows,
                cols,
                pixel_width: pw,
                pixel_height: ph,
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
        self.child.as_ref().and_then(|c| c.process_id())
    }

    /// The program running in the foreground of this pane (`vim`, `cargo`),
    /// or `None` while the shell itself is waiting for input. Unix only.
    pub fn foreground_command(&self) -> Option<String> {
        #[cfg(unix)]
        {
            let leader = self.master.as_ref()?.process_group_leader()?;
            let leader = u32::try_from(leader).ok()?;
            if Some(leader) == self.pid() {
                return None;
            }
            process_name(leader)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// The foreground program's arguments (`["vim", "notes.md"]`), to offer
    /// running it again after a restart. `None` while the shell itself waits
    /// for input. Unix only.
    pub fn foreground_args(&self) -> Option<Vec<String>> {
        #[cfg(unix)]
        {
            let leader = self.master.as_ref()?.process_group_leader()?;
            let leader = u32::try_from(leader).ok()?;
            if Some(leader) == self.pid() {
                return None;
            }
            process_args(leader).filter(|args| !args.is_empty())
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// The file the foreground program was started on: its first argument that
    /// is not an option and names an existing file, resolved against the
    /// program's working directory (`src/main.rs` for `vim src/main.rs`).
    /// `None` while the shell itself waits for input.
    pub fn foreground_file(&self) -> Option<String> {
        #[cfg(unix)]
        {
            let leader = self.master.as_ref()?.process_group_leader()?;
            let leader = u32::try_from(leader).ok()?;
            if Some(leader) == self.pid() {
                return None;
            }
            let args = process_args(leader)?;
            file_argument(args.get(1..)?, process_cwd(leader).as_deref())
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// The last finished command's output, when the shell marks commands
    /// with OSC 133 (the mtty zsh integration does).
    pub fn last_command_output(&self) -> Option<&CommandOutput> {
        self.last_output.as_ref()
    }

    /// OSC 133: `C` starts a command's output, `D[;exit]` ends it.
    fn observe_semantic_prompt(&mut self, payload: &[u8]) {
        match payload.first() {
            Some(b'C') => self.capture = Some(Vec::new()),
            Some(b'D') => {
                let Some(buf) = self.capture.take() else {
                    return;
                };
                let exit = std::str::from_utf8(payload)
                    .ok()
                    .and_then(|p| p.split(';').nth(1))
                    .and_then(|code| code.trim().parse().ok());
                let truncated = buf.len() > MAX_CAPTURE;
                let end = buf.len().min(MAX_CAPTURE);
                self.last_output = Some(CommandOutput {
                    text: plain_text(&buf[..end]),
                    exit,
                    truncated,
                });
            }
            _ => {}
        }
    }

    /// The window title reported via OSC 0/2, if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Observe a complete OSC from the shared stream scanner, without copying
    /// raw PTY chunks into a second protocol buffer.
    fn observe_osc(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_OSC {
            return;
        }
        let Some(semi) = bytes.iter().position(|&b| b == b';') else {
            return;
        };
        let code = &bytes[..semi];
        if code == b"133" {
            self.observe_semantic_prompt(&bytes[semi + 1..]);
            return;
        }
        if !matches!(code, b"0" | b"2" | b"7") {
            return;
        }
        let Ok(payload) = std::str::from_utf8(&bytes[semi + 1..]) else {
            return;
        };
        match code {
            b"7" => {
                if let Some(path) = parse_osc7(payload) {
                    self.cwd = Some(path);
                    self.cwd_reported = true;
                }
            }
            b"0" | b"2" => self.title = Some(payload.to_string()),
            _ => {}
        }
    }
}

/// Terminal output as plain text: escape sequences removed, `\r\n` as
/// newlines, a bare `\r` overwriting its line (progress bars), backspace
/// erasing, trailing blank space trimmed.
pub fn plain_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then a final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC / DCS / APC / PM / SOS: until BEL or ST (ESC \).
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                // Two-byte escapes (ESC 7, ESC ( B, …): drop the next char.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                line.clear();
            }
            '\n' => lines.push(std::mem::take(&mut line)),
            '\x08' => {
                line.pop();
            }
            c if c.is_control() && c != '\t' => {}
            c => line.push(c),
        }
    }
    lines.push(line);
    let joined: Vec<String> = lines
        .into_iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    joined.join("\n").trim_end().to_string()
}

/// Colour on by default for tools that ask for it (BSD/macOS `ls` with
/// `CLICOLOR`), unless the user chose otherwise: an existing `CLICOLOR`, or
/// `NO_COLOR` (no-color.org). Tools still colour only when writing to a
/// terminal, so pipes and redirected output stay plain.
fn default_colour_env(is_set: impl Fn(&str) -> bool) -> Option<(&'static str, &'static str)> {
    (!is_set("CLICOLOR") && !is_set("NO_COLOR")).then_some(("CLICOLOR", "1"))
}

/// A process's short name (`comm`).
#[cfg(unix)]
fn process_name(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 256];
        // proc_name writes at most `buf.len()` bytes and returns the length.
        let n = unsafe {
            libc::proc_name(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if n <= 0 {
            return None;
        }
        String::from_utf8(buf[..n as usize].to_vec()).ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// The first of `args` that is not an option and is an existing file, as an
/// absolute path (relative ones resolved against `cwd`).
#[cfg(unix)]
fn file_argument(args: &[String], cwd: Option<&str>) -> Option<String> {
    args.iter()
        .filter(|a| !a.is_empty() && !a.starts_with('-') && !a.starts_with('+'))
        .find_map(|a| {
            let path = std::path::Path::new(a);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::path::Path::new(cwd?).join(path)
            };
            path.is_file().then(|| path.to_string_lossy().into_owned())
        })
}

/// A process's command line (`argv`), program name first.
#[cfg(unix)]
fn process_args(pid: u32) -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        // KERN_PROCARGS2: argc (a C int), the executable path, NUL padding,
        // then argc NUL-terminated arguments, then the environment.
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
        let mut size: libc::size_t = 0;
        // First call sizes the buffer; the second fills it.
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok != 0 || size < 4 {
            return None;
        }
        let mut buf = vec![0u8; size];
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok != 0 {
            return None;
        }
        buf.truncate(size);
        parse_procargs2(&buf)
    }
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let args: Vec<String> = raw
            .split(|&b| b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        (!args.is_empty()).then_some(args)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// Split a `KERN_PROCARGS2` buffer into its arguments.
#[cfg(any(target_os = "macos", test))]
fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let rest = &buf[4..];
    // Skip the executable path and the NULs that pad it.
    let exec_end = rest.iter().position(|&b| b == 0)?;
    let start = exec_end + rest[exec_end..].iter().position(|&b| b != 0)?;
    let args: Vec<String> = rest[start..]
        .split(|&b| b == 0)
        .take(argc)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    (args.len() == argc).then_some(args)
}

fn process_cwd(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>();
        // proc_pidinfo writes this fixed-size structure only on success.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                info.as_mut_ptr().cast(),
                size as libc::c_int,
            )
        };
        if n as usize != size {
            return None;
        }
        let info = unsafe { info.assume_init() };
        let bytes: Vec<u8> = info
            .pvi_cdir
            .vip_path
            .iter()
            .flatten()
            .map(|&b| b as u8)
            .take_while(|&b| b != 0)
            .collect();
        String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// Count line-advancing controls in a chunk (`\n`, IND `ESC D`, NEL `ESC E`).
fn count_line_feeds(bytes: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                n += 1;
                i += 1;
            }
            0x1b if i + 1 < bytes.len() && (bytes[i + 1] == b'D' || bytes[i + 1] == b'E') => {
                n += 1;
                i += 2;
            }
            _ => i += 1,
        }
    }
    n
}

fn parse_osc7(payload: &str) -> Option<String> {
    let rest = payload.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let bytes = &rest.as_bytes()[slash..];
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded)
        .ok()
        .filter(|s| !s.contains('\0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_is_on_by_default_unless_the_user_chose() {
        let set = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
        assert_eq!(default_colour_env(set(&[])), Some(("CLICOLOR", "1")));
        assert_eq!(default_colour_env(set(&["CLICOLOR"])), None, "keep theirs");
        assert_eq!(default_colour_env(set(&["NO_COLOR"])), None, "no-color.org");
    }

    fn make() -> Terminal {
        let waker: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {});
        Terminal::new(None, 20, 5, 100, None, &[], waker).expect("spawn shell")
    }

    /// A writer that records what the terminal sends over the pipe.
    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A terminal over a plain byte pipe has no child, consumes what the pipe
    /// sends and reports EOF as exit (ADR 0037).
    #[test]
    fn a_pipe_terminal_reads_and_writes_without_a_child() {
        let waker: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {});
        let out = SharedBuf::default();
        let writer: Box<dyn Write + Send> = Box::new(out.clone());
        let reader = std::io::Cursor::new(b"\x1b[31mhi\x1b[0m\r\n".to_vec());
        let mut term = Terminal::from_pipe(20, 5, 100, reader, writer, waker);
        assert_eq!(term.pid(), None);
        assert_eq!(term.foreground_command(), None);
        for _ in 0..200 {
            term.process_pending();
            if term.exited() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(term.exited(), "a closed pipe ends the session");
        assert_eq!(term.screen().line_text(0).trim_end(), "hi");
        term.write(b"ping");
        assert_eq!(&*out.0.lock().unwrap(), b"ping");
        term.resize(6, 30);
    }

    #[test]
    #[ignore = "release full input-path benchmark"]
    fn full_input_path_throughput() {
        let mut term = make();
        let line = b"\x1b[31mhello\x1b[0m world \x1b[1;32mfoo\x1b[0m bar 1234567890\r\n";
        let mut data = Vec::with_capacity(16 << 20);
        while data.len() < 16 << 20 {
            data.extend_from_slice(line);
        }
        for chunk in data[..64 * 1024].chunks(8192) {
            term.feed(chunk);
        }
        let start = std::time::Instant::now();
        for chunk in data.chunks(8192) {
            term.feed(chunk);
        }
        let mbps = data.len() as f64 / 1e6 / start.elapsed().as_secs_f64();
        println!("full input path (8 KiB chunks): {mbps:.2} MB/s");
        let scale = std::env::var("MTTY_PERF_SCALE")
            .or_else(|_| std::env::var("MIAOTTY_PERF_SCALE"))
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        assert!(mbps > 15.0 / scale, "full input-path throughput regression");
    }

    #[test]
    fn output_queue_backpressure_and_drain_continuation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let mut term = make();
        let wakes = Arc::new(AtomicUsize::new(0));
        let notified = Arc::clone(&wakes);
        term.waker = Arc::new(move || {
            notified.fetch_add(1, Ordering::Relaxed);
        });
        let (tx, rx) = mpsc::sync_channel(PTY_QUEUE_CHUNKS);
        term.rx = rx;
        for _ in 0..PTY_QUEUE_CHUNKS {
            tx.try_send(vec![b'x'; PTY_READ_BYTES]).unwrap();
        }
        assert!(matches!(
            tx.try_send(b"\r\ndrain complete".to_vec()),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert!(term.process_pending());
        assert!(
            wakes.load(Ordering::Relaxed) > 0,
            "yield must schedule continuation"
        );
        assert!(!term.exited());
        tx.try_send(b"\r\ndrain complete".to_vec()).unwrap();
        drop(tx);
        // Even with no further producer wakes, the queued continuation drains
        // all output in order and eventually observes EOF.
        for _ in 0..PTY_QUEUE_CHUNKS + 2 {
            term.process_pending();
            if term.exited() {
                break;
            }
        }
        assert!(term.exited());
        assert!(term
            .screen()
            .contents_between(0, 0, 4, 19)
            .contains("drain complete"));
    }

    #[test]
    fn unified_stream_updates_metadata_and_replies_at_the_query_position() {
        use std::sync::{Arc, Mutex};
        struct Replies(Arc<Mutex<Vec<u8>>>);
        impl Write for Replies {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let stream = b"abc\x1b[6n\r\nxy\x1b[6n\x1b]2;new title\x1b\\\x1b]7;file://localhost/tmp/a%20b\x07done";
        for chunk_size in [1, 2, 3, 4, 8, stream.len()] {
            let mut term = make();
            let replies = Arc::new(Mutex::new(Vec::new()));
            term.writer = Box::new(Replies(Arc::clone(&replies)));
            for chunk in stream.chunks(chunk_size) {
                term.feed(chunk);
            }
            assert_eq!(&*replies.lock().unwrap(), b"\x1b[1;4R\x1b[2;3R");
            assert_eq!(term.title(), Some("new title"));
            assert_eq!(term.cwd(), Some("/tmp/a b"));
            assert_eq!(term.screen.line_text(0), "abc");
            assert_eq!(term.screen.line_text(1), "xydone");
        }
    }

    #[test]
    fn osc7_decodes_directory_uri() {
        assert_eq!(
            parse_osc7("file://localhost/tmp/a%20b/%E7%9B%AE%E5%BD%95"),
            Some("/tmp/a b/目录".into())
        );
        assert!(parse_osc7("file://localhost/tmp/%00").is_none());
        assert!(parse_osc7("file://localhost/tmp/%ZZ").is_none());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn shell_directory_tracks_cd_without_osc_hooks() {
        let root = std::env::temp_dir().join(format!("mtty-cwd-{}", std::process::id()));
        let next = root.join("space 目录");
        std::fs::create_dir_all(&next).unwrap();
        let mut term = Terminal::new(
            Some("/bin/sh".into()),
            80,
            24,
            100,
            Some(root.clone()),
            &[],
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        assert_eq!(term.cwd(), Some(root.to_string_lossy().as_ref()));
        term.write("cd 'space 目录'\r".as_bytes());
        let expected = std::fs::canonicalize(&next)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while term.cwd() != Some(expected.as_str()) && std::time::Instant::now() < deadline {
            term.process_pending();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            term.cwd(),
            Some(expected.as_str()),
            "directory panels must follow a plain shell's cd; screen: {}",
            term.screen().contents_between(0, 0, 23, 79)
        );
        drop(term);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn foreground_command_names_the_running_program() {
        let mut term = Terminal::new(
            Some("/bin/sh".into()),
            80,
            24,
            100,
            None,
            &[],
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        let wait_for = |term: &mut Terminal, want: Option<&str>| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                term.process_pending();
                let got = term.foreground_command();
                if got.as_deref() == want || std::time::Instant::now() > deadline {
                    return got;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        assert_eq!(
            wait_for(&mut term, None),
            None,
            "an idle shell has no command"
        );
        term.write(b"sleep 3\r");
        assert_eq!(wait_for(&mut term, Some("sleep")).as_deref(), Some("sleep"));
    }

    #[cfg(unix)]
    #[test]
    fn file_argument_skips_options_and_missing_paths() {
        let dir = std::env::temp_dir().join(format!("mtty-fg-file-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let want = dir.join("src/main.rs").to_string_lossy().to_string();
        assert_eq!(
            file_argument(
                &args(&["-R", "+12", "missing.rs", "src/main.rs"]),
                Some(&cwd)
            ),
            Some(want.clone())
        );
        assert_eq!(file_argument(&args(&[&want]), None), Some(want));
        assert_eq!(
            file_argument(&args(&["src"]), Some(&cwd)),
            None,
            "a directory"
        );
        assert_eq!(file_argument(&args(&["src/main.rs"]), None), None, "no cwd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn procargs2_buffers_split_into_arguments() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/vim\0\0\0\0vim\0notes.md\0HOME=/x\0");
        assert_eq!(
            parse_procargs2(&buf),
            Some(vec!["vim".to_string(), "notes.md".to_string()])
        );
        assert_eq!(parse_procargs2(&buf[..3]), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn foreground_file_names_the_file_the_program_opened() {
        let dir = std::env::temp_dir().join(format!("mtty-fg-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.md");
        std::fs::write(&file, "x\n").unwrap();
        let mut term = Terminal::new(
            Some("/bin/sh".into()),
            80,
            24,
            100,
            Some(dir.clone()),
            &[],
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        assert_eq!(term.foreground_file(), None, "an idle shell has no file");
        term.write(b"tail -f notes.md\r");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut got = None;
        while got.is_none() && std::time::Instant::now() < deadline {
            term.process_pending();
            got = term.foreground_file();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let want = std::fs::canonicalize(&file).unwrap();
        let got = got.map(|g| std::fs::canonicalize(g).unwrap());
        assert_eq!(got, Some(want));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run `shell` through its integration in an empty HOME (the user's own
    /// startup files stay out) and check command output capture, exit codes
    /// and the cwd it reports after `cd`.
    fn integration_end_to_end(shell: &str, ok_then_fail: &str, fail_code: i32) {
        let home = std::env::temp_dir().join(format!(
            "mtty-shell-home-{}-{}",
            shell.rsplit(['/', '\\']).next().unwrap(),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("a dir")).unwrap();
        let home_s = home.to_string_lossy().to_string();
        let env = [
            ("HOME".to_string(), home_s.clone()),
            ("USERPROFILE".to_string(), home_s.clone()),
            ("XDG_CONFIG_HOME".to_string(), format!("{home_s}/.config")),
        ];
        let mut term = Terminal::new(
            Some(shell.into()),
            100,
            24,
            100,
            Some(home.clone()),
            &env,
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        let wait = |term: &mut Terminal, done: &dyn Fn(&Terminal) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !done(term) && std::time::Instant::now() < deadline {
                term.process_pending();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        // Line editors such as PSReadLine start after the prompt is drawn and
        // drop a typed-ahead Enter, so after the prompt give them a moment,
        // as a person would.
        // `cwd()` starts as the spawn directory, so it cannot tell that the
        // shell is up. Nothing has been typed yet, so anything on screen is
        // the shell's own (banner or prompt): wait for that, as long as a cold
        // PowerShell start on a CI runner needs.
        let started = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while (0..24).all(|r| term.screen().line_text(r).trim().is_empty())
            && std::time::Instant::now() < started
        {
            term.process_pending();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            (0..24).any(|r| !term.screen().line_text(r).trim().is_empty()),
            "{shell}: no prompt within 60 s"
        );
        let settle = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while std::time::Instant::now() < settle {
            term.process_pending();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        term.write(ok_then_fail.as_bytes());
        std::thread::sleep(std::time::Duration::from_millis(200));
        term.process_pending();
        // A slow machine may still be loading the line editor: if the Enter
        // was dropped (no output after a while), press it again. An extra
        // Enter on an empty line runs nothing.
        for _ in 0..3 {
            term.write(b"\r");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            while term.last_command_output().is_none() && std::time::Instant::now() < deadline {
                term.process_pending();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if term.last_command_output().is_some() {
                break;
            }
        }
        let out = term.last_command_output().cloned().unwrap_or_else(|| {
            let screen: Vec<String> = (0..24).map(|r| term.screen().line_text(r)).collect();
            panic!(
                "{shell}: no OSC 133 C/D; screen:\n{}",
                screen.join("\n").trim_end()
            )
        });
        assert_eq!(out.text, "first\nsecond", "{shell}");
        assert_eq!(out.exit, Some(fail_code), "{shell}");
        term.write("echo 中文\r".as_bytes());
        wait(&mut term, &|t| {
            t.last_command_output().is_some_and(|o| o.exit == Some(0))
        });
        assert_eq!(term.last_command_output().unwrap().text, "中文", "{shell}");
        term.write(b"cd 'a dir'\r");
        wait(&mut term, &|t| {
            t.cwd().is_some_and(|c| c.ends_with("a dir"))
        });
        let cwd = term.cwd().map(str::to_string);
        assert!(
            cwd.as_deref().is_some_and(|c| c.ends_with("a dir")),
            "{shell}: cwd {cwd:?}"
        );
        drop(term);
        let _ = std::fs::remove_dir_all(&home);
        eprintln!("shell integration checked: {shell}");
    }

    /// The shells installed here (CI images differ); missing ones are skipped.
    fn installed(candidates: &[&str]) -> Vec<String> {
        let mut found: Vec<String> = candidates
            .iter()
            .filter(|p| std::path::Path::new(p).exists())
            .map(|p| p.to_string())
            .collect();
        found.dedup();
        found
    }

    #[cfg(unix)]
    #[test]
    fn zsh_integration_marks_commands_end_to_end() {
        for zsh in installed(&["/bin/zsh", "/usr/bin/zsh"]).into_iter().take(1) {
            integration_end_to_end(&zsh, "printf 'first\\nsecond\\n'; false", 1);
        }
    }

    /// bash 3.2 (macOS, DEBUG trap) and 4.4+ (PS0) both.
    #[cfg(unix)]
    #[test]
    fn bash_integration_marks_commands_end_to_end() {
        for bash in installed(&["/bin/bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash"]) {
            integration_end_to_end(&bash, "printf 'first\\nsecond\\n'; (exit 3)", 3);
        }
    }

    #[cfg(unix)]
    #[test]
    fn fish_integration_marks_commands_end_to_end() {
        let fish = installed(&[
            "/usr/bin/fish",
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
        ]);
        for fish in fish.into_iter().take(1) {
            integration_end_to_end(&fish, "printf 'first\\nsecond\\n'; false", 1);
        }
    }

    #[test]
    fn powershell_integration_marks_commands_end_to_end() {
        let pwsh = installed(&[
            "/usr/bin/pwsh",
            "/usr/local/bin/pwsh",
            "/opt/homebrew/bin/pwsh",
            "/snap/bin/pwsh",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
        ]);
        let pwsh = pwsh.into_iter().next().or_else(|| {
            std::env::var("MTTY_TEST_PWSH")
                .ok()
                .filter(|p| std::path::Path::new(p).exists())
        });
        if let Some(pwsh) = pwsh {
            let fail = if cfg!(windows) {
                "'first'; 'second'; cmd /c exit 4"
            } else {
                "'first'; 'second'; sh -c 'exit 4'"
            };
            integration_end_to_end(&pwsh, fail, 4);
        }
    }

    #[test]
    fn plain_text_drops_escapes_and_keeps_the_last_overwrite() {
        let raw =
            b"\x1b[31mred\x1b[0m line\r\nprogress 10%\rprogress 100%\r\nab\x08c\x1b]0;title\x07\n";
        assert_eq!(plain_text(raw), "red line\nprogress 100%\nac");
        assert_eq!(plain_text("中文 输出\r\n".as_bytes()), "中文 输出");
    }

    #[test]
    fn semantic_prompts_capture_the_last_command_output() {
        let mut t = make();
        t.feed_for_test(b"$ \x1b]133;C\x07hello\r\n\x1b[1mworld\x1b[0m\r\n\x1b]133;D;1\x07$ ");
        let out = t.last_command_output().unwrap();
        assert_eq!(out.text, "hello\nworld");
        assert_eq!(out.exit, Some(1));
        assert!(!out.truncated);
        // A D without a C (the first prompt) changes nothing.
        t.feed_for_test(b"\x1b]133;D;0\x07");
        assert_eq!(t.last_command_output().unwrap().exit, Some(1));
        // The ST terminator works too, and a missing exit code is None.
        t.feed_for_test(b"\x1b]133;C\x1b\\ok\r\n\x1b]133;D\x1b\\");
        let out = t.last_command_output().unwrap();
        assert_eq!((out.text.as_str(), out.exit), ("ok", None));
    }

    #[test]
    fn sixel_reaches_the_graphics_layer() {
        let mut t = make();
        t.feed_for_test(b"\x1bPq#0;2;100;0;0#0~~~~~~~~~~\x1b\\");
        let imgs = &t.graphics().images;
        assert_eq!(imgs.len(), 1, "one image placed");
        assert_eq!((imgs[0].image.width, imgs[0].image.height), (10, 6));
        // Red at the top-left pixel.
        assert_eq!(&imgs[0].image.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn kitty_rgba_reaches_the_graphics_layer() {
        let mut t = make();
        // 2x2 RGBA (f=32) transmit+display with an explicit 8x8 cell footprint.
        t.feed_for_test(b"\x1b_Ga=T,f=32,s=2x2,i=1,c=8,r=8;/wAA//8AAP//AAD//wAA/w==\x1b\\");
        let imgs = &t.graphics().images;
        assert_eq!(imgs.len(), 1, "one image placed");
        assert_eq!((imgs[0].image.width, imgs[0].image.height), (2, 2));
        assert_eq!(&imgs[0].image.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!((imgs[0].cols, imgs[0].rows), (Some(8), Some(8)));
    }

    #[test]
    fn kitty_cursor_advance_survives_old_image_eviction() {
        let mut term = make();
        term.graphics.max_image_bytes = 4;
        for id in [1, 2] {
            term.feed_for_test(format!("\x1b_Ga=T,f=24,s=1x1,C=0,i={id};AAAA\x1b\\").as_bytes());
            assert_eq!(term.graphics.images.len(), 1);
            assert_eq!(term.screen.cursor().0, id as u16);
        }
    }

    #[test]
    fn kitty_put_uses_cell_coordinates() {
        let mut t = make();
        // a=p at x=5,y=2 with a 1x1 raw RGB pixel (base64 "AAAA").
        t.feed_for_test(b"\x1b_Ga=p,f=24,s=1x1,x=5,y=2;AAAA\x1b\\");
        let imgs = &t.graphics().images;
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].col, 5);
        assert_eq!(imgs[0].anchor, 2);
    }

    #[test]
    fn cell_height_drives_sixel_cursor_advance() {
        let mut t = make(); // 5-row screen
        t.set_cell_size(8, 16);
        // One 6px band => ceil(6/16) = 1 row below.
        t.feed_for_test(b"\x1bPq#0;2;100;0;0#0@\x1b\\");
        assert_eq!(t.screen().cursor().0, 1, "cursor moved one row down");
        // Same image with a 6px cell => still one row.
        let mut t2 = make();
        t2.set_cell_size(8, 6);
        t2.feed_for_test(b"\x1bPq#0;2;100;0;0#0@\x1b\\");
        assert_eq!(t2.screen().cursor().0, 1);
    }

    #[test]
    fn graphics_can_be_disabled() {
        let mut t = make();
        t.set_graphics_enabled(false);
        t.feed_for_test(b"\x1bPq#0;2;100;0;0#0~~~~~~~~~~\x1b\\");
        assert!(t.graphics().images.is_empty());
    }

    #[test]
    fn plain_text_still_flows() {
        let mut t = make();
        t.feed_for_test(b"hi\r\n");
        assert_eq!(t.screen().line_text(0), "hi");
        assert!(t.graphics().images.is_empty());
    }
}

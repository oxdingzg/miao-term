//! The host process (ADR 0041 §1, Unix): owns the PTY and the program, keeps
//! the output ring, and serves one client at a time over a private socket.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::proto::{FromHost, ToHost, PROTO};
use crate::ring::Ring;
use crate::scanner::Scanner;

/// A pending sequence longer than this is sent anyway (as a frame that does
/// not end on a boundary) rather than held without bound.
const MAX_PENDING: usize = 1 << 20;

/// A client that does not take its output within this long is dropped.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// After `Kill`, the program gets this long to exit on `SIGHUP`.
const KILL_GRACE: Duration = Duration::from_millis(250);

/// How long the host waits for the program's last output after it exits.
const LAST_OUTPUT_WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostArgs {
    pub socket: PathBuf,
    /// Metadata for the recovered-sessions list, removed at exit.
    pub meta: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub ring: usize,
    /// End the program after this long without a client.
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
    pub argv: Vec<OsString>,
}

impl HostArgs {
    /// `--socket P [--meta M] [--cols N] [--rows N] [--ring BYTES]
    /// [--timeout SECS] [--cwd DIR] -- PROGRAM ARGS…`
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut socket = None;
        let mut out = HostArgs {
            socket: PathBuf::new(),
            meta: None,
            cols: 80,
            rows: 24,
            ring: 8 << 20,
            timeout: Duration::from_secs(24 * 60 * 60),
            cwd: None,
            argv: Vec::new(),
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let name = arg.to_string_lossy().into_owned();
            if name == "--" {
                out.argv = args.collect();
                break;
            }
            let value = args.next().ok_or_else(|| format!("{name} needs a value"))?;
            let text = value.to_string_lossy().into_owned();
            let number = |what: &str| -> Result<u64, String> {
                text.parse()
                    .map_err(|_| format!("{what}: not a number: {text}"))
            };
            match name.as_str() {
                "--socket" => socket = Some(PathBuf::from(value)),
                "--meta" => out.meta = Some(PathBuf::from(value)),
                "--cols" => out.cols = number("--cols")?.clamp(1, u16::MAX as u64) as u16,
                "--rows" => out.rows = number("--rows")?.clamp(1, u16::MAX as u64) as u16,
                "--ring" => out.ring = number("--ring")? as usize,
                "--timeout" => out.timeout = Duration::from_secs(number("--timeout")?),
                "--cwd" => out.cwd = Some(PathBuf::from(value)),
                other => return Err(format!("unknown option {other}")),
            }
        }
        out.socket = socket.ok_or("--socket is required")?;
        if out.argv.is_empty() {
            return Err("no program after --".into());
        }
        Ok(out)
    }

    /// The command line for [`HostArgs::parse`].
    pub fn to_args(&self) -> Vec<OsString> {
        let mut out: Vec<OsString> = vec!["--socket".into(), self.socket.clone().into()];
        if let Some(meta) = &self.meta {
            out.extend(["--meta".into(), meta.clone().into()]);
        }
        for (name, value) in [
            ("--cols", self.cols.to_string()),
            ("--rows", self.rows.to_string()),
            ("--ring", self.ring.to_string()),
            ("--timeout", self.timeout.as_secs().to_string()),
        ] {
            out.extend([name.into(), value.into()]);
        }
        if let Some(cwd) = &self.cwd {
            out.extend(["--cwd".into(), cwd.clone().into()]);
        }
        out.push("--".into());
        out.extend(self.argv.iter().cloned());
        out
    }
}

struct Shared {
    ring: Ring,
    /// The attached client: its id and the stream output goes to.
    client: Option<(u64, UnixStream)>,
    /// Since when no client has been attached.
    detached_since: Option<Instant>,
    /// The program's exit status and the output offset at exit.
    exited: Option<(i32, u64)>,
    /// When the last client that saw the exit left (the host can end).
    exit_collected: bool,
    /// A client asked to end the program.
    killed: Option<Instant>,
}

impl Shared {
    /// Send to the attached client; a client that cannot take it is dropped.
    fn send(&mut self, message: &FromHost) {
        let Some((_, stream)) = self.client.as_mut() else {
            return;
        };
        if message.write(stream).is_err() {
            self.drop_client();
        }
    }

    fn drop_client(&mut self) {
        if let Some((_, stream)) = self.client.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.detached_since = Some(Instant::now());
        if self.exited.is_some() {
            self.exit_collected = true;
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The peer runs as the same user.
fn same_user(stream: &UnixStream) -> bool {
    let fd = stream.as_raw_fd();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred`/`len` describe a valid buffer for SO_PEERCRED.
        let ok = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        } == 0;
        // SAFETY: getuid has no preconditions.
        ok && cred.uid == unsafe { libc::getuid() }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: getpeereid writes the two ids on success.
        let ok = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0;
        // SAFETY: getuid has no preconditions.
        ok && uid == unsafe { libc::getuid() }
    }
}

fn write_meta(args: &HostArgs, child_pid: u32, started_at: u64) {
    let Some(path) = &args.meta else {
        return;
    };
    let quote = |s: &str| {
        let mut out = String::from('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    let program = args
        .argv
        .first()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let json = format!(
        "{{\"version\":{},\"proto\":{PROTO},\"pid\":{},\"child_pid\":{child_pid},\
         \"started_at\":{started_at},\"socket\":{},\"program\":{}}}\n",
        quote(crate::VERSION),
        std::process::id(),
        quote(&args.socket.to_string_lossy()),
        quote(&program),
    );
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Run the host until the program has ended and a client has seen it, or no
/// client came back within the timeout. Returns the process exit code.
pub fn run(args: HostArgs) -> io::Result<i32> {
    // SAFETY: plain libc calls without preconditions. Leave the app's session
    // so its hang-up never reaches us, and ignore hang-ups ourselves.
    unsafe {
        libc::setsid();
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: args.rows,
            cols: args.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(io::Error::other)?;
    let mut cmd = CommandBuilder::from_argv(args.argv.clone());
    if let Some(cwd) = &args.cwd {
        cmd.cwd(cwd);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);
    let child_pid = child.process_id().unwrap_or(0);
    let started_at = now_secs();
    let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().map_err(io::Error::other)?,
    ));
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));

    let _ = std::fs::remove_file(&args.socket);
    let listener = UnixListener::bind(&args.socket)?;
    let _ = std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o600));
    write_meta(&args, child_pid, started_at);

    let shared = Arc::new(Mutex::new(Shared {
        ring: Ring::new(args.ring),
        client: None,
        detached_since: Some(Instant::now()),
        exited: None,
        exit_collected: false,
        killed: None,
    }));

    // Output: scan, frame at boundaries, keep, forward.
    let (output_done_tx, output_done) = std::sync::mpsc::channel::<()>();
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut scanner = Scanner::new();
            let mut pending: Vec<u8> = Vec::new();
            let mut pending_modes = scanner.modes().clone();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if pending.is_empty() {
                    pending_modes = scanner.modes().clone();
                }
                let before = pending.len();
                let cut = scanner.feed(&buf[..n]);
                pending.extend_from_slice(&buf[..n]);
                let (frame, boundary) = if cut > 0 {
                    let rest = pending.split_off(before + cut);
                    (std::mem::replace(&mut pending, rest), true)
                } else if pending.len() > MAX_PENDING {
                    (std::mem::take(&mut pending), false)
                } else {
                    continue;
                };
                let modes_after = scanner.modes().clone();
                let modes_before = std::mem::replace(&mut pending_modes, modes_after.clone());
                let mut s = shared.lock().unwrap();
                let offset = s.ring.end();
                s.send(&FromHost::Output {
                    offset,
                    boundary,
                    bytes: frame.clone(),
                });
                s.ring.push(boundary, frame, modes_before, modes_after);
            }
            let _ = output_done_tx.send(());
        });
    }

    // The program's exit, once its last output is in.
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let status = child
                .wait()
                .map_or(-1, |s| i32::try_from(s.exit_code()).unwrap_or(-1));
            let _ = output_done.recv_timeout(LAST_OUTPUT_WAIT);
            let mut s = shared.lock().unwrap();
            let offset = s.ring.end();
            s.exited = Some((status, offset));
            s.send(&FromHost::Exited { status, offset });
        });
    }

    // Clients.
    {
        let shared = shared.clone();
        let listener = listener.try_clone()?;
        std::thread::spawn(move || {
            let mut next_id = 0u64;
            for stream in listener.incoming().flatten() {
                if !same_user(&stream) {
                    continue;
                }
                next_id += 1;
                let id = next_id;
                let shared = shared.clone();
                let writer = writer.clone();
                let master = master.clone();
                std::thread::spawn(move || {
                    let _ = serve(id, stream, &shared, &writer, &master, child_pid, started_at);
                });
            }
        });
    }

    // Housekeeping: end once the exit was seen, or after the timeout.
    let code = loop {
        std::thread::sleep(Duration::from_millis(100));
        let s = shared.lock().unwrap();
        if let Some((status, _)) = s.exited {
            let done = s.exit_collected
                || s.killed.is_some()
                || s.detached_since.is_some_and(|t| t.elapsed() > args.timeout);
            if done {
                break status;
            }
            continue;
        }
        // Killed but still not gone (it ignores even SIGKILL's group): stop.
        if s.killed.is_some_and(|t| t.elapsed() > KILL_GRACE * 8) {
            break -1;
        }
        if s.detached_since.is_some_and(|t| t.elapsed() > args.timeout) {
            drop(s);
            hang_up(child_pid);
            break -1;
        }
    };
    let _ = std::fs::remove_file(&args.socket);
    if let Some(meta) = &args.meta {
        let _ = std::fs::remove_file(meta);
    }
    Ok(code)
}

/// `SIGHUP` to the program's process group (it leads its own session), then
/// `SIGKILL` after a grace period.
fn hang_up(child_pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(child_pid) else {
        return;
    };
    if pid <= 0 {
        return;
    }
    // SAFETY: plain signal delivery to a known process group.
    unsafe {
        libc::kill(-pid, libc::SIGHUP);
        libc::kill(pid, libc::SIGHUP);
    }
    std::thread::spawn(move || {
        std::thread::sleep(KILL_GRACE);
        // SAFETY: as above.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    });
}

fn serve(
    id: u64,
    stream: UnixStream,
    shared: &Mutex<Shared>,
    writer: &Mutex<Box<dyn Write + Send>>,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    child_pid: u32,
    started_at: u64,
) -> io::Result<()> {
    let mut reader = stream.try_clone()?;
    reader.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut out = stream.try_clone()?;
    out.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))?;
    match ToHost::read(&mut reader)? {
        ToHost::Hello { .. } => {}
        _ => return Ok(()),
    }
    FromHost::Welcome {
        proto: PROTO,
        caps: 0,
        child_pid,
        started_at,
        host_version: crate::VERSION.into(),
    }
    .write(&mut out)?;
    reader.set_read_timeout(None)?;
    let result = (|| -> io::Result<()> {
        loop {
            match ToHost::read(&mut reader)? {
                ToHost::Hello { .. } => {}
                ToHost::Attach { from } => attach(id, &out, from, shared)?,
                ToHost::Input(bytes) => {
                    let mut w = writer.lock().unwrap();
                    w.write_all(&bytes)?;
                    w.flush()?;
                }
                ToHost::Resize {
                    cols,
                    rows,
                    px_w,
                    px_h,
                } => {
                    let _ = master.lock().unwrap().resize(PtySize {
                        rows: rows.max(1),
                        cols: cols.max(1),
                        pixel_width: px_w,
                        pixel_height: px_h,
                    });
                }
                ToHost::Kill => {
                    shared.lock().unwrap().killed = Some(Instant::now());
                    hang_up(child_pid);
                    return Ok(());
                }
                ToHost::Detach => return Ok(()),
            }
        }
    })();
    let mut s = shared.lock().unwrap();
    if s.client.as_ref().is_some_and(|(current, _)| *current == id) {
        s.drop_client();
    }
    result
}

/// Make `stream` the client: replay from `from`, then stream live output.
/// Holding the lock throughout keeps replay and live output in order.
fn attach(id: u64, stream: &UnixStream, from: u64, shared: &Mutex<Shared>) -> io::Result<()> {
    let mut s = shared.lock().unwrap();
    if let Some((old, previous)) = s.client.take() {
        if old != id {
            let mut previous = previous;
            let _ = FromHost::Detached.write(&mut previous);
            let _ = previous.shutdown(std::net::Shutdown::Both);
        }
    }
    let mut out = stream.try_clone()?;
    let replay = s.ring.since(from);
    if let Some((oldest, modes)) = replay.truncated {
        FromHost::Truncated {
            oldest,
            modes: modes.encode(),
        }
        .write(&mut out)?;
    }
    for frame in replay.frames {
        FromHost::Output {
            offset: frame.offset,
            boundary: frame.boundary,
            bytes: frame.bytes,
        }
        .write(&mut out)?;
    }
    FromHost::Live {
        offset: s.ring.end(),
    }
    .write(&mut out)?;
    if let Some((status, offset)) = s.exited {
        FromHost::Exited { status, offset }.write(&mut out)?;
    }
    s.client = Some((id, out));
    s.detached_since = None;
    Ok(())
}

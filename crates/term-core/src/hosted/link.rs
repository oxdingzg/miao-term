//! The app's connection to one host (Unix).

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use miao_term_ptyhost::client;
use miao_term_ptyhost::proto::{FromHost, ToHost};
use portable_pty::CommandBuilder;

use super::Incoming;

enum Conn {
    /// Messages sent before the host answered, in order.
    Pending(Vec<ToHost>),
    Open(UnixStream),
    Closed,
}

/// The app's side of one host.
pub(crate) struct HostLink {
    pub id: String,
    pub socket: PathBuf,
    conn: Arc<Mutex<Conn>>,
    child_pid: Arc<AtomicU32>,
    /// The output offset processed so far, and whether it is a boundary.
    pub offset: u64,
    pub boundary: bool,
    /// Replaying output the program already had answered: replies are
    /// dropped and resizes wait until the replay ends.
    pub replaying: bool,
    /// Attached to a host that was running before: nudge the program to
    /// redraw once the replay is done.
    pub reattached: bool,
    pub pending_resize: Option<(u16, u16)>,
    /// What to run in the app instead if the host never answers.
    pub fallback: Option<CommandBuilder>,
    /// Dropping the terminal ends the program (closing a pane) unless the
    /// pane was detached to outlive the app.
    pub end_on_drop: bool,
}

impl HostLink {
    pub fn send(&self, message: ToHost) {
        let mut conn = self.conn.lock().unwrap();
        match &mut *conn {
            Conn::Pending(queue) => queue.push(message),
            Conn::Open(stream) => {
                if message.write(stream).is_err() {
                    *conn = Conn::Closed;
                }
            }
            Conn::Closed => {}
        }
    }

    pub fn child_pid(&self) -> Option<u32> {
        Some(self.child_pid.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }

    pub fn writer(&self) -> Box<dyn Write + Send> {
        Box::new(HostWriter(self.conn.clone()))
    }
}

/// Keystrokes and paste as `Input` frames.
struct HostWriter(Arc<Mutex<Conn>>);

impl Write for HostWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut conn = self.0.lock().unwrap();
        let message = ToHost::Input(buf.to_vec());
        match &mut *conn {
            Conn::Pending(queue) => queue.push(message),
            Conn::Open(stream) => {
                if let Err(e) = message.write(stream) {
                    *conn = Conn::Closed;
                    return Err(e);
                }
            }
            Conn::Closed => return Err(io::ErrorKind::BrokenPipe.into()),
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Connect (unless `stream` already is), attach from `from`, and forward
/// what the host sends until it ends or another client takes over.
pub(crate) fn start(
    socket: PathBuf,
    id: String,
    stream: Option<UnixStream>,
    from: u64,
    tx: SyncSender<Incoming>,
    waker: Arc<dyn Fn() + Send + Sync>,
) -> HostLink {
    let conn = Arc::new(Mutex::new(Conn::Pending(Vec::new())));
    let child_pid = Arc::new(AtomicU32::new(0));
    let link = HostLink {
        id,
        socket: socket.clone(),
        conn: conn.clone(),
        child_pid: child_pid.clone(),
        offset: from,
        boundary: true,
        replaying: false,
        reattached: false,
        pending_resize: None,
        fallback: None,
        end_on_drop: true,
    };
    std::thread::spawn(move || {
        let send = |message: Incoming| {
            let ok = tx.send(message).is_ok();
            waker();
            ok
        };
        let mut stream = match stream {
            Some(stream) => stream,
            None => match client::connect(&socket, Duration::from_secs(2)) {
                Ok((stream, welcome)) => {
                    child_pid.store(welcome.child_pid, Ordering::Relaxed);
                    stream
                }
                Err(e) => {
                    *conn.lock().unwrap() = Conn::Closed;
                    send(Incoming::HostFailed(e.to_string()));
                    return;
                }
            },
        };
        // Attach first, then whatever was typed or resized meanwhile.
        {
            let mut c = conn.lock().unwrap();
            let opened = stream.try_clone().and_then(|mut w| {
                ToHost::Attach { from }.write(&mut w)?;
                if let Conn::Pending(queue) = std::mem::replace(&mut *c, Conn::Closed) {
                    for message in queue {
                        message.write(&mut w)?;
                    }
                }
                Ok(w)
            });
            match opened {
                Ok(w) => *c = Conn::Open(w),
                Err(e) => {
                    drop(c);
                    send(Incoming::HostFailed(e.to_string()));
                    return;
                }
            }
        }
        loop {
            let forwarded = match FromHost::read(&mut stream) {
                Ok(FromHost::Output {
                    offset,
                    boundary,
                    bytes,
                }) => {
                    let end = offset + bytes.len() as u64;
                    send(Incoming::Hosted {
                        bytes,
                        end,
                        boundary,
                    })
                }
                Ok(FromHost::Truncated { modes, .. }) => send(Incoming::Truncated(modes)),
                Ok(FromHost::Live { .. }) => send(Incoming::Live),
                Ok(FromHost::Welcome { .. }) => true,
                // The program ended, another client took over, or the host
                // is gone: this pane's stream is over.
                Ok(FromHost::Exited { .. } | FromHost::Detached) | Err(_) => false,
            };
            if !forwarded {
                break;
            }
        }
        *conn.lock().unwrap() = Conn::Closed;
    });
    link
}

/// Connect to a running host and greet it, failing fast: used at startup to
/// decide between reattaching and a fresh shell.
pub(crate) fn connect_existing(socket: &std::path::Path) -> io::Result<(UnixStream, u32)> {
    let (stream, welcome) = client::connect(socket, Duration::from_millis(300))?;
    Ok((stream, welcome.child_pid))
}

impl HostLink {
    pub fn set_child_pid(&self, pid: u32) {
        self.child_pid.store(pid, Ordering::Relaxed);
    }
}

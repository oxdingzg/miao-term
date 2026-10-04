//! Connecting to a host (ADR 0041 §4).

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::proto::{FromHost, ToHost, PROTO};
use crate::sys::Stream as UnixStream;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub proto: u16,
    pub caps: u32,
    pub child_pid: u32,
    pub started_at: u64,
    pub host_version: String,
}

/// Connect and greet. A host that is still starting (no socket yet, or not
/// listening) is retried until `wait` has passed.
pub fn connect(socket: &Path, wait: Duration) -> io::Result<(UnixStream, Welcome)> {
    let deadline = Instant::now() + wait;
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(stream) => break stream,
            // Not there yet, or not listening yet (Windows reports these as
            // assorted socket errors): retry until the deadline.
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e),
        }
    };
    let left = deadline.saturating_duration_since(Instant::now());
    stream.set_read_timeout(Some(left.max(Duration::from_millis(500))))?;
    ToHost::Hello {
        proto: PROTO,
        client_version: crate::VERSION.into(),
    }
    .write(&mut stream)?;
    let welcome = match FromHost::read(&mut stream)? {
        FromHost::Welcome {
            proto,
            caps,
            child_pid,
            started_at,
            host_version,
        } => Welcome {
            proto,
            caps,
            child_pid,
            started_at,
            host_version,
        },
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected a welcome, got {other:?}"),
            ))
        }
    };
    if welcome.proto != PROTO {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("host speaks protocol {}", welcome.proto),
        ));
    }
    stream.set_read_timeout(None)?;
    Ok((stream, welcome))
}

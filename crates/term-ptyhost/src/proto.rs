//! The host protocol, v1 (ADR 0041 §4): length-prefixed binary frames
//! `type: u8, len: u32 LE, payload`.
//!
//! Hosts outlive app versions, so v1 never changes: additions are optional
//! capability bits in `Welcome::caps`, and every app keeps a client for
//! every version it can meet. The golden-frame tests pin the encoding.

use std::io::{self, Read, Write};

/// The protocol version this build speaks.
pub const PROTO: u16 = 1;

/// The largest payload accepted; a longer frame means a broken stream.
pub const MAX_PAYLOAD: usize = 4 << 20;

/// Client → host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToHost {
    Hello {
        proto: u16,
        client_version: String,
    },
    /// Replay output from `from` (an absolute byte offset), then stream.
    Attach {
        from: u64,
    },
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        px_w: u16,
        px_h: u16,
    },
    /// Hang up the program and end the host.
    Kill,
    /// Stop streaming; the program keeps running.
    Detach,
}

/// Host → client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromHost {
    Welcome {
        proto: u16,
        caps: u32,
        child_pid: u32,
        /// Seconds since the Unix epoch.
        started_at: u64,
        host_version: String,
    },
    /// Output starting at byte `offset`. `boundary` is false only when the
    /// frame ends inside an escape sequence or a UTF-8 character (a pending
    /// sequence grew past the host's limit).
    Output {
        offset: u64,
        boundary: bool,
        bytes: Vec<u8>,
    },
    /// The ring no longer reaches the requested offset; replay starts at
    /// `oldest`. `modes` is the scanner's private-mode state at `oldest`.
    Truncated { oldest: u64, modes: Vec<u8> },
    /// Replay is complete; what follows is live.
    Live { offset: u64 },
    /// The program ended; `status` is its exit code (-1 when unknown).
    Exited { status: i32, offset: u64 },
    /// Another client attached; this connection is done.
    Detached,
}

const HELLO: u8 = 0x01;
const ATTACH: u8 = 0x02;
const INPUT: u8 = 0x03;
const RESIZE: u8 = 0x04;
const KILL: u8 = 0x05;
const DETACH: u8 = 0x06;
const WELCOME: u8 = 0x81;
const OUTPUT: u8 = 0x82;
const TRUNCATED: u8 = 0x83;
const LIVE: u8 = 0x84;
const EXITED: u8 = 0x85;
const DETACHED: u8 = 0x86;

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

impl ToHost {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ToHost::Hello {
                proto,
                client_version,
            } => {
                let mut p = proto.to_le_bytes().to_vec();
                p.extend_from_slice(client_version.as_bytes());
                frame(HELLO, &p)
            }
            ToHost::Attach { from } => frame(ATTACH, &from.to_le_bytes()),
            ToHost::Input(bytes) => frame(INPUT, bytes),
            ToHost::Resize {
                cols,
                rows,
                px_w,
                px_h,
            } => {
                let mut p = Vec::with_capacity(8);
                for v in [cols, rows, px_w, px_h] {
                    p.extend_from_slice(&v.to_le_bytes());
                }
                frame(RESIZE, &p)
            }
            ToHost::Kill => frame(KILL, &[]),
            ToHost::Detach => frame(DETACH, &[]),
        }
    }

    fn decode(kind: u8, p: &[u8]) -> io::Result<Self> {
        let mut r = Fields(p);
        Ok(match kind {
            HELLO => ToHost::Hello {
                proto: r.u16()?,
                client_version: r.rest_str(),
            },
            ATTACH => ToHost::Attach { from: r.u64()? },
            INPUT => ToHost::Input(p.to_vec()),
            RESIZE => ToHost::Resize {
                cols: r.u16()?,
                rows: r.u16()?,
                px_w: r.u16()?,
                px_h: r.u16()?,
            },
            KILL => ToHost::Kill,
            DETACH => ToHost::Detach,
            other => return Err(bad(&format!("unknown client frame {other:#x}"))),
        })
    }

    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let (kind, payload) = read_frame(reader)?;
        Self::decode(kind, &payload)
    }

    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(&self.encode())
    }
}

impl FromHost {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            FromHost::Welcome {
                proto,
                caps,
                child_pid,
                started_at,
                host_version,
            } => {
                let mut p = proto.to_le_bytes().to_vec();
                p.extend_from_slice(&caps.to_le_bytes());
                p.extend_from_slice(&child_pid.to_le_bytes());
                p.extend_from_slice(&started_at.to_le_bytes());
                p.extend_from_slice(host_version.as_bytes());
                frame(WELCOME, &p)
            }
            FromHost::Output {
                offset,
                boundary,
                bytes,
            } => {
                let mut p = Vec::with_capacity(9 + bytes.len());
                p.extend_from_slice(&offset.to_le_bytes());
                p.push(u8::from(*boundary));
                p.extend_from_slice(bytes);
                frame(OUTPUT, &p)
            }
            FromHost::Truncated { oldest, modes } => {
                let mut p = oldest.to_le_bytes().to_vec();
                p.extend_from_slice(modes);
                frame(TRUNCATED, &p)
            }
            FromHost::Live { offset } => frame(LIVE, &offset.to_le_bytes()),
            FromHost::Exited { status, offset } => {
                let mut p = status.to_le_bytes().to_vec();
                p.extend_from_slice(&offset.to_le_bytes());
                frame(EXITED, &p)
            }
            FromHost::Detached => frame(DETACHED, &[]),
        }
    }

    fn decode(kind: u8, p: &[u8]) -> io::Result<Self> {
        let mut r = Fields(p);
        Ok(match kind {
            WELCOME => FromHost::Welcome {
                proto: r.u16()?,
                caps: r.u32()?,
                child_pid: r.u32()?,
                started_at: r.u64()?,
                host_version: r.rest_str(),
            },
            OUTPUT => FromHost::Output {
                offset: r.u64()?,
                boundary: r.u8()? != 0,
                bytes: r.rest().to_vec(),
            },
            TRUNCATED => FromHost::Truncated {
                oldest: r.u64()?,
                modes: r.rest().to_vec(),
            },
            LIVE => FromHost::Live { offset: r.u64()? },
            EXITED => FromHost::Exited {
                status: r.u32()? as i32,
                offset: r.u64()?,
            },
            DETACHED => FromHost::Detached,
            other => return Err(bad(&format!("unknown host frame {other:#x}"))),
        })
    }

    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let (kind, payload) = read_frame(reader)?;
        Self::decode(kind, &payload)
    }

    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(&self.encode())
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn read_frame(reader: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    reader.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(bad("frame too long"));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

/// Little-endian fields read in order.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        if self.0.len() < N {
            return Err(bad("short frame"));
        }
        let (head, rest) = self.0.split_at(N);
        self.0 = rest;
        Ok(head.try_into().expect("split at N"))
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn rest(&mut self) -> &[u8] {
        std::mem::take(&mut self.0)
    }
    fn rest_str(&mut self) -> String {
        String::from_utf8_lossy(self.rest()).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1 is frozen: these bytes must never change.
    #[test]
    fn golden_frames() {
        let hello = ToHost::Hello {
            proto: 1,
            client_version: "0.1".into(),
        };
        assert_eq!(hello.encode(), b"\x01\x05\0\0\0\x01\x000.1");
        assert_eq!(
            ToHost::Attach { from: 258 }.encode(),
            b"\x02\x08\0\0\0\x02\x01\0\0\0\0\0\0"
        );
        assert_eq!(
            ToHost::Input(b"ls\r".to_vec()).encode(),
            b"\x03\x03\0\0\0ls\r"
        );
        assert_eq!(
            ToHost::Resize {
                cols: 80,
                rows: 24,
                px_w: 0,
                px_h: 0
            }
            .encode(),
            b"\x04\x08\0\0\0\x50\0\x18\0\0\0\0\0"
        );
        assert_eq!(ToHost::Kill.encode(), b"\x05\0\0\0\0");
        assert_eq!(ToHost::Detach.encode(), b"\x06\0\0\0\0");
        let welcome = FromHost::Welcome {
            proto: 1,
            caps: 0,
            child_pid: 7,
            started_at: 9,
            host_version: "v".into(),
        };
        assert_eq!(
            welcome.encode(),
            b"\x81\x13\0\0\0\x01\0\0\0\0\0\x07\0\0\0\x09\0\0\0\0\0\0\0v"
        );
        let output = FromHost::Output {
            offset: 1,
            boundary: true,
            bytes: b"hi".to_vec(),
        };
        assert_eq!(output.encode(), b"\x82\x0b\0\0\0\x01\0\0\0\0\0\0\0\x01hi");
        assert_eq!(
            FromHost::Truncated {
                oldest: 2,
                modes: vec![9]
            }
            .encode(),
            b"\x83\x09\0\0\0\x02\0\0\0\0\0\0\0\x09"
        );
        assert_eq!(
            FromHost::Live { offset: 3 }.encode(),
            b"\x84\x08\0\0\0\x03\0\0\0\0\0\0\0"
        );
        assert_eq!(
            FromHost::Exited {
                status: -1,
                offset: 4
            }
            .encode(),
            b"\x85\x0c\0\0\0\xff\xff\xff\xff\x04\0\0\0\0\0\0\0"
        );
        assert_eq!(FromHost::Detached.encode(), b"\x86\0\0\0\0");
    }

    #[test]
    fn frames_round_trip() {
        let to = [
            ToHost::Hello {
                proto: PROTO,
                client_version: "x".into(),
            },
            ToHost::Attach { from: u64::MAX },
            ToHost::Input(vec![0, 255]),
            ToHost::Resize {
                cols: 1,
                rows: 2,
                px_w: 3,
                px_h: 4,
            },
            ToHost::Kill,
            ToHost::Detach,
        ];
        for m in to {
            assert_eq!(ToHost::read(&mut &m.encode()[..]).unwrap(), m);
        }
        let from = [
            FromHost::Welcome {
                proto: PROTO,
                caps: 5,
                child_pid: 6,
                started_at: 7,
                host_version: "y".into(),
            },
            FromHost::Output {
                offset: 8,
                boundary: false,
                bytes: vec![1, 2],
            },
            FromHost::Truncated {
                oldest: 9,
                modes: vec![],
            },
            FromHost::Live { offset: 10 },
            FromHost::Exited {
                status: 3,
                offset: 11,
            },
            FromHost::Detached,
        ];
        for m in from {
            assert_eq!(FromHost::read(&mut &m.encode()[..]).unwrap(), m);
        }
    }

    #[test]
    fn broken_streams_are_errors() {
        assert!(ToHost::read(&mut &b"\x02\x01\0\0\0x"[..]).is_err(), "short");
        assert!(ToHost::read(&mut &b"\x7f\0\0\0\0"[..]).is_err(), "unknown");
        assert!(
            FromHost::read(&mut &b"\x82\xff\xff\xff\xff"[..]).is_err(),
            "huge"
        );
        assert!(FromHost::read(&mut &b"\x82\x02\0\0\0"[..]).is_err(), "eof");
    }
}

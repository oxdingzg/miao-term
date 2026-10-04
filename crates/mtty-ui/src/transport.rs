//! Byte-stream transports for serial, Telnet and raw TCP sessions (ADR 0037).
//!
//! The terminal core only needs a reader and a writer
//! (`mtty-core::Terminal::from_pipe`); these build one from a saved
//! profile, with no child process behind it. Everything here is GPU-free and
//! unit-tested.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// A live byte-stream connection: the pane reads from `reader` and writes to
/// `writer`, and labels itself with `label`.
pub struct Connection {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub label: String,
}

/// Connect a raw TCP socket. `timeout` bounds only the connect.
pub fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<Connection, String> {
    let stream = connect_stream(host, port, timeout)?;
    let label = format!("tcp {host}:{port}");
    let writer = stream.try_clone().map_err(|e| e.to_string())?;
    Ok(Connection {
        reader: Box::new(stream),
        writer: Box::new(writer),
        label,
    })
}

/// Connect a Telnet session: the data stream has IAC negotiation stripped and
/// answered, and writes have IAC escaped, so the terminal core sees only text.
pub fn connect_telnet(host: &str, port: u16, timeout: Duration) -> Result<Connection, String> {
    let stream = connect_stream(host, port, timeout)?;
    let reply = stream.try_clone().map_err(|e| e.to_string())?;
    let out = stream.try_clone().map_err(|e| e.to_string())?;
    let label = format!("telnet {host}:{port}");
    Ok(Connection {
        reader: Box::new(TelnetReader::new(stream, Box::new(reply))),
        writer: Box::new(TelnetWriter::new(out)),
        label,
    })
}

/// A serial port's settings, saved in `hosts.toml` (ADR 0037).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialConfig {
    pub device: String,
    pub baud: u32,
    /// 5, 6, 7 or 8.
    pub data_bits: u8,
    /// "none", "odd" or "even".
    pub parity: String,
    /// 1 or 2.
    pub stop_bits: u8,
    /// "none", "software" or "hardware".
    pub flow: String,
}

impl SerialConfig {
    pub fn label(&self) -> String {
        format!("{} @{}", self.device, self.baud)
    }
}

/// Open a serial port as a byte pipe.
pub fn connect_serial(cfg: &SerialConfig) -> Result<Connection, String> {
    use serial2::{CharSize, FlowControl, Parity, StopBits};
    let char_size = match cfg.data_bits {
        5 => CharSize::Bits5,
        6 => CharSize::Bits6,
        7 => CharSize::Bits7,
        _ => CharSize::Bits8,
    };
    let parity = match cfg.parity.as_str() {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    };
    let stop_bits = if cfg.stop_bits == 2 {
        StopBits::Two
    } else {
        StopBits::One
    };
    let flow = match cfg.flow.as_str() {
        "software" => FlowControl::XonXoff,
        "hardware" => FlowControl::RtsCts,
        _ => FlowControl::None,
    };
    let port = serial2::SerialPort::open(&cfg.device, |mut settings: serial2::Settings| {
        settings.set_raw();
        settings.set_baud_rate(cfg.baud)?;
        settings.set_char_size(char_size);
        settings.set_parity(parity);
        settings.set_stop_bits(stop_bits);
        settings.set_flow_control(flow);
        Ok(settings)
    })
    .map_err(|e| format!("{}: {e}", cfg.device))?;
    let reader = port.try_clone().map_err(|e| e.to_string())?;
    Ok(Connection {
        reader: Box::new(reader),
        writer: Box::new(port),
        label: cfg.label(),
    })
}

/// Serial device names to offer in the profile form: the platform's own
/// enumeration where it has one, else a `/dev` scan.
pub fn serial_ports() -> Vec<String> {
    let mut out: Vec<String> = serial2::SerialPort::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if out.is_empty() {
        if let Ok(dir) = std::fs::read_dir("/dev") {
            for entry in dir.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("tty.")
                    || name.starts_with("cu.")
                    || name.starts_with("ttyUSB")
                    || name.starts_with("ttyACM")
                    || name.starts_with("ttyS")
                {
                    out.push(format!("/dev/{name}"));
                }
            }
        }
        out.sort();
    }
    out
}

fn connect_stream(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("{host}:{port}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host}:{port}: no address"))?;
    let stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{host}:{port}: {e}"))?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

// Telnet control bytes (RFC 854, 855).
const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const ECHO: u8 = 1;

/// Reads a Telnet stream: strips IAC sequences, answers negotiation (accepting
/// server echo, refusing everything else) and yields only data bytes.
pub struct TelnetReader<R: Read> {
    inner: R,
    /// Where negotiation replies go: a clone of the socket's write half.
    reply: Box<dyn Write + Send>,
    state: State,
    /// The last reply sent for each option, so repeated requests stay quiet.
    answered: [u8; 256],
    pending: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    Negotiate(u8),
    Sub,
    SubIac,
}

impl<R: Read> TelnetReader<R> {
    pub fn new(inner: R, reply: Box<dyn Write + Send>) -> Self {
        TelnetReader {
            inner,
            reply,
            state: State::Data,
            answered: [0; 256],
            pending: Vec::new(),
        }
    }

    /// Answer an option negotiation once per state change.
    fn answer(&mut self, command: u8, option: u8) {
        let (reply, want) = match command {
            WILL if option == ECHO => (DO, DO),
            WILL => (DONT, DONT),
            DO => (WONT, WONT),
            // WONT/DONT end an option: nothing to send back.
            _ => return,
        };
        if self.answered[option as usize] == want {
            return;
        }
        self.answered[option as usize] = want;
        let _ = self.reply.write_all(&[IAC, reply, option]);
        let _ = self.reply.flush();
    }

    /// Push one incoming byte through the state machine.
    fn step(&mut self, byte: u8) {
        match self.state {
            State::Data => {
                if byte == IAC {
                    self.state = State::Iac;
                } else {
                    self.pending.push(byte);
                }
            }
            State::Iac => match byte {
                // A doubled IAC is a literal 0xFF.
                IAC => {
                    self.pending.push(IAC);
                    self.state = State::Data;
                }
                WILL | WONT | DO | DONT => self.state = State::Negotiate(byte),
                SB => self.state = State::Sub,
                // Single-byte commands (NOP, Go Ahead, …) are dropped.
                _ => self.state = State::Data,
            },
            State::Negotiate(command) => {
                self.answer(command, byte);
                self.state = State::Data;
            }
            State::Sub => {
                if byte == IAC {
                    self.state = State::SubIac;
                }
            }
            State::SubIac => match byte {
                SE => self.state = State::Data,
                // A doubled IAC inside a subnegotiation is a literal byte.
                IAC => self.state = State::Sub,
                _ => self.state = State::Sub,
            },
        }
    }
}

impl<R: Read> Read for TelnetReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if !self.pending.is_empty() {
                let n = buf.len().min(self.pending.len());
                buf[..n].copy_from_slice(&self.pending[..n]);
                self.pending.drain(..n);
                return Ok(n);
            }
            let mut chunk = [0u8; 4096];
            let n = self.inner.read(&mut chunk)?;
            if n == 0 {
                return Ok(0);
            }
            for &b in &chunk[..n] {
                self.step(b);
            }
        }
    }
}

/// Writes a Telnet stream: every 0xFF is doubled so data cannot look like IAC.
pub struct TelnetWriter<W: Write> {
    inner: W,
}

impl<W: Write> TelnetWriter<W> {
    pub fn new(inner: W) -> Self {
        TelnetWriter { inner }
    }
}

impl<W: Write> Write for TelnetWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut escaped = Vec::with_capacity(buf.len());
        for &b in buf {
            if b == IAC {
                escaped.push(IAC);
            }
            escaped.push(b);
        }
        self.inner.write_all(&escaped)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);
    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn reader(input: &[u8], reply: SharedBuf) -> TelnetReader<Cursor<Vec<u8>>> {
        TelnetReader::new(Cursor::new(input.to_vec()), Box::new(reply))
    }

    fn read_all(mut r: TelnetReader<Cursor<Vec<u8>>>) -> Vec<u8> {
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn telnet_accepts_server_echo_and_refuses_the_rest() {
        let reply = SharedBuf::default();
        // IAC WILL ECHO, then IAC WILL TERMINAL-TYPE, then data.
        let input = [IAC, WILL, ECHO, IAC, WILL, 24, b'h', b'i'];
        assert_eq!(read_all(reader(&input, reply.clone())), b"hi");
        assert_eq!(
            &*reply.0.lock().unwrap(),
            &[IAC, DO, ECHO, IAC, DONT, 24],
            "accept echo, refuse terminal type"
        );
    }

    #[test]
    fn telnet_refuses_client_side_requests_and_repeats_nothing() {
        let reply = SharedBuf::default();
        // IAC DO 1 asked twice: one WONT, then silence.
        let input = [IAC, DO, 1, IAC, DO, 1, b'x'];
        assert_eq!(read_all(reader(&input, reply.clone())), b"x");
        assert_eq!(&*reply.0.lock().unwrap(), &[IAC, WONT, 1]);
    }

    #[test]
    fn telnet_passes_literal_iac_and_strips_subnegotiation() {
        let reply = SharedBuf::default();
        // Literal 0xFF, then a subnegotiation, then data.
        let input = [IAC, IAC, IAC, SB, 24, 0, b'x', IAC, SE, b'o', b'k'];
        assert_eq!(read_all(reader(&input, reply.clone())), [IAC, b'o', b'k']);
        assert!(reply.0.lock().unwrap().is_empty());
    }

    #[test]
    fn telnet_writer_doubles_iac() {
        let out = SharedBuf::default();
        let mut w = TelnetWriter::new(out.clone());
        w.write_all(&[IAC, b'a', IAC]).unwrap();
        assert_eq!(&*out.0.lock().unwrap(), &[IAC, IAC, b'a', IAC, IAC]);
    }
}

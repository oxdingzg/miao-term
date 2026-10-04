# ADR 0037 — Serial, Telnet and raw TCP sessions

> 中文: [`0037-serial-telnet-tcp.zh-CN.md`](0037-serial-telnet-tcp.zh-CN.md)

Status: accepted.

## Context

M6 (PuTTY-style remote) starts with R1: serial consoles (baud, data bits,
parity, stop bits, flow control), Telnet and raw TCP, saved in the host
library beside SSH. Today every pane is a shell on a PTY: `mtty-core`'s
`Terminal` spawns a `MasterPty` plus a child process and reads the master on a
thread (`crates/term-core/src/term.rs`), and the host library
(`mtty-config::hosts`) stores SSH targets only.

A serial console, a Telnet server and a raw TCP peer have no shell, no PTY and
no OSC 133 command boundaries — they are just a byte stream in and out. The
terminal core must therefore be able to consume and write a plain transport,
without pretending there is a process behind it.

## Decision

1. **The terminal core becomes transport-agnostic.** Add
   `Terminal::from_pipe(cols, rows, scrollback, reader, writer, waker)`; the
   core assumes only a `Read` whose bytes a background thread forwards into
   `rx`, and a `Box<dyn Write + Send>`. `Terminal::new` keeps the PTY path by
   building that same pipe from the master. `master` and `child` become
   options, so a session with no process tears down without killing anything.
   The PTY-only hooks (shell integration, the OSC 7 working directory, command
   capture) stay inert when there is no shell.
2. **Transport backends** live in `mtty-ui` (GPU-free, so unit-tested),
   one module each:
   - **raw TCP** — `std::net::TcpStream`, no protocol.
   - **Telnet** — a small in-tree codec over `TcpStream`. It answers IAC
     negotiation (WILL/WONT/DO/DONT, refusing everything but the charset and
     echo it already has), strips IAC sequences from the bytes the core sees,
     and doubles IAC on write. Hand-rolled rather than a crate: the option set
     is tiny and this keeps the dependency and licence surface at zero
     (ADR 0006).
   - **serial** — the `serialport` crate (MIT), configured from the saved
     profile (baud, data bits, parity, stop bits, software and hardware flow).
3. **Host library.** `hosts.toml`'s `Host` gains `kind` (`ssh` default,
   `serial`, `telnet`, `tcp`) plus the per-kind fields (an address and port, or
   a serial profile). Import from `~/.ssh/config` only ever creates `ssh`
   entries; files saved before this load unchanged (a missing `kind` is `ssh`).
4. **UI.** The host list, Open Quickly, `mtty://host/<name>` and the palette
   offer the new kinds; a serial session's dialog (device, baud, parity, data
   bits, stop bits, flow) is a small form prefilled from the profile. Telnet
   and raw TCP are unencrypted, so the session is marked as plaintext in the
   sidebar exactly as plain FTP is, and mtty stores no credentials for them. A
   serial device that cannot be opened reports the OS error in the pane.
5. **Lifecycle.** A transport that closes or errors ends the pane the way an
   exited shell does; reconnect re-opens the same saved profile, and a session
   is restored by reconnecting as SSH sessions are. No shell shim, OSC 7
   working directory or command capture is advertised for a non-PTY transport.

## Consequences

- One new dependency, `serialport` (MIT); Telnet and raw TCP add none.
- The terminal core no longer assumes a child process, which also makes the
  native-SSH proposal (R3) cheaper.
- A non-PTY pane has no working directory (the sidebar shows the transport
  instead) and no command boundaries; recipes or agents that need a shell
  refuse it explicitly.
- "Copy/send the last command's output" and the agent queue do not apply to
  these sessions, and the UI says so rather than failing silently.
- Windows serial ports work through `serialport`; the profile form lists the
  system's COM ports and Unix device nodes.

## Amendment — the serial crate

This ADR, and the dependency comment it led to, recorded `serialport` as **MIT**.
It is not. `serialport` 4.x declares **MPL-2.0** — a weak copyleft licence, and
one ADR 0006 excludes from the engine. The crate was chosen on a licence claim
that was never true, and nothing verified it: the claim was copied from the
first place it was written down into every later place.

The dependency is now **`serial2`**, a fork of the same project under
**BSD-2-Clause OR Apache-2.0**, both on ADR 0006's allow-list. It is used the
same way: a port is opened by path with the saved profile's settings, and the
reader is a `try_clone()` of the writer. The one behavioural difference is
enumeration — `serialport::available_ports()` returned records carrying a
`port_name`, while `serial2` returns paths — and the `/dev` scan fallback for
platforms that cannot enumerate is unchanged.

Nothing else in this decision changes. The two licence claims above are
corrected by this note rather than edited in place, so that what was decided,
and what was got wrong, both stay readable. The gate that found this is recorded
in [ADR 0006](0006-license-policy.md).

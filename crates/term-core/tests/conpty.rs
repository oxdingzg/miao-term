//! Windows ConPTY smoke test for `miao-term-core`.
//!
//! Spawns a real shell on a ConPTY through [`Terminal`], drives it, and checks the
//! output reaches the screen model. Also checks resize propagates to screen + PTY.
#![cfg(windows)]

use std::time::{Duration, Instant};

use miao_term_core::Terminal;

fn screen_text(term: &Terminal) -> String {
    let (rows, _cols) = term.screen().size();
    (0..rows)
        .map(|row| term.screen().line_text(row))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn conpty_spawns_shell_and_echoes() {
    let mut term = Terminal::new(Some("cmd.exe".to_string()), 100, 30, 2000, None, &[], std::sync::Arc::new(|| {}))
        .expect("openpty + spawn cmd.exe on ConPTY");

    // Let the shell come up; resend a few times in case input raced startup.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut text = String::new();
    let mut last_send = Instant::now() - Duration::from_secs(1);
    while Instant::now() < deadline {
        term.process_pending();
        text = screen_text(&term);
        if text.contains("CONPTY_OK") {
            break;
        }
        if last_send.elapsed() >= Duration::from_millis(800) {
            term.write(b"echo CONPTY_OK\r\n");
            last_send = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Make the shell exit so the PTY closes and nothing is left running.
    term.write(b"exit\r\n");

    assert!(
        text.contains("CONPTY_OK"),
        "expected echo output; screen was:\n{text}"
    );
}

#[test]
fn conpty_resize_updates_screen_and_pty() {
    let mut term = Terminal::new(Some("cmd.exe".to_string()), 80, 24, 1000, None, &[], std::sync::Arc::new(|| {}))
        .expect("openpty + spawn cmd.exe on ConPTY");

    term.process_pending();
    term.resize(40, 120);
    // Let the shell exit: on Windows `ClosePseudoConsole` blocks until the
    // client exits, so dropping a live shell would stall for minutes.
    term.write(b"exit\r\n");

    assert_eq!(
        term.size(),
        (40, 120),
        "Terminal::size should reflect the resize"
    );
    assert_eq!(
        term.screen().size(),
        (40, 120),
        "screen model should be resized"
    );
}

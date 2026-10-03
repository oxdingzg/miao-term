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
    let mut term = Terminal::new(
        Some("cmd.exe".to_string()),
        100,
        30,
        2000,
        None,
        &[],
        std::sync::Arc::new(|| {}),
    )
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
    let mut term = Terminal::new(
        Some("cmd.exe".to_string()),
        80,
        24,
        1000,
        None,
        &[],
        std::sync::Arc::new(|| {}),
    )
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

#[test]
fn conpty_uses_current_user_environment_and_explicit_overrides() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .unwrap();
    let name = format!("MTTY_ENV_QA_{}", std::process::id());
    struct Cleanup {
        key: RegKey,
        name: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.key.delete_value(&self.name);
            std::env::remove_var(&self.name);
        }
    }
    let cleanup = Cleanup { key, name };
    std::env::set_var(&cleanup.name, "HOST_SNAPSHOT_OLD");
    cleanup
        .key
        .set_value(&cleanup.name, &"USER_ENV_CURRENT")
        .unwrap();
    for (extra, expected) in [
        (vec![], "USER_ENV_CURRENT"),
        (
            vec![(cleanup.name.clone(), "PANE_OVERRIDE".into())],
            "PANE_OVERRIDE",
        ),
    ] {
        let mut term = Terminal::new(
            Some("cmd.exe".into()),
            100,
            30,
            2000,
            None,
            &extra,
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut sent = false;
        let mut text = String::new();
        while Instant::now() < deadline {
            term.process_pending();
            text = screen_text(&term);
            if text.contains(expected) {
                break;
            }
            if !sent && text.contains('>') {
                term.write(format!("echo %{}%\r", cleanup.name).as_bytes());
                sent = true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        term.write(b"exit\r");
        assert!(
            text.contains(expected),
            "fresh environment not inherited: {text}"
        );
        assert!(!text.contains("HOST_SNAPSHOT_OLD"));
    }
}

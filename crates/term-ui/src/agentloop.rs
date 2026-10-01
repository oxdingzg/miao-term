//! Agent-loop side effects: system notifications and sleep prevention
//! (ADR 0010). Both shell out to platform tools and degrade to no-ops where a
//! tool is missing, so nothing here can fail the app.

use std::process::{Child, Command};

/// Post a system notification. Best-effort: failures are ignored.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape(body),
            escape(title)
        );
        let _ = Command::new("osascript").arg("-e").arg(script).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send").arg(title).arg(body).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // Best-effort; requires the BurntToast module, which may be absent.
        let script = format!(
            "New-BurntToastNotification -Text '{}','{}'",
            escape_ps(title),
            escape_ps(body)
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (title, body);
    }
}

/// Escape a string for an AppleScript double-quoted literal.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn escape_ps(s: &str) -> String {
    s.replace('\'', "''")
}

/// Keeps the machine awake while an agent is processing. The child process is
/// platform-specific and is killed when no longer needed or on drop.
#[derive(Default)]
pub struct SleepGuard {
    child: Option<Child>,
}

impl SleepGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Idempotently start or stop the inhibitor.
    pub fn set_awake(&mut self, awake: bool) {
        match (awake, self.child.is_some()) {
            (true, false) => self.child = spawn_inhibitor(),
            (false, true) => {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            _ => {}
        }
    }

    pub fn awake(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for SleepGuard {
    fn drop(&mut self) {
        self.set_awake(false);
    }
}

// Both inhibitors also watch our pid, so they end with miaotty even when it
// exits without dropping the guard (process::exit, a crash, a kill).
#[cfg(target_os = "macos")]
fn spawn_inhibitor() -> Option<Child> {
    let pid = std::process::id().to_string();
    Command::new("caffeinate")
        .args(["-dims", "-w", &pid])
        .spawn()
        .ok()
}

#[cfg(target_os = "linux")]
fn spawn_inhibitor() -> Option<Child> {
    Command::new("systemd-inhibit")
        .args([
            "--what=idle:sleep",
            "--why=miaotty: agent processing",
            "--mode=block",
            "tail",
            &format!("--pid={}", std::process::id()),
            "-f",
            "/dev/null",
        ])
        .spawn()
        .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn spawn_inhibitor() -> Option<Child> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_applescript_strings() {
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn sleep_guard_is_idempotent() {
        let mut guard = SleepGuard::new();
        guard.set_awake(false);
        assert!(!guard.awake());
        guard.set_awake(false);
        assert!(!guard.awake());
    }
}

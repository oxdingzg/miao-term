//! Native system integration shared by the mtty hosts (ADR 0035).
//!
//! Notifications are native where the platform allows it — on macOS the banner
//! goes through `UserNotifications`, so it belongs to mtty and can be clicked,
//! instead of being attributed to Script Editor like `osascript` — and fall
//! back to a command-line tool elsewhere. Putting the platform-specific
//! dependencies here keeps `term-ui` host-agnostic. Clipboard image access
//! will live here too; see ADR 0035.

use std::process::Command;

#[cfg(target_os = "macos")]
mod macos;

/// Post a system notification. Best-effort: failures are ignored.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        if macos::notify(title, body) {
            return;
        }
        // Not running from an app bundle: there is no bundle identifier for
        // the notification center, so keep the AppleScript path (ADR 0035).
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

/// Show a blocking error dialog, for failures before any window can draw
/// (an app started from Finder or a launcher has no visible stderr).
/// Best-effort: without a dialog tool it does nothing.
pub fn alert(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display alert \"{}\" message \"{}\" as critical",
            escape(title),
            escape(body)
        );
        let _ = Command::new("osascript").arg("-e").arg(script).status();
    }
    #[cfg(target_os = "linux")]
    {
        let text = format!("{title}\n\n{body}");
        let shown = Command::new("zenity")
            .args(["--error", "--no-markup", "--text", &text])
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            let _ = Command::new("kdialog").args(["--error", &text]).status();
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; \
             [System.Windows.MessageBox]::Show('{}', '{}', 'OK', 'Error') | Out-Null",
            escape_ps(body),
            escape_ps(title)
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (title, body);
    }
}

/// The clipboard's image as PNG bytes when it holds one, or `None` on platforms
/// without a backend yet (ADR 0036). Callers fall back to text or the
/// application's own clipboard reader.
pub fn clipboard_image() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos::clipboard_image()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_applescript_strings() {
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    }
}

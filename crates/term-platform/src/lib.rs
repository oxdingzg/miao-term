//! Native system integration shared by the mtty hosts (ADR 0035).
//!
//! Notifications are native where the platform allows it — on macOS the banner
//! goes through `UserNotifications`, so it belongs to mtty and can be clicked,
//! instead of being attributed to Script Editor like `osascript` — and fall
//! back to a command-line tool elsewhere. Putting the platform-specific
//! dependencies here keeps `term-ui` host-agnostic. Clipboard image access
//! will live here too; see ADR 0035.

use std::process::Command;

/// Build a background command without creating a Windows console window.
/// Output can still be captured with `Command::output`; GUI tools retain
/// their own windows. Use ordinary commands for interactive terminal shells.
pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    };
    command
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod winrt;

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
        if linux::notify(title, body) {
            return;
        }
        let _ = Command::new("notify-send")
            .arg("-a")
            .arg("mtty")
            .arg(title)
            .arg(body)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        if winrt::notify(title, body) {
            return;
        }
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
    #[cfg(windows)]
    fn background_process_has_no_console_and_captures_output() {
        // Query the actual child process, rather than just checking its flags.
        let script = r#"
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class ConsoleProbe { [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); }'
[Console]::Out.WriteLine([ConsoleProbe]::GetConsoleWindow().ToInt64())
[Console]::Error.WriteLine('probe stderr')
exit 7
"#;
        let out = background_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "probe stderr");
    }

    #[test]
    fn escapes_applescript_strings() {
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    }
}

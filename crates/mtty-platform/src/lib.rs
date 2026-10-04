//! Native system integration shared by the mtty hosts (ADR 0035).
//!
//! Notifications are native where the platform allows it — on macOS the banner
//! goes through `UserNotifications`, so it belongs to mtty and can be clicked,
//! instead of being attributed to Script Editor like `osascript` — and fall
//! back to a command-line tool elsewhere. Putting the platform-specific
//! dependencies here keeps `mtty-ui` host-agnostic. Clipboard image access
//! will live here too; see ADR 0035.

use std::process::Command;

/// Build a background command without creating a Windows console window.
/// Output can still be captured with `Command::output`; GUI tools retain
/// their own windows. Use ordinary commands for interactive terminal shells.
pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(windows)]
    let program = find_executable(program.as_ref())
        .unwrap_or_else(|| std::path::PathBuf::from(program.as_ref()));
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

/// Locate a tool, including Windows executable and batch-file extensions.
pub fn find_executable(program: impl AsRef<std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let extensions: Vec<std::ffi::OsString> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|ext| ext.starts_with('.') && !ext.contains(['/', '\\']))
            .map(std::ffi::OsString::from)
            .collect()
    } else {
        Vec::new()
    };
    find_program(
        program.as_ref(),
        std::env::var_os("PATH").as_deref(),
        &extensions,
    )
}

fn find_program(
    program: &std::ffi::OsStr,
    path: Option<&std::ffi::OsStr>,
    extensions: &[std::ffi::OsString],
) -> Option<std::path::PathBuf> {
    let program_path = std::path::Path::new(program);
    let dirs = if program_path.is_absolute() || program_path.components().count() > 1 {
        vec![std::path::PathBuf::new()]
    } else {
        std::env::split_paths(path?).collect()
    };
    for dir in dirs {
        let candidate = dir.join(program_path);
        if candidate.is_file() {
            return Some(candidate);
        }
        for ext in extensions {
            let mut name = candidate.as_os_str().to_os_string();
            name.push(ext);
            let candidate = std::path::PathBuf::from(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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
        let _ = background_command("powershell")
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
        let _ = background_command("powershell")
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
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        // Windows uses the native clipboard; Linux uses X11. Wayland images
        // are read by the host's existing data device on its own connection.
        let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
        rgba_png(image.width, image.height, &image.bytes)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn rgba_png(width: usize, height: usize, bytes: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    if width == 0
        || height == 0
        || bytes.len()
            != (width as usize)
                .checked_mul(height as usize)?
                .checked_mul(4)?
    {
        return None;
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(bytes, width, height, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(png)
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
    fn finds_native_and_batch_tools_with_windows_extensions() {
        let root = std::env::temp_dir().join(format!("mtty-path-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let extensions = [".exe".into(), ".cmd".into()];
        let path = std::env::join_paths([&root]).unwrap();
        for name in ["claude.exe", "miao.cmd"] {
            std::fs::write(root.join(name), "test").unwrap();
        }
        for (program, expected) in [("claude", "claude.exe"), ("miao", "miao.cmd")] {
            assert_eq!(
                find_program(program.as_ref(), Some(&path), &extensions),
                Some(root.join(expected))
            );
            assert_eq!(
                find_program(root.join(program).as_os_str(), None, &extensions),
                Some(root.join(expected))
            );
        }
        assert!(find_program("missing".as_ref(), Some(&path), &extensions).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn background_batch_file_runs_with_spaces_and_captures_errors() {
        let root = std::env::temp_dir().join(format!("mtty batch test {}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("probe.cmd"),
            "@echo off\r\necho batch output\r\necho batch error 1>&2\r\nexit /b 9\r\n",
        )
        .unwrap();
        let out = background_command(root.join("probe")).output().unwrap();
        assert_eq!(out.status.code(), Some(9));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "batch output");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "batch error");
        std::fs::remove_dir_all(root).unwrap();
    }

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

    #[test]
    fn clipboard_rgba_encodes_exact_pixels_and_rejects_invalid_dimensions() {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128];
        let png = rgba_png(2, 1, &pixels).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.as_raw(), &pixels);
        assert!(rgba_png(0, 1, &[]).is_none());
        assert!(rgba_png(2, 1, &pixels[..4]).is_none());
        assert!(rgba_png(usize::MAX, 1, &pixels).is_none());
    }
}

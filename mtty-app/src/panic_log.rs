//! Durable panic records.
//!
//! The macOS app is started by launchd, which discards stderr, so the default
//! panic message (and the reason for an exit 101) is otherwise lost. The hook
//! appends a record to a per-user log file and then runs the previous hook, so
//! stderr output and the default behavior stay unchanged.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Install the hook writing to the platform log location (if one resolves).
pub fn install() {
    if let Some(path) = default_path() {
        install_at(path);
    }
}

/// Install the hook writing to `path`; chains to the previously set hook.
pub fn install_at(path: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let thread = std::thread::current();
        let thread = format!(
            "{} ({:?})",
            thread.name().unwrap_or("<unnamed>"),
            thread.id()
        );
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let record = format_record(
            unix,
            env!("CARGO_PKG_VERSION"),
            &thread,
            message,
            &location,
            &backtrace,
        );
        // A failed write must never panic inside the hook (that aborts).
        let _ = append(&path, &record);
        previous(info);
    }));
}

/// `~/Library/Logs/mtty/panic.log` on macOS, `$XDG_STATE_HOME/mtty/panic.log`
/// (or `~/.local/state/mtty/panic.log`) on Linux and other Unix systems,
/// `%LOCALAPPDATA%\mtty\logs\panic.log` on Windows.
pub fn default_path() -> Option<PathBuf> {
    let env_dir = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let dir = if cfg!(windows) {
        env_dir("LOCALAPPDATA")?.join("mtty").join("logs")
    } else if cfg!(target_os = "macos") {
        env_dir("HOME")?.join("Library/Logs/mtty")
    } else {
        env_dir("XDG_STATE_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".local/state")))?
            .join("mtty")
    };
    Some(dir.join("panic.log"))
}

/// One panic record as appended to the log.
pub fn format_record(
    unix_secs: u64,
    version: &str,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> String {
    format!(
        "=== mtty panic ===\n\
         time: {} UTC (unix {unix_secs})\n\
         version: {version}\n\
         thread: {thread}\n\
         location: {location}\n\
         message: {message}\n\
         backtrace:\n{}\n\n",
        utc_string(unix_secs),
        backtrace.trim_end(),
    )
}

/// Append `record`, creating the parent directory when missing.
pub fn append(path: &Path, record: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(record.as_bytes())
}

/// `YYYY-MM-DD HH:MM:SS` for a unix timestamp (Howard Hinnant's civil_from_days).
fn utc_string(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "mtty-panic-log-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn utc_string_matches_known_instants() {
        assert_eq!(utc_string(0), "1970-01-01 00:00:00");
        assert_eq!(utc_string(951_782_400), "2000-02-29 00:00:00");
        // 2026-10-02 07:22:51 +08:00
        assert_eq!(utc_string(1_790_896_971), "2026-10-01 23:22:51");
    }

    #[test]
    fn format_record_contains_every_field() {
        let record = format_record(
            1_790_896_971,
            "9.9.9",
            "main (ThreadId(1))",
            "boom: reentrant",
            "src/event_handler.rs:135:9",
            "   0: frame_a\n   1: frame_b\n",
        );
        for needle in [
            "2026-10-01 23:22:51 UTC",
            "unix 1790896971",
            "version: 9.9.9",
            "thread: main (ThreadId(1))",
            "location: src/event_handler.rs:135:9",
            "message: boom: reentrant",
            "   0: frame_a\n   1: frame_b",
        ] {
            assert!(record.contains(needle), "missing {needle:?} in {record}");
        }
    }

    #[test]
    fn append_into_unwritable_location_returns_error_without_panicking() {
        let dir = scratch_dir("unwritable");
        // A regular file where a directory is expected: create_dir_all fails
        // regardless of the user's privileges.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("sub").join("panic.log");
        let outcome = std::panic::catch_unwind(|| append(&path, "record"));
        assert!(matches!(outcome, Ok(Err(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installed_hook_appends_a_record_and_keeps_unwinding() {
        let dir = scratch_dir("hook");
        let path = dir.join("nested").join("panic.log");
        install_at(path.clone());
        let caught = std::panic::catch_unwind(|| {
            std::thread::Builder::new()
                .name("panic-log-probe".into())
                .spawn(|| panic!("probe panic {}", 42))
                .unwrap()
                .join()
        });
        // Restore the default hook so no other test writes into this file.
        let _ = std::panic::take_hook();
        assert!(
            matches!(caught, Ok(Err(_))),
            "panic should stay in the thread"
        );
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("message: probe panic 42"), "{log}");
        assert!(log.contains("thread: panic-log-probe"), "{log}");
        assert!(log.contains("panic_log.rs:"), "{log}");
        assert!(log.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))));
        assert!(log.contains("backtrace:\n"), "{log}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

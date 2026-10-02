//! Finding a server's program. An app started from the Dock or a desktop
//! launcher has a short `PATH` without `~/.cargo/bin`, nvm's Node or
//! Homebrew; the user's login shell knows them, so its `PATH` is asked for
//! once (on Unix) and put first.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The `PATH` servers are looked up in and started with.
pub fn search_path() -> Option<OsString> {
    static PATH: OnceLock<Option<OsString>> = OnceLock::new();
    PATH.get_or_init(|| {
        let current = std::env::var_os("PATH");
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(shell) = login_shell_path() {
            dirs.extend(std::env::split_paths(&shell));
        }
        if let Some(cur) = &current {
            dirs.extend(std::env::split_paths(cur));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for d in [".cargo/bin", ".local/bin", "go/bin", ".bun/bin"] {
                dirs.push(home.join(d));
            }
        }
        if cfg!(target_os = "macos") {
            dirs.push("/opt/homebrew/bin".into());
            dirs.push("/usr/local/bin".into());
        }
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
        std::env::join_paths(dirs).ok().or(current)
    })
    .clone()
}

/// The login shell's `PATH` (interactive, so nvm and the like have run),
/// or `None` after 5 s or on Windows.
fn login_shell_path() -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    const MARK: &str = "__MTTY_PATH__";
    let mut child = std::process::Command::new(shell)
        .args([
            "-l",
            "-i",
            "-c",
            &format!("printf '{MARK}%s{MARK}' \"$PATH\""),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let start = out.find(MARK)? + MARK.len();
    let end = start + out[start..].find(MARK)?;
    let path = &out[start..end];
    (!path.is_empty()).then(|| OsString::from(path))
}

/// `program` as found on `path` (an explicit path is taken as it is).
pub fn which(program: &str, path: &Option<OsString>) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(path.as_ref()?).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{program}{e}")))
            .find(|c| c.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_programs_on_a_path() {
        let dir = std::env::temp_dir().join(format!("mtty-lsp-which-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) { "srv.exe" } else { "srv" };
        std::fs::write(dir.join(name), "").unwrap();
        let path = std::env::join_paths([dir.clone()]).ok();
        assert_eq!(which("srv", &path), Some(dir.join(name)));
        assert_eq!(which("missing", &path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

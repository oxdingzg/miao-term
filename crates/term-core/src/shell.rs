//! Shell integration: a `ZDOTDIR` shim so the spawned shell reports its cwd
//! (OSC 7) and command history (via `miaotty-cli`) without the user editing any
//! dotfiles.
//!
//! The shim restores the user's real `ZDOTDIR` first, then sources their
//! `~/.zshenv`, so their setup is untouched — same approach Ghostty uses.

use std::io;
use std::path::{Path, PathBuf};

const ZSHENV: &str = r#"# miaotty shell integration (zsh) — auto-generated, do not edit.
if [[ -n "${MIAOTTY_ZDOTDIR_ORIG+x}" ]]; then
  export ZDOTDIR="$MIAOTTY_ZDOTDIR_ORIG"
  unset MIAOTTY_ZDOTDIR_ORIG
else
  unset ZDOTDIR
fi
[[ -r "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"

_miaotty_osc7() { printf '\033]7;file://%s%s\033\\' "${HOST:-localhost}" "$PWD"; }
_miaotty_preexec() {
  command -v miaotty-cli >/dev/null 2>&1 || return 0
  ( miaotty-cli history:add --command "$1" --cwd "$PWD" >/dev/null 2>&1 & )
}
autoload -Uz add-zsh-hook 2>/dev/null
if (( $+functions[add-zsh-hook] )); then
  add-zsh-hook chpwd _miaotty_osc7
  add-zsh-hook precmd _miaotty_osc7
  add-zsh-hook preexec _miaotty_preexec
fi
_miaotty_osc7
"#;

/// Per-user directory the shim is written to. Prefers `$XDG_RUNTIME_DIR` and
/// otherwise `$TMPDIR`, falling back to the system temp dir.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return path;
        }
    }
    std::env::temp_dir()
}

/// Create (or validate) a directory only the current user can access.
///
/// The shim is `source`d by every new shell, so a tamperable directory would
/// mean arbitrary code execution. We refuse anything that is not a real
/// directory or that grants group/other permissions.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match dir.symlink_metadata() {
        Ok(md) => {
            if !md.is_dir() || md.file_type().is_symlink() {
                return Err(io::Error::other("shim path is not a directory"));
            }
            if md.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::other("shim directory is group/other accessible"));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(dir)?;
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn shim_dir() -> Option<PathBuf> {
    let dir = runtime_dir().join("miaotty-zdotdir");
    ensure_private_dir(&dir).ok()?;
    Some(dir)
}

/// Extra environment for `shell` (empty unless it is zsh).
pub fn env_for(shell: &str) -> Vec<(String, String)> {
    let base = std::path::Path::new(shell)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if base != "zsh" {
        return Vec::new();
    }

    let Some(dir) = shim_dir() else {
        return Vec::new();
    };
    if std::fs::write(dir.join(".zshenv"), ZSHENV).is_err() {
        return Vec::new();
    }

    let orig = std::env::var("ZDOTDIR")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());

    vec![
        ("ZDOTDIR".to_string(), dir.to_string_lossy().into_owned()),
        ("MIAOTTY_ZDOTDIR_ORIG".to_string(), orig),
    ]
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_path() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "miaotty-shim-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn creates_a_private_dir() {
        let base = temp_path();
        let dir = base.join("nested/shim");
        ensure_private_dir(&dir).unwrap();
        let mode = dir.symlink_metadata().unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "shim dir must not be group/other accessible"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_a_world_accessible_dir() {
        let dir = temp_path();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_private_dir(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Shell integration: a `ZDOTDIR` shim so the spawned shell reports its cwd
//! (OSC 7) and command history (via `miaotty-cli`) without the user editing any
//! dotfiles.
//!
//! The shim restores the user's real `ZDOTDIR` first, then sources their
//! `~/.zshenv`, so their setup is untouched — same approach Ghostty uses.

use std::path::PathBuf;

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

fn shim_dir() -> PathBuf {
    std::env::temp_dir().join("miaotty-zdotdir")
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

    let dir = shim_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
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

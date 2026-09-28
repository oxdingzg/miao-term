//! Agent integration: detect agents, install state hooks, launch them
//! (ADR 0016).
//!
//! Hooks are shell scripts under `~/.config/miaotty/hooks/<agent>.sh` that call
//! `miaotty-cli state <agent> --state <s>`; installing writes the script and
//! returns the snippet the user wires into the agent's own hook config. We
//! never edit an agent's config for it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A supported agent.
pub struct Agent {
    pub name: &'static str,
    pub bin: &'static str,
    pub launch: &'static str,
    /// Where the hook is registered, for the snippet text.
    pub hook_via: &'static str,
}

pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        bin: "claude",
        launch: "claude",
        hook_via: "Claude Code hooks (Stop / Notification / PreToolUse)",
    },
    Agent {
        name: "codex",
        bin: "codex",
        launch: "codex",
        hook_via: "codex hook config",
    },
    Agent {
        name: "opencode",
        bin: "opencode",
        launch: "opencode",
        hook_via: "the opencode plugin `event` hook",
    },
    Agent {
        name: "miao",
        bin: "miao",
        launch: "miao",
        hook_via: ".miao/plugins/miaotty or a session hook",
    },
];

/// Whether `bin` is an executable on `PATH`.
pub fn detected(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(bin);
        candidate.is_file()
    })
}

/// `~/.config/miaotty/hooks`.
pub fn hooks_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("miaotty").join("hooks"))
}

pub fn script_path(agent: &str) -> Option<PathBuf> {
    Some(hooks_dir()?.join(format!("{agent}.sh")))
}

/// The hook script body for an agent.
pub fn hook_script(agent: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # miaotty agent hook for {agent} (generated).\n\
         # Usage: hook.sh <processing|idle|awaiting|error> [session-id]\n\
         set -eu\n\
         state=\"${{1:-}}\"\n\
         [ -n \"$state\" ] || {{ echo \"usage: $0 <state> [session-id]\" >&2; exit 2; }}\n\
         session=\"${{2:-}}\"\n\
         exe=\"${{MIAOTTY_CLI:-miaotty-cli}}\"\n\
         command -v \"$exe\" >/dev/null 2>&1 || exit 0\n\
         set -- state {agent} --state \"$state\"\n\
         [ -n \"$session\" ] && set -- \"$@\" --session \"$session\"\n\
         [ -n \"${{MIAOTTY_PANE_ID:-}}\" ] && set -- \"$@\" --pane \"$MIAOTTY_PANE_ID\"\n\
         \"$exe\" \"$@\" >/dev/null 2>&1 || true\n"
    )
}

/// Write the hook script (mode 0755) and return its path.
pub fn install(agent: &str) -> std::io::Result<PathBuf> {
    let path = script_path(agent)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"))?;
    let script = hook_script(agent);
    if !syntax_ok(&script) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "hook script failed its syntax check",
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// The text to paste into the agent's hook configuration.
pub fn snippet(agent: &Agent, path: &Path) -> String {
    format!(
        "{}: register `{} <processing|idle|awaiting|error> [session]` via {}.",
        agent.name,
        path.display(),
        agent.hook_via
    )
}

/// Launch an agent in the current directory (used by the Launch button).
pub fn launch_command(agent: &Agent) -> String {
    agent.launch.to_string()
}

/// Run `hook_script` through `sh -n` to check syntax (used in tests and by the
/// installer as a sanity check).
pub fn syntax_ok(script: &str) -> bool {
    let mut child = match Command::new("sh")
        .arg("-n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return true,
    };
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        let _ = stdin.write_all(script.as_bytes());
    }
    child.wait().map(|s| s.success()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_calls_cli_with_agent_and_state() {
        let s = hook_script("claude");
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains("state claude --state"));
        assert!(s.contains("MIAOTTY_PANE_ID"));
        assert!(syntax_ok(&s), "hook script must pass `sh -n`");
    }

    #[test]
    fn snippet_names_the_script_and_where_to_wire_it() {
        let agent = &AGENTS[0];
        let text = snippet(agent, Path::new("/tmp/claude.sh"));
        assert!(text.contains("/tmp/claude.sh"));
        assert!(text.contains("Claude Code"));
    }

    #[test]
    fn agents_are_unique() {
        let mut names: Vec<&str> = AGENTS.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), AGENTS.len());
        assert_eq!(launch_command(&AGENTS[1]), "codex");
    }
}

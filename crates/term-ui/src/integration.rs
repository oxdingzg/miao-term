//! Agent integration: detect agents, install state hooks, launch them
//! (ADR 0016).
//!
//! Hooks are shell scripts under `~/.config/mtty/hooks/<agent>.sh` that call
//! `mtty-cli state <agent> --state <s>`; installing writes the script and
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
    /// The agent reports its own state from inside mtty; no hook wiring is
    /// required of the user.
    pub auto: bool,
}

pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        bin: "claude",
        launch: "claude",
        hook_via: "Claude Code hooks (Stop / Notification / PreToolUse)",
        auto: false,
    },
    Agent {
        name: "codex",
        bin: "codex",
        launch: "codex",
        hook_via: "codex hook config",
        auto: false,
    },
    Agent {
        name: "opencode",
        bin: "opencode",
        launch: "opencode",
        hook_via: "the opencode plugin `event` hook",
        auto: false,
    },
    Agent {
        name: "miao",
        bin: "miao",
        launch: "miao",
        hook_via: "the built-in miao integration",
        auto: true,
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

/// `~/.config/mtty/hooks`.
pub fn hooks_dir() -> Option<PathBuf> {
    Some(miao_term_config::config_dir()?.join("hooks"))
}

pub fn script_path(agent: &str) -> Option<PathBuf> {
    Some(hooks_dir()?.join(format!("{agent}.sh")))
}

/// The hook script body for an agent.
pub fn hook_script(agent: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # mtty agent hook for {agent} (generated).\n\
         # Usage: hook.sh <processing|idle|awaiting|error> [session-id | --stdin]\n\
         # --stdin reads the session id from the JSON event on stdin.\n\
         set -eu\n\
         state=\"${{1:-}}\"\n\
         [ -n \"$state\" ] || {{ echo \"usage: $0 <state> [session-id]\" >&2; exit 2; }}\n\
         session=\"${{2:-}}\"\n\
         pane=\"${{MTTY_PANE_ID:-${{MIAOTTY_PANE_ID:-}}}}\"\n\
         # Hooks are global; only report for agents running inside an mtty pane.\n\
         [ -n \"$pane\" ] || exit 0\n\
         # Claude Code and codex pass the event as JSON on stdin.\n\
         if [ \"$session\" = --stdin ]; then\n\
         \x20 session=$(sed -n 's/.*\"session_id\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p' | head -n 1)\n\
         fi\n\
         exe=\"${{MTTY_CLI:-${{MIAOTTY_CLI:-mtty-cli}}}}\"\n\
         command -v \"$exe\" >/dev/null 2>&1 || exit 0\n\
         set -- state {agent} --state \"$state\"\n\
         [ -n \"$session\" ] && set -- \"$@\" --session \"$session\"\n\
         [ -n \"$pane\" ] && set -- \"$@\" --pane \"$pane\"\n\
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

/// The text to paste into the agent's hook configuration: where it goes and
/// a ready-to-merge configuration that calls the installed script.
pub fn snippet(agent: &Agent, path: &Path) -> String {
    if agent.auto {
        return format!(
            "{}: reports its state automatically when launched inside mtty; no hook wiring needed.",
            agent.name
        );
    }
    let script = crate::ssh::shell_quote(&path.display().to_string());
    match agent.name {
        "claude" => format!(
            "Merge into \"hooks\" in ~/.claude/settings.json:\n\n{}",
            hooks_json(
                &script,
                &[
                    ("SessionStart", "idle"),
                    ("UserPromptSubmit", "processing"),
                    ("Notification", "awaiting"),
                    ("Stop", "idle"),
                ]
            )
        ),
        "codex" => format!(
            "Merge into \"hooks\" in ~/.codex/hooks.json, and enable them in \
             ~/.codex/config.toml with `[features]` `hooks = true`. codex asks you \
             to trust the new hooks once.\n\n{}",
            hooks_json(
                &script,
                &[
                    ("SessionStart", "idle"),
                    ("UserPromptSubmit", "processing"),
                    ("PermissionRequest", "awaiting"),
                    ("Stop", "idle"),
                ]
            )
        ),
        "opencode" => format!(
            "Save as ~/.config/opencode/plugin/mtty.js:\n\n\
             export const MttyPlugin = async ({{ $ }}) => {{\n\
             \x20 const report = (state) => $`{} ${{state}}`.quiet().nothrow()\n\
             \x20 return {{\n\
             \x20   event: async ({{ event }}) => {{\n\
             \x20     const t = event.type\n\
             \x20     if (t === \"session.idle\") await report(\"idle\")\n\
             \x20     else if (t === \"session.error\") await report(\"error\")\n\
             \x20     else if (t.startsWith(\"permission.\")) await report(\"awaiting\")\n\
             \x20     else if (t === \"session.status\") await report(\"processing\")\n\
             \x20   }},\n\
             \x20 }}\n\
             }}\n",
            path.display()
        ),
        _ => format!(
            "{}: register `{script} <processing|idle|awaiting|error> [session]` via {}.",
            agent.name, agent.hook_via
        ),
    }
}

/// A Claude Code / codex style `{"hooks": {Event: [...]}}` block that runs
/// `script <state> --stdin` for each event.
fn hooks_json(script: &str, events: &[(&str, &str)]) -> String {
    let mut hooks = serde_json::Map::new();
    for (event, state) in events {
        hooks.insert(
            (*event).to_string(),
            serde_json::json!([{ "hooks": [{
                "type": "command",
                "command": format!("{script} {state} --stdin"),
            }]}]),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks })).unwrap_or_default()
}

/// Tools that can bind a system-wide hotkey to `mtty --quick`, for
/// platforms (Linux) where the app has no built-in global grab.
pub const HOTKEY_TOOLS: &[&str] = &[
    "skhd",
    "hammerspoon",
    "autohotkey",
    "gnome",
    "sway",
    "hyprland",
];

/// A ready-to-paste binding for the given tool.
pub fn hotkey_snippet(tool: &str) -> String {
    match tool {
        "skhd" => "# ~/.skhdrc\ncmd + shift - t : mtty --quick".to_string(),
        "hammerspoon" => {
            "hs.hotkey.bind({\"cmd\",\"shift\"}, \"T\", function()\n  hs.execute(\"mtty --quick\")\nend)"
                .to_string()
        }
        "autohotkey" => "; AutoHotkey v2\n#+t::Run \"mtty --quick\"".to_string(),
        "gnome" => "# GNOME: Settings → Keyboard → Custom Shortcuts\n\
                    # Name: Quick Terminal    Command: mtty --quick"
            .to_string(),
        // Wayland compositors: bind the app's own intent, no global grab needed.
        "sway" => "# ~/.config/sway/config\nbindsym $mod+Shift+t exec mtty --quick".to_string(),
        "hyprland" => {
            "# ~/.config/hypr/hyprland.conf\nbind = SUPER SHIFT, T, exec, mtty --quick"
                .to_string()
        }
        _ => String::new(),
    }
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
        assert!(s.contains("MTTY_PANE_ID") && s.contains("MIAOTTY_PANE_ID"));
        assert!(syntax_ok(&s), "hook script must pass `sh -n`");
    }

    #[test]
    fn claude_and_codex_snippets_are_mergeable_hook_configs() {
        for (name, file, awaiting) in [
            ("claude", "~/.claude/settings.json", "Notification"),
            ("codex", "~/.codex/hooks.json", "PermissionRequest"),
        ] {
            let agent = AGENTS.iter().find(|a| a.name == name).unwrap();
            let text = snippet(agent, Path::new("/tmp/hook dir/x.sh"));
            assert!(text.contains(file), "{text}");
            let json: serde_json::Value =
                serde_json::from_str(&text[text.find('{').unwrap()..]).unwrap();
            let command = |event: &str| {
                json["hooks"][event][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };
            assert_eq!(
                command("UserPromptSubmit"),
                "'/tmp/hook dir/x.sh' processing --stdin"
            );
            assert_eq!(command(awaiting), "'/tmp/hook dir/x.sh' awaiting --stdin");
            assert_eq!(command("Stop"), "'/tmp/hook dir/x.sh' idle --stdin");
        }
    }

    #[cfg(unix)]
    #[test]
    fn hook_script_reports_session_and_pane_and_never_waits() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mtty-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hook = dir.join("codex.sh");
        std::fs::write(&hook, hook_script("codex")).unwrap();
        let cli = dir.join("fake-cli");
        let log = dir.join("calls");
        std::fs::write(
            &cli,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        for p in [&hook, &cli] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let run = |args: &[&str], pane: Option<&str>, stdin: &str| {
            let mut cmd = Command::new(&hook);
            cmd.args(args)
                .env("MTTY_CLI", &cli)
                .env_remove("MTTY_PANE_ID")
                .env_remove("MIAOTTY_PANE_ID")
                .stdin(std::process::Stdio::piped());
            if let Some(pane) = pane {
                cmd.env("MTTY_PANE_ID", pane);
            }
            let mut child = cmd.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(stdin.as_bytes())
                .unwrap();
            assert!(child.wait().unwrap().success());
        };
        run(
            &["awaiting", "--stdin"],
            Some("pane3"),
            r#"{"session_id": "abc-123", "x": 1}"#,
        );
        // Outside an mtty pane nothing is reported.
        run(&["idle", "--stdin"], None, r#"{"session_id": "zzz"}"#);
        // An explicit session never reads stdin (which stays open here).
        let mut child = Command::new(&hook)
            .args(["processing", "s9"])
            .env("MTTY_CLI", &cli)
            .env("MTTY_PANE_ID", "pane3")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let _keep_stdin_open = child.stdin.take();
        assert!(child.wait().unwrap().success());
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            [
                "state codex --state awaiting --session abc-123 --pane pane3",
                "state codex --state processing --session s9 --pane pane3",
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hotkey_snippets_mention_quick() {
        for tool in HOTKEY_TOOLS {
            let snippet = hotkey_snippet(tool);
            assert!(snippet.contains("mtty --quick"), "{tool} snippet");
        }
        assert!(hotkey_snippet("nope").is_empty());
    }

    #[test]
    fn agents_are_unique() {
        let mut names: Vec<&str> = AGENTS.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), AGENTS.len());
        assert_eq!(launch_command(&AGENTS[1]), "codex");
    }

    #[test]
    fn miao_integrates_itself() {
        let miao = AGENTS.iter().find(|a| a.name == "miao").unwrap();
        assert!(miao.auto);
        let text = snippet(miao, Path::new("/tmp/miao.sh"));
        assert!(text.contains("no hook wiring needed"));
    }

    #[test]
    fn other_agents_need_wiring() {
        assert!(AGENTS.iter().filter(|a| !a.auto).count() >= 3);
    }
}

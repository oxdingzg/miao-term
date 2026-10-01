//! URL-scheme launching (ADR 0013).
//!
//! The OS invokes us with a URL argument when a registered scheme is opened
//! (`mtty://…` or the former `miaotty://…`, `ssh://…`, `x-man-page://…`). We translate that into a shell
//! command to run in a fresh tab; `mtty://` only activates the app.

/// What a launch asks the (running) app to do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Intent {
    /// Bring the app up / do nothing.
    #[default]
    Activate,
    /// Toggle the Quick Terminal.
    Quick,
    /// Focus a specific pane by id.
    Focus(String),
    /// Run a command in a new tab.
    Run(String),
}

impl Intent {
    /// Parse argv (after the program name).
    pub fn from_args(args: &[String]) -> Self {
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--quick" => return Intent::Quick,
                "--focus" => {
                    if let Some(id) = args.get(i + 1) {
                        return Intent::Focus(id.clone());
                    }
                }
                other => {
                    if let Some(intent) = Intent::from_url(other) {
                        return intent;
                    }
                }
            }
            i += 1;
        }
        Intent::Activate
    }

    /// The URL forms we understand (`mtty://quick`, `mtty://focus?pane=…`, the
    /// same under the former `miaotty://` scheme, plus `ssh://` /
    /// `x-man-page://` which become commands).
    pub fn from_url(arg: &str) -> Option<Self> {
        let (scheme, rest) = arg.split_once("://")?;
        match scheme.to_ascii_lowercase().as_str() {
            "mtty" | "miaotty" => {
                let rest = rest.trim_start_matches('/');
                if rest.starts_with("quick") {
                    Some(Intent::Quick)
                } else {
                    rest.strip_prefix("focus")
                        .and_then(|r| r.split("pane=").nth(1))
                        .map(|pane| Intent::Focus(pane.to_string()))
                }
            }
            _ => command_for(arg).map(Intent::Run),
        }
    }

    /// Encode for the cross-instance inbox.
    pub fn encode(&self) -> String {
        match self {
            Intent::Activate => String::new(),
            Intent::Quick => "quick".to_string(),
            Intent::Focus(id) => format!("focus\t{id}"),
            Intent::Run(cmd) => format!("run\t{cmd}"),
        }
    }

    /// Decode an inbox line (the inverse of [`Intent::encode`]).
    pub fn decode(line: &str) -> Self {
        match line.split_once('\t') {
            Some(("focus", id)) => Intent::Focus(id.to_string()),
            Some(("run", cmd)) => Intent::Run(cmd.to_string()),
            _ if line.trim() == "quick" => Intent::Quick,
            _ => Intent::Activate,
        }
    }
}

/// The command a launch URL maps to, or `None` when it should just activate.
pub fn command_for(arg: &str) -> Option<String> {
    let (scheme, rest) = arg.split_once("://")?;
    match scheme.to_ascii_lowercase().as_str() {
        "mtty" | "miaotty" => None,
        "ssh" => ssh_command(rest),
        "x-man-page" => {
            let cmd = rest.split(['/', '?']).next().filter(|s| !s.is_empty())?;
            Some(format!("man {}", crate::ssh::shell_quote(cmd)))
        }
        _ => None,
    }
}

/// The inbox directory for cross-instance launches.
pub fn inbox_dir() -> Option<std::path::PathBuf> {
    Some(miao_term_config::data_dir()?.join("inbox"))
}

/// Hand a launch to an already-running instance (single-instance deep link) and
/// wake it, so it drains the inbox promptly. Returns true when one was reached.
pub fn forward_to_running(request: &str) -> bool {
    let socket = miao_term_mtp::default_socket();
    let Ok(mut client) = miao_term_mtp::client::connect(&socket) else {
        return false;
    };
    if let Some(dir) = inbox_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let _ = std::fs::write(dir.join(format!("{stamp}.request")), request);
    }
    // Any request wakes the host (see `ServerState::wake`), which then drains.
    let _ = client.call("core", "health", serde_json::json!({}));
    true
}

/// Consume forwarding requests written by later launches. Each line is an
/// [`Intent::encode`] payload; an empty line means a bare activation.
pub fn drain_inbox() -> Vec<String> {
    let mut out = Vec::new();
    let Some(dir) = inbox_dir() else {
        return out;
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut paths: Vec<std::path::PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("request"))
        .collect();
    paths.sort();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.push(text.trim().to_string());
        }
        let _ = std::fs::remove_file(&path);
    }
    out
}

/// Parse `[user@]host[:port][/path][?query]` into an `ssh` invocation using
/// the SSH integration (connection reuse + remote bootstrap, ADR 0014).
fn ssh_command(rest: &str) -> Option<String> {
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let target = crate::ssh::Target::parse(authority)?;
    Some(crate::ssh::command(
        &target,
        &crate::ssh::bootstrap("xterm-256color"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_round_trips_and_parses_argv() {
        for intent in [
            Intent::Activate,
            Intent::Quick,
            Intent::Focus("pane_3".into()),
            Intent::Run("ssh 'host'".into()),
        ] {
            assert_eq!(Intent::decode(&intent.encode()), intent);
        }
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Intent::from_args(&args(&["--quick"])), Intent::Quick);
        assert_eq!(
            Intent::from_args(&args(&["--focus", "pane_9"])),
            Intent::Focus("pane_9".into())
        );
        assert_eq!(
            Intent::from_args(&args(&["miaotty://focus?pane=p1"])),
            Intent::Focus("p1".into())
        );
        assert_eq!(
            Intent::from_args(&args(&["miaotty://quick"])),
            Intent::Quick
        );
        assert_eq!(Intent::from_args(&args(&["mtty://quick"])), Intent::Quick);
        assert_eq!(
            Intent::from_args(&args(&["MTTY://focus?pane=p2"])),
            Intent::Focus("p2".into())
        );
        assert!(matches!(
            Intent::from_args(&args(&["ssh://h"])),
            Intent::Run(_)
        ));
        assert_eq!(Intent::from_args(&args(&[])), Intent::Activate);
    }

    #[test]
    fn ssh_variants() {
        let cmd = command_for("ssh://user@host:2222/some/path").unwrap();
        assert!(cmd.starts_with("ssh -t "));
        assert!(cmd.contains("ControlMaster=auto"));
        assert!(cmd.contains(" -p 2222 "));
        assert!(cmd.contains("'user@host'"));
        assert!(command_for("ssh://host").unwrap().contains("'host'"));
        assert!(command_for("ssh://[::1]:2200")
            .unwrap()
            .contains(" -p 2200 "));
        assert!(command_for("ssh://").is_none());
    }

    #[test]
    fn man_page_and_activation() {
        assert_eq!(command_for("x-man-page://ls").as_deref(), Some("man 'ls'"));
        assert_eq!(
            command_for("x-man-page://git/commit").as_deref(),
            Some("man 'git'")
        );
        assert!(command_for("miaotty://open").is_none());
        assert!(command_for("mtty://open").is_none());
        assert!(command_for("unknown://x").is_none());
        assert!(command_for("not-a-url").is_none());
    }

    #[test]
    fn quoting_is_safe() {
        assert_eq!(crate::ssh::shell_quote("a'b"), "'a'\\''b'");
    }
}

//! URL-scheme launching (ADR 0013).
//!
//! The OS invokes us with a URL argument when a registered scheme is opened
//! (`miaotty://…`, `ssh://…`, `x-man-page://…`). We translate that into a shell
//! command to run in a fresh tab; `miaotty://` only activates the app.

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

    /// The URL forms we understand (`miaotty://quick`, `miaotty://focus?pane=…`,
    /// plus `ssh://` / `x-man-page://` which become commands).
    pub fn from_url(arg: &str) -> Option<Self> {
        let (scheme, rest) = arg.split_once("://")?;
        match scheme.to_ascii_lowercase().as_str() {
            "miaotty" => {
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
        "miaotty" => None,
        "ssh" => ssh_command(rest),
        "x-man-page" => {
            let cmd = rest.split(['/', '?']).next().filter(|s| !s.is_empty())?;
            Some(format!("man {}", crate::ssh::shell_quote(cmd)))
        }
        _ => None,
    }
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
        assert!(command_for("unknown://x").is_none());
        assert!(command_for("not-a-url").is_none());
    }

    #[test]
    fn quoting_is_safe() {
        assert_eq!(crate::ssh::shell_quote("a'b"), "'a'\\''b'");
    }
}

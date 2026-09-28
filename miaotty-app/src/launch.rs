//! URL-scheme launching (ADR 0013).
//!
//! The OS invokes us with a URL argument when a registered scheme is opened
//! (`miaotty://…`, `ssh://…`, `x-man-page://…`). We translate that into a shell
//! command to run in a fresh tab; `miaotty://` only activates the app.

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

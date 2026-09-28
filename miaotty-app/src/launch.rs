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
            Some(format!("man {}", shell_quote(cmd)))
        }
        _ => None,
    }
}

/// Parse `[user@]host[:port][/path][?query]` into an `ssh` invocation.
fn ssh_command(rest: &str) -> Option<String> {
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    let (host, port) = split_host_port(authority);
    if host.is_empty() {
        return None;
    }
    let mut cmd = String::from("ssh");
    if let Some(port) = port {
        cmd.push_str(&format!(" -p {}", shell_quote(&port)));
    }
    cmd.push(' ');
    cmd.push_str(&shell_quote(&host));
    Some(cmd)
}

/// Split a URL authority into host and optional port, handling `[ipv6]:port`.
fn split_host_port(authority: &str) -> (String, Option<String>) {
    if let Some(rest) = authority.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail.strip_prefix(':').filter(|p| !p.is_empty());
            return (format!("[{host}]"), port.map(str::to_string));
        }
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return (host.to_string(), Some(port.to_string()));
        }
    }
    (authority.to_string(), None)
}

/// Single-quote a string for the shell.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_variants() {
        assert_eq!(command_for("ssh://host").as_deref(), Some("ssh 'host'"));
        assert_eq!(
            command_for("ssh://user@host:2222").as_deref(),
            Some("ssh -p '2222' 'user@host'")
        );
        assert_eq!(
            command_for("ssh://user@host:2222/some/path").as_deref(),
            Some("ssh -p '2222' 'user@host'")
        );
        assert_eq!(
            command_for("ssh://[::1]:2200").as_deref(),
            Some("ssh -p '2200' '[::1]'")
        );
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
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}

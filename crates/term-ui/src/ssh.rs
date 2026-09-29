//! SSH integration (ADR 0014): target parsing, `ssh -G` resolution,
//! ControlMaster connection reuse, and a remote terminfo bootstrap with a
//! zero-install fallback.

use std::process::Command;

/// A user-typed target: `[user@]host[:port]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
}

impl Target {
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        let (user, rest) = match input.split_once('@') {
            Some((u, r)) if !u.is_empty() => (Some(u.to_string()), r),
            _ => (None, input),
        };
        // Split host/port, keeping `[ipv6]:port` intact.
        let (host, port) = if let Some(stripped) = rest.strip_prefix('[') {
            match stripped.split_once(']') {
                Some((h, tail)) => (
                    format!("[{h}]"),
                    tail.strip_prefix(':').and_then(|p| p.parse().ok()),
                ),
                None => return None,
            }
        } else if let Some((h, p)) = rest.rsplit_once(':') {
            match p.parse::<u16>() {
                Ok(port) => (h.to_string(), Some(port)),
                Err(_) => (rest.to_string(), None),
            }
        } else {
            (rest.to_string(), None)
        };
        if host.is_empty() {
            return None;
        }
        Some(Target { host, user, port })
    }

    /// `[user@]host[:port]` as ssh accepts it on the command line.
    pub fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }
}

/// The subset of `ssh -G` output we use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
}

/// Parse `ssh -G <host>` output (lowercased `key value` lines).
pub fn parse_ssh_g(text: &str) -> Resolved {
    let mut out = Resolved::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "hostname" if !value.is_empty() => out.hostname = Some(value.to_string()),
            "user" if !value.is_empty() => out.user = Some(value.to_string()),
            "port" => out.port = value.parse().ok(),
            _ => {}
        }
    }
    out
}

/// Resolve a target through the user's ssh config, falling back to the typed
/// values when `ssh -G` is unavailable.
pub fn resolve(target: &Target) -> Target {
    let output = Command::new("ssh")
        .args(["-G", &target.destination()])
        .output();
    let Ok(output) = output else {
        return target.clone();
    };
    if !output.status.success() {
        return target.clone();
    }
    let resolved = parse_ssh_g(&String::from_utf8_lossy(&output.stdout));
    Target {
        host: resolved.hostname.unwrap_or_else(|| target.host.clone()),
        user: resolved.user.or_else(|| target.user.clone()),
        port: resolved.port.or(target.port),
    }
}

/// The shared ControlMaster socket, using ssh's `%` tokens.
pub fn control_path() -> String {
    "~/.ssh/miaotty-cm-%r@%h:%p".to_string()
}

/// Build the `ssh` invocation: connection reuse plus the remote bootstrap.
pub fn command(target: &Target, bootstrap: &str) -> String {
    let mut cmd = String::from("ssh -t");
    cmd.push_str(&format!(
        " -o ControlMaster=auto -o ControlPath={}",
        control_path()
    ));
    cmd.push_str(" -o ControlPersist=60s");
    if let Some(port) = target.port {
        cmd.push_str(&format!(" -p {port}"));
    }
    cmd.push(' ');
    cmd.push_str(&shell_quote(&target.destination()));
    cmd.push(' ');
    cmd.push_str(&shell_quote(bootstrap));
    cmd
}

/// The remote bootstrap: install the local terminfo entry if the host lacks it,
/// else fall back to `xterm-256color`, then start a login shell.
pub fn bootstrap_with(terminfo_b64: Option<&str>, term: &str) -> String {
    match terminfo_b64 {
        Some(b64) => format!(
            "sh -c 'if ! infocmp {term} >/dev/null 2>&1; then mkdir -p ~/.terminfo && \
             echo {b64} | base64 -d | tic -x -o ~/.terminfo - >/dev/null 2>&1; fi; \
             export TERM={term}; exec ${{SHELL:-sh}} -l'"
        ),
        None => "sh -c 'export TERM=xterm-256color; exec ${SHELL:-sh} -l'".to_string(),
    }
}

/// Compute the bootstrap, embedding the local `miaotty` terminfo entry when
/// `infocmp` can produce one.
pub fn bootstrap(term: &str) -> String {
    let entry = Command::new("infocmp").args(["-x", term]).output().ok();
    let b64 = entry
        .filter(|o| o.status.success() && !o.stdout.is_empty())
        .map(|o| base64_encode(&o.stdout));
    bootstrap_with(b64.as_deref(), term)
}

/// Standard base64 (RFC 4648) with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Cap on how much of a remote file we read into the reader.
pub const MAX_REMOTE_BYTES: u64 = 2 * 1024 * 1024;

/// The `ssh` argv for reading a remote file (`cat`), reusing the ControlMaster.
pub fn read_args(dest: &str, path: &str) -> Vec<String> {
    let mut args = base_args();
    args.push(dest.to_string());
    args.push(format!("cat -- {}", shell_quote(path)));
    args
}

/// The `ssh` argv for writing a remote file (`cat >`), reusing the ControlMaster.
pub fn write_args(dest: &str, path: &str) -> Vec<String> {
    let mut args = base_args();
    args.push(dest.to_string());
    args.push(format!("cat > {}", shell_quote(path)));
    args
}

fn base_args() -> Vec<String> {
    vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        format!("ControlPath={}", control_path()),
        "-o".to_string(),
        "ControlPersist=60s".to_string(),
    ]
}

/// Read a remote file over ssh (bounded). Errors carry the ssh stderr.
pub fn read_remote(dest: &str, path: &str) -> std::io::Result<Vec<u8>> {
    let out = Command::new("ssh").args(read_args(dest, path)).output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let mut data = out.stdout;
    data.truncate(MAX_REMOTE_BYTES as usize);
    Ok(data)
}

/// Write a remote file over ssh (`cat >`), feeding `data` on stdin.
pub fn write_remote(dest: &str, path: &str, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = Command::new("ssh")
        .args(write_args(dest, path))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(data)?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(
            Target::parse("alice@example.com"),
            Some(Target {
                host: "example.com".into(),
                user: Some("alice".into()),
                port: None
            })
        );
        assert_eq!(
            Target::parse("example.com:2222"),
            Some(Target {
                host: "example.com".into(),
                user: None,
                port: Some(2222)
            })
        );
        assert_eq!(Target::parse("[::1]:2200").unwrap().port, Some(2200));
        assert!(Target::parse("  ").is_none());
    }

    #[test]
    fn parses_ssh_g() {
        let text = "hostname example.com\nuser bob\nport 2222\ncontrolmaster auto\n";
        let r = parse_ssh_g(text);
        assert_eq!(r.hostname.as_deref(), Some("example.com"));
        assert_eq!(r.user.as_deref(), Some("bob"));
        assert_eq!(r.port, Some(2222));
    }

    #[test]
    fn builds_command_with_reuse_and_bootstrap() {
        let t = Target {
            host: "h".into(),
            user: Some("u".into()),
            port: Some(2200),
        };
        let cmd = command(&t, "sh -c 'x'");
        assert!(cmd.starts_with("ssh -t "));
        assert!(cmd.contains("ControlMaster=auto"));
        assert!(cmd.contains("ControlPath=~/.ssh/miaotty-cm-%r@%h:%p"));
        assert!(cmd.contains(" -p 2200 "));
        assert!(cmd.contains("'u@h'"));
    }

    #[test]
    fn bootstrap_variants() {
        assert!(bootstrap_with(None, "miaotty").contains("xterm-256color"));
        let with = bootstrap_with(Some("QUJD"), "miaotty");
        assert!(with.contains("base64 -d"));
        assert!(with.contains("QUJD"));
        assert!(with.contains("export TERM=miaotty"));
    }

    #[test]
    fn remote_args_reuse_control_master_and_quote() {
        let r = read_args("u@h", "/tmp/a b.txt");
        assert!(r.iter().any(|a| a == "BatchMode=yes"));
        assert!(r.iter().any(|a| a.starts_with("ControlPath=")));
        assert_eq!(r.last().unwrap(), "cat -- '/tmp/a b.txt'");
        let w = write_args("u@h", "x");
        assert_eq!(w.last().unwrap(), "cat > 'x'");
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}

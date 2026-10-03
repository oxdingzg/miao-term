//! Host keys and the ssh agent (B3.2), through the user's own OpenSSH tools.
//!
//! An interactive ssh in a pane asks about unknown host keys itself. Work that
//! runs ssh in the background (remote files, port forwarding, SFTP) uses
//! `BatchMode`, where an unknown key is just a failure, so the host library
//! checks keys up front: known, unknown (show the fingerprints and let the user
//! decide) or changed (refuse — that is what a man-in-the-middle looks like).
//! Nothing here stores or handles passwords.

use std::io::Write;
use std::process::Stdio;

/// How a host's key compares with `~/.ssh/known_hosts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKey {
    /// A key the host offers is already trusted.
    Known,
    /// Never seen: these are the offered keys' fingerprints.
    Unknown { fingerprints: Vec<String> },
    /// known_hosts has keys for the host, but none the host now offers.
    Changed { fingerprints: Vec<String> },
}

/// The `[host]:port` form known_hosts uses for a non-default port.
pub fn known_hosts_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// The key part (`type base64`) of known_hosts / ssh-keyscan lines.
fn keys(lines: &str) -> Vec<String> {
    lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let _host = parts.next()?;
            let kind = parts.next()?;
            let key = parts.next()?;
            Some(format!("{kind} {key}"))
        })
        .collect()
}

/// Compare what known_hosts holds for a host (`ssh-keygen -F` output) with
/// what the host offers now (`ssh-keyscan` output).
pub fn classify(known: &str, scanned: &str, fingerprints: Vec<String>) -> HostKey {
    let known = keys(known);
    let offered = keys(scanned);
    if offered.iter().any(|k| known.contains(k)) {
        HostKey::Known
    } else if known.is_empty() {
        HostKey::Unknown { fingerprints }
    } else {
        HostKey::Changed { fingerprints }
    }
}

/// `SHA256:…` fingerprints of keyscan output (`ssh-keygen -lf -`).
fn fingerprints(scanned: &str) -> Vec<String> {
    let Ok(mut child) = miao_term_platform::background_command("ssh-keygen")
        .args(["-l", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(scanned.as_bytes());
    }
    child
        .wait_with_output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The host, port and jump host ssh would use for `destination` (from
/// `ssh -G`, so aliases resolve). Slow: run off the UI thread.
pub fn resolve(
    destination: &str,
    options: &[String],
) -> Result<(String, u16, Option<String>), String> {
    let out = miao_term_platform::background_command("ssh")
        .args(options)
        .arg("-G")
        .arg(destination)
        .output()
        .map_err(|e| format!("ssh: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix(' ')))
            .map(str::to_string)
    };
    let host = field("hostname").ok_or("ssh -G gave no hostname")?;
    let port = field("port").and_then(|p| p.parse().ok()).unwrap_or(22);
    let jump = field("proxyjump").filter(|j| j != "none");
    Ok((host, port, jump))
}

/// Check a host's key. A host reached through a jump host cannot be scanned
/// directly, so only its known_hosts entry is reported.
pub fn check(destination: &str, options: &[String]) -> Result<HostKey, String> {
    let (host, port, jump) = resolve(destination, options)?;
    let name = known_hosts_name(&host, port);
    let known = miao_term_platform::background_command("ssh-keygen")
        .args(["-F", &name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if jump.is_some() {
        return Ok(if keys(&known).is_empty() {
            HostKey::Unknown {
                fingerprints: Vec::new(),
            }
        } else {
            HostKey::Known
        });
    }
    let scanned = miao_term_platform::background_command("ssh-keyscan")
        .args(["-T", "5", "-p", &port.to_string(), &host])
        .output()
        .map_err(|e| format!("ssh-keyscan: {e}"))?;
    let scanned = String::from_utf8_lossy(&scanned.stdout).to_string();
    if keys(&scanned).is_empty() {
        return Err(format!("{name} did not answer with a host key"));
    }
    let prints = fingerprints(&scanned);
    Ok(classify(&known, &scanned, prints))
}

/// Trust the keys a host offers now (after the user compared fingerprints):
/// append them to `~/.ssh/known_hosts`. Never used for a changed key.
pub fn trust(destination: &str, options: &[String]) -> Result<(), String> {
    let (host, port, _) = resolve(destination, options)?;
    let scanned = miao_term_platform::background_command("ssh-keyscan")
        .args(["-T", "5", "-p", &port.to_string(), &host])
        .output()
        .map_err(|e| format!("ssh-keyscan: {e}"))?;
    let scanned = String::from_utf8_lossy(&scanned.stdout).to_string();
    if keys(&scanned).is_empty() {
        return Err("no host key received".into());
    }
    let home = miao_term_config::home_dir().ok_or("no home directory")?;
    let dir = home.join(".ssh");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("known_hosts"))
        .map_err(|e| e.to_string())?;
    let lines: String = scanned
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    file.write_all(lines.as_bytes()).map_err(|e| e.to_string())
}

/// What the ssh agent holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agent {
    Keys(Vec<String>),
    NoKeys,
    NotRunning,
}

/// Parse `ssh-add -l` (exit 0: keys, 1: none, 2: no agent).
pub fn parse_agent(code: Option<i32>, stdout: &str) -> Agent {
    match code {
        Some(0) => Agent::Keys(
            stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        Some(1) => Agent::NoKeys,
        _ => Agent::NotRunning,
    }
}

pub fn agent_status() -> Agent {
    match miao_term_platform::background_command("ssh-add")
        .arg("-l")
        .output()
    {
        Ok(o) => parse_agent(o.status.code(), &String::from_utf8_lossy(&o.stdout)),
        Err(_) => Agent::NotRunning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA";
    const KEY_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB";

    #[test]
    fn host_keys_are_known_unknown_or_changed() {
        let known = format!("# Host example.com found: line 3\nexample.com {KEY_A}\n");
        let offered_same = format!("example.com {KEY_A}\nexample.com ecdsa-sha2-nistp256 AAAX\n");
        let offered_other = format!("example.com {KEY_B}\n");
        assert_eq!(classify(&known, &offered_same, vec![]), HostKey::Known);
        assert_eq!(
            classify("", &offered_other, vec!["SHA256:x".into()]),
            HostKey::Unknown {
                fingerprints: vec!["SHA256:x".into()]
            }
        );
        assert!(matches!(
            classify(&known, &offered_other, vec![]),
            HostKey::Changed { .. }
        ));
    }

    #[test]
    fn non_default_ports_use_brackets() {
        assert_eq!(known_hosts_name("example.com", 22), "example.com");
        assert_eq!(known_hosts_name("example.com", 2222), "[example.com]:2222");
    }

    #[test]
    fn agent_listing_is_parsed_by_exit_code() {
        let list = "256 SHA256:abc you@laptop (ED25519)\n";
        assert_eq!(
            parse_agent(Some(0), list),
            Agent::Keys(vec!["256 SHA256:abc you@laptop (ED25519)".into()])
        );
        assert_eq!(
            parse_agent(Some(1), "The agent has no identities."),
            Agent::NoKeys
        );
        assert_eq!(parse_agent(Some(2), ""), Agent::NotRunning);
        assert_eq!(parse_agent(None, ""), Agent::NotRunning);
    }

    /// Against a real host you have connected to: `MTTY_HOSTKEY_TEST=<alias>`.
    #[test]
    fn a_host_you_trust_checks_as_known() {
        let Ok(host) = std::env::var("MTTY_HOSTKEY_TEST") else {
            return;
        };
        assert_eq!(check(&host, &[]).unwrap(), HostKey::Known);
    }
}

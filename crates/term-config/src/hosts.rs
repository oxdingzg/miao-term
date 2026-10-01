//! The host library (B3.1): saved SSH hosts in `~/.config/mtty/hosts.toml`.
//!
//! ```toml
//! [[host]]
//! name = "web-1"
//! address = "203.0.113.5"
//! user = "deploy"
//! port = 2222
//! group = "prod"
//! tags = ["web"]
//! jump = "bastion"
//! tmux = "mtty"        # keep the shell in a tmux session (optional)
//! mosh = true          # connect with mosh (optional)
//!
//! [[host]]
//! name = "work"        # imported from ~/.ssh/config
//! alias = true
//! ```
//!
//! A host imported from `~/.ssh/config` is connected by its alias, so every
//! option ssh has for that `Host` block (keys, ProxyJump, …) applies. No
//! passwords or keys are stored here.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    /// Connect with `ssh <name>` and let `~/.ssh/config` supply the rest.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alias: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// A jump host (`ssh -J`), for hosts not described in `~/.ssh/config`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    /// Keep the remote shell in this tmux session, so reconnecting returns to
    /// it (B3.6). Ignored where the host has no tmux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux: Option<String>,
    /// Connect with mosh (survives roaming and sleep; needs mosh on both ends).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mosh: bool,
    /// Saved port forwards (B3.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "forward")]
    pub forwards: Vec<Forward>,
}

/// One port forward, in ssh's own notation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forward {
    pub kind: ForwardKind,
    /// `[bind:]port:host:hostport` for local/remote, `[bind:]port` for dynamic.
    pub spec: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardKind {
    /// `-L`: a local port reaches a host as seen from the server.
    Local,
    /// `-R`: a port on the server reaches a host as seen from here.
    Remote,
    /// `-D`: a local SOCKS proxy through the server.
    Dynamic,
}

impl ForwardKind {
    pub fn flag(self) -> &'static str {
        match self {
            ForwardKind::Local => "-L",
            ForwardKind::Remote => "-R",
            ForwardKind::Dynamic => "-D",
        }
    }
}

impl Forward {
    /// Check the spec's shape before handing it to ssh.
    pub fn validate(&self) -> Result<(), String> {
        let spec = self.spec.trim();
        if spec.is_empty() || spec.chars().any(char::is_whitespace) {
            return Err("empty or contains spaces".into());
        }
        // Split on ':' outside [ipv6] brackets.
        let mut parts = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        for (i, c) in spec.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                ':' if depth == 0 => {
                    parts.push(&spec[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(&spec[start..]);
        let port_ok = |p: &str| p.parse::<u16>().is_ok_and(|n| n > 0);
        let ok = match (self.kind, parts.as_slice()) {
            (ForwardKind::Dynamic, [port]) | (ForwardKind::Dynamic, [_, port]) => port_ok(port),
            (ForwardKind::Local | ForwardKind::Remote, [port, host, hport])
            | (ForwardKind::Local | ForwardKind::Remote, [_, port, host, hport]) => {
                port_ok(port) && !host.is_empty() && port_ok(hport)
            }
            _ => false,
        };
        if ok {
            Ok(())
        } else {
            Err(match self.kind {
                ForwardKind::Dynamic => "use [bind:]port, e.g. 1080".into(),
                _ => "use [bind:]port:host:hostport, e.g. 8080:localhost:80".into(),
            })
        }
    }
}

impl Host {
    /// The ssh destination: the alias itself, or `[user@]address`.
    pub fn destination(&self) -> String {
        if self.alias {
            return self.name.clone();
        }
        let address = self.address.as_deref().unwrap_or(&self.name);
        match &self.user {
            Some(user) => format!("{user}@{address}"),
            None => address.to_string(),
        }
    }

    /// Extra ssh options beyond the destination (`-p`, `-J`).
    pub fn ssh_options(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.alias {
            return out;
        }
        if let Some(port) = self.port {
            out.push("-p".into());
            out.push(port.to_string());
        }
        if let Some(jump) = self.jump.as_deref().filter(|j| !j.trim().is_empty()) {
            out.push("-J".into());
            out.push(jump.trim().to_string());
        }
        out
    }

    /// A one-line description for lists.
    pub fn summary(&self) -> String {
        if self.alias {
            return "~/.ssh/config".into();
        }
        let mut s = self.destination();
        if let Some(port) = self.port {
            s.push_str(&format!(":{port}"));
        }
        if let Some(jump) = &self.jump {
            s.push_str(&format!(" via {jump}"));
        }
        s
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBook {
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
}

impl HostBook {
    pub fn path() -> Option<PathBuf> {
        Some(crate::config_dir()?.join("hosts.toml"))
    }

    /// Load the library; a missing file is an empty library, a malformed one
    /// is an error (so a save never overwrites what could not be read).
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                let first = e.to_string();
                format!("hosts.toml: {}", first.lines().next().unwrap_or_default())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Add hosts whose name is not in the library yet; returns how many.
    pub fn merge(&mut self, imported: Vec<Host>) -> usize {
        let mut added = 0;
        for host in imported {
            if !self.hosts.iter().any(|h| h.name == host.name) {
                self.hosts.push(host);
                added += 1;
            }
        }
        added
    }

    /// Hosts sorted for display: by group (ungrouped first), then name.
    pub fn sorted(&self) -> Vec<(usize, &Host)> {
        let mut out: Vec<(usize, &Host)> = self.hosts.iter().enumerate().collect();
        out.sort_by(|a, b| (&a.1.group, &a.1.name).cmp(&(&b.1.group, &b.1.name)));
        out
    }
}

/// The concrete hosts named in an ssh config (`Host` patterns without
/// wildcards or negation), as alias entries. `Include` is not followed.
pub fn parse_ssh_config(text: &str) -> Vec<Host> {
    let mut out: Vec<Host> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (
                k.to_ascii_lowercase(),
                v.trim().trim_start_matches('=').trim(),
            ),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for name in value.split_whitespace() {
                    let concrete = !name.contains(['*', '?', '!']);
                    if concrete && !out.iter().any(|h| h.name == name) {
                        out.push(Host {
                            name: name.to_string(),
                            alias: true,
                            ..Default::default()
                        });
                        current.push(out.len() - 1);
                    }
                }
            }
            "match" => current.clear(),
            // Shown in the summary only; ssh itself reads the config.
            "hostname" => current
                .iter()
                .for_each(|&i| out[i].address = Some(value.into())),
            "user" => current
                .iter()
                .for_each(|&i| out[i].user = Some(value.into())),
            _ => {}
        }
    }
    out
}

/// `~/.ssh/config`, if readable.
pub fn read_ssh_config() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    std::fs::read_to_string(PathBuf::from(home).join(".ssh/config")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_import_keeps_concrete_aliases() {
        let config = "\
# comment
Host *
  ServerAliveInterval 30
Host work work-alt
  HostName 203.0.113.5
  User deploy
  ProxyJump bastion
Host *.internal !secret
  User x
Host=db
  HostName=db.example.com
Match host foo
  User nobody
";
        let hosts = parse_ssh_config(config);
        let names: Vec<_> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["work", "work-alt", "db"]);
        assert!(hosts.iter().all(|h| h.alias));
        assert_eq!(hosts[0].address.as_deref(), Some("203.0.113.5"));
        assert_eq!(hosts[0].user.as_deref(), Some("deploy"));
        assert_eq!(hosts[2].address.as_deref(), Some("db.example.com"));
        // Aliases connect by name so ssh applies the whole Host block.
        assert_eq!(hosts[0].destination(), "work");
        assert!(hosts[0].ssh_options().is_empty());
    }

    #[test]
    fn manual_hosts_build_their_own_options() {
        let host = Host {
            name: "web-1".into(),
            address: Some("203.0.113.7".into()),
            user: Some("deploy".into()),
            port: Some(2222),
            jump: Some("bastion".into()),
            ..Default::default()
        };
        assert_eq!(host.destination(), "deploy@203.0.113.7");
        assert_eq!(host.ssh_options(), ["-p", "2222", "-J", "bastion"]);
        assert_eq!(host.summary(), "deploy@203.0.113.7:2222 via bastion");
    }

    #[test]
    fn the_library_round_trips_and_merges_by_name() {
        let mut book = HostBook {
            hosts: vec![Host {
                name: "web-1".into(),
                address: Some("203.0.113.7".into()),
                group: Some("prod".into()),
                tags: vec!["web".into()],
                ..Default::default()
            }],
        };
        let text = toml::to_string_pretty(&book).unwrap();
        assert!(text.contains("[[host]]"), "{text}");
        assert_eq!(toml::from_str::<HostBook>(&text).unwrap(), book);
        let added = book.merge(parse_ssh_config("Host web-1\nHost work\n"));
        assert_eq!(added, 1, "an existing name is kept as configured");
        let order: Vec<_> = book.sorted().iter().map(|(_, h)| h.name.clone()).collect();
        assert_eq!(order, ["work", "web-1"], "ungrouped first, then by group");
    }

    #[test]
    fn forward_specs_follow_ssh_notation() {
        let f = |kind, spec: &str| {
            Forward {
                kind,
                spec: spec.into(),
            }
            .validate()
        };
        assert!(f(ForwardKind::Local, "8080:localhost:80").is_ok());
        assert!(f(ForwardKind::Local, "127.0.0.1:8080:db.internal:5432").is_ok());
        assert!(f(ForwardKind::Remote, "9000:[::1]:9000").is_ok());
        assert!(f(ForwardKind::Dynamic, "1080").is_ok());
        assert!(f(ForwardKind::Dynamic, "localhost:1080").is_ok());
        for bad in [
            "",
            "8080",
            "8080:host",
            "0:h:80",
            "x:h:80",
            "8080:h:99999",
            "80 80:h:1",
        ] {
            assert!(f(ForwardKind::Local, bad).is_err(), "{bad:?}");
        }
        assert!(f(ForwardKind::Dynamic, "8080:h:80").is_err());
        let host = Host {
            name: "db".into(),
            forwards: vec![Forward {
                kind: ForwardKind::Local,
                spec: "5432:localhost:5432".into(),
            }],
            ..Default::default()
        };
        let text = toml::to_string_pretty(&HostBook {
            hosts: vec![host.clone()],
        })
        .unwrap();
        assert!(
            text.contains("[[host.forward]]") && text.contains("kind = \"local\""),
            "{text}"
        );
        assert_eq!(toml::from_str::<HostBook>(&text).unwrap().hosts[0], host);
    }
}

//! Port forwards (B3.3): one background `ssh -N` per rule, owned by mtty.
//!
//! A dedicated process (rather than `ssh -O forward` on the shared master)
//! keeps a forward alive independently of the 60 s ControlPersist of pane
//! connections, makes its state plain — running, or exited with ssh's own
//! message — and ends with mtty (the child is killed on drop).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use miao_term_config::hosts::Forward;

/// The ssh arguments for one forward to `destination`.
pub fn args(destination: &str, options: &[String], forward: &Forward) -> Vec<String> {
    let mut out: Vec<String> = [
        "-N",
        "-o",
        "BatchMode=yes",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ServerAliveInterval=30",
        "-o",
        "ServerAliveCountMax=3",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    out.extend(options.iter().cloned());
    out.push(forward.kind.flag().to_string());
    out.push(forward.spec.trim().to_string());
    out.push(destination.to_string());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelState {
    Running,
    /// ssh exited; its last error line, if it printed one.
    Exited(String),
}

pub struct Tunnel {
    child: Child,
    pub started: Instant,
    stderr: Arc<Mutex<String>>,
    exited: Option<String>,
}

impl Tunnel {
    pub fn start(
        destination: &str,
        options: &[String],
        forward: &Forward,
    ) -> std::io::Result<Self> {
        forward
            .validate()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let mut child = Command::new("ssh")
            .args(args(destination, options, forward))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let sink = stderr.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut text = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if text.len() < 16 * 1024 {
                        text.push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                }
            });
        }
        Ok(Self {
            child,
            started: Instant::now(),
            stderr,
            exited: None,
        })
    }

    pub fn state(&mut self) -> TunnelState {
        if let Some(msg) = &self.exited {
            return TunnelState::Exited(msg.clone());
        }
        match self.child.try_wait() {
            Ok(None) => TunnelState::Running,
            Ok(Some(status)) => {
                // Give the reader thread a moment to collect the message.
                std::thread::sleep(std::time::Duration::from_millis(50));
                let text = self
                    .stderr
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let last = text
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty() && !l.starts_with("**"))
                    .map(str::trim)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("ssh exited ({status})"));
                self.exited = Some(last.clone());
                TunnelState::Exited(last)
            }
            Err(e) => TunnelState::Exited(e.to_string()),
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miao_term_config::hosts::ForwardKind;

    #[test]
    fn forwards_run_ssh_without_a_shell_and_fail_loudly() {
        let fwd = Forward {
            kind: ForwardKind::Local,
            spec: "8080:localhost:80".into(),
        };
        let a = args("deploy@203.0.113.5", &["-p".into(), "2222".into()], &fwd);
        assert_eq!(a[0], "-N");
        assert!(a.contains(&"ExitOnForwardFailure=yes".to_string()));
        assert!(a.contains(&"BatchMode=yes".to_string()));
        assert_eq!(
            &a[a.len() - 5..],
            [
                "-p",
                "2222",
                "-L",
                "8080:localhost:80",
                "deploy@203.0.113.5"
            ]
        );
        let bad = Forward {
            kind: ForwardKind::Local,
            spec: "nope".into(),
        };
        assert!(Tunnel::start("h", &[], &bad).is_err());
    }

    /// Against a real host: `MTTY_FORWARD_TEST_HOST=<alias>` forwards a free
    /// local port to the host's own sshd and reads its banner through it.
    #[test]
    fn a_local_forward_reaches_the_remote_side() {
        let Ok(host) = std::env::var("MTTY_FORWARD_TEST_HOST") else {
            return;
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let fwd = Forward {
            kind: ForwardKind::Local,
            spec: format!("127.0.0.1:{port}:localhost:22"),
        };
        let mut tunnel = Tunnel::start(&host, &[], &fwd).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        let banner = loop {
            assert_eq!(tunnel.state(), TunnelState::Running);
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut buf = [0u8; 64];
                if let Ok(n) = s.read(&mut buf) {
                    if n > 0 {
                        break String::from_utf8_lossy(&buf[..n]).to_string();
                    }
                }
            }
            assert!(Instant::now() < deadline, "forward never came up");
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        assert!(banner.starts_with("SSH-"), "{banner:?}");
        drop(tunnel);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "stopping closes the port"
        );
    }

    #[test]
    fn a_failing_forward_reports_why() {
        // A port that cannot be bound makes ExitOnForwardFailure end ssh.
        let fwd = Forward {
            kind: ForwardKind::Local,
            spec: "127.0.0.1:1:localhost:22".into(),
        };
        let Ok(mut tunnel) = Tunnel::start("mtty-no-such-host.invalid", &[], &fwd) else {
            return;
        };
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let TunnelState::Exited(msg) = tunnel.state() {
                assert!(!msg.is_empty());
                break;
            }
            assert!(Instant::now() < deadline, "ssh should give up");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

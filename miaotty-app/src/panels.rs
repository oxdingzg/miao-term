//! Background data for the Details panels: git status, directory listing and
//! listening ports. Work runs on a worker thread so the UI never blocks; the
//! latest result is published to a shared slot (ADR 0009).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// One entry in the Files panel.
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

/// Git branch + changed paths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    /// `(status code, path)` as reported by `git status --porcelain`.
    pub changes: Vec<(String, String)>,
}

/// A listening TCP port.
#[derive(Debug, Clone, PartialEq)]
pub struct PortInfo {
    pub port: u16,
    pub process: String,
}

/// The published snapshot for one (cwd, pid).
#[derive(Debug, Clone, Default)]
pub struct Panels {
    pub git: Option<GitInfo>,
    pub files: Vec<FileEntry>,
    pub ports: Vec<PortInfo>,
}

/// A request to refresh for a given context.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub cwd: Option<String>,
    pub pid: Option<u32>,
}

/// Handle to the background refresher.
pub struct PanelsWorker {
    tx: Sender<Request>,
    slot: Arc<Mutex<Panels>>,
}

impl PanelsWorker {
    pub fn spawn() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        let slot = Arc::new(Mutex::new(Panels::default()));
        let worker_slot = slot.clone();
        std::thread::spawn(move || worker(rx, worker_slot));
        Self { tx, slot }
    }

    /// Queue a refresh; coalesces so a busy UI cannot flood the worker.
    pub fn request(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    pub fn snapshot(&self) -> Panels {
        self.slot.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

fn worker(rx: Receiver<Request>, slot: Arc<Mutex<Panels>>) {
    while let Ok(req) = rx.recv() {
        // Drop stale requests: only the newest pending one matters.
        let mut req = req;
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        let git = req.cwd.as_deref().and_then(git_info);
        let files = req.cwd.as_deref().map(list_dir).unwrap_or_default();
        let ports = req.pid.map(list_ports).unwrap_or_default();
        if let Ok(mut guard) = slot.lock() {
            *guard = Panels { git, files, ports };
        }
    }
}

/// `git -C <cwd> rev-parse --abbrev-ref HEAD` + `status --porcelain`.
fn git_info(cwd: &str) -> Option<GitInfo> {
    let branch = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();

    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    let changes = parse_porcelain(&String::from_utf8_lossy(&status.stdout));
    Some(GitInfo { branch, changes })
}

/// Parse `git status --porcelain` lines into `(XY, path)`.
pub fn parse_porcelain(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let code = l[..2].trim().to_string();
            let mut path = l[3..].to_string();
            // `status --porcelain` quotes unusual paths.
            path = path.trim_matches('"').to_string();
            (code, path)
        })
        .collect()
}

/// List a directory, directories first then case-insensitive name order.
fn list_dir(cwd: &str) -> Vec<FileEntry> {
    read_dir_entries(Path::new(cwd))
}

pub fn read_dir_entries(dir: &Path) -> Vec<FileEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<FileEntry> = read
        .flatten()
        .map(|e| {
            let meta = e.metadata().ok();
            FileEntry {
                name: e.file_name().to_string_lossy().to_string(),
                path: e.path(),
                is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Listening TCP ports owned by `pid` (uses `lsof`; empty where unavailable).
fn list_ports(pid: u32) -> Vec<PortInfo> {
    let Ok(out) = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-a", "-p"])
        .arg(pid.to_string())
        .output()
    else {
        return Vec::new();
    };
    parse_lsof(&String::from_utf8_lossy(&out.stdout))
}

/// Extract `(port, command)` from `lsof -nP -iTCP` output.
pub fn parse_lsof(text: &str) -> Vec<PortInfo> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(command) = tokens.first() else {
            continue;
        };
        // The NAME column is `addr:port` (a trailing `(LISTEN)` may follow).
        let name = tokens.iter().rev().find(|t| {
            t.rsplit_once(':')
                .is_some_and(|(host, port)| {
                    !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
                })
        });
        let Some(name) = name else {
            continue;
        };
        let port = name.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok());
        if let Some(port) = port {
            if !out.iter().any(|p: &PortInfo| p.port == port) {
                out.push(PortInfo {
                    port,
                    process: command.to_string(),
                });
            }
        }
    }
    out.sort_by_key(|p| p.port);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parses_codes_and_paths() {
        let out = parse_porcelain(" M src/main.rs\n?? new.txt\nA  added.rs\n");
        assert_eq!(
            out,
            vec![
                ("M".to_string(), "src/main.rs".to_string()),
                ("??".to_string(), "new.txt".to_string()),
                ("A".to_string(), "added.rs".to_string()),
            ]
        );
        assert!(parse_porcelain("").is_empty());
    }

    #[test]
    fn lsof_parses_ports_and_dedupes() {
        let text = "\
COMMAND   PID USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
node    12345  me    20u  IPv6 0x1234      0t0  TCP *:3000 (LISTEN)
node    12345  me    21u  IPv4 0x1235      0t0  TCP 127.0.0.1:3000 (LISTEN)
postgres 1111  me    5u   IPv4 0x1236      0t0  TCP *:5432 (LISTEN)
";
        let ports = parse_lsof(text);
        assert_eq!(
            ports,
            vec![
                PortInfo {
                    port: 3000,
                    process: "node".into()
                },
                PortInfo {
                    port: 5432,
                    process: "postgres".into()
                },
            ]
        );
    }
}

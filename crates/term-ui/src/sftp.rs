//! SFTP through the user's OpenSSH `sftp` client in batch mode (B3.4).
//!
//! Using the system client keeps `~/.ssh/config` (aliases, ProxyJump, keys),
//! the ssh agent and the shared ControlMaster connection exactly as they are
//! for shells. The long listing is formatted by the sftp *client* from the
//! file attributes, so it looks the same whatever system the server runs.
//! Every call blocks: run them as background jobs.

use std::io::Write;
use std::process::Stdio;

/// A remote directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    /// `rwxr-xr-x` style permissions with the type letter (`drwxr-xr-x`).
    pub perms: String,
    /// As sftp shows it (`Oct  1 17:49` or `Apr 27  2025`).
    pub modified: String,
}

/// An sftp target: destination plus the host's ssh options (`-p`, `-J`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub destination: String,
    pub options: Vec<String>,
}

/// A path as one sftp batch argument.
pub fn quote(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Join a remote directory and a name with `/`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent of a remote path (`/` stays `/`).
pub fn parent(dir: &str) -> String {
    let trimmed = dir.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => trimmed[..i].to_string(),
    }
}

impl Remote {
    fn args(&self) -> Vec<String> {
        let mut out: Vec<String> = ["-q", "-b", "-", "-o", "BatchMode=yes"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        out.extend(crate::ssh::reuse_options());
        // sftp spells the port flag -P; -J is the same.
        let mut opts = self.options.iter();
        while let Some(flag) = opts.next() {
            match (flag.as_str(), opts.next()) {
                ("-p", Some(port)) => out.extend(["-P".into(), port.clone()]),
                (other, Some(value)) => out.extend([other.to_string(), value.clone()]),
                (other, None) => out.push(other.to_string()),
            }
        }
        out.push(self.destination.clone());
        out
    }

    /// Run a batch script; stdout without the echoed `sftp>` lines, or the
    /// first error sftp reported.
    pub fn run(&self, script: &str) -> Result<String, String> {
        let mut child = mtty_platform::background_command("sftp")
            .args(self.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("sftp: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(script.as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let stdout: String = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.starts_with("sftp> "))
            .map(|l| format!("{l}\n"))
            .collect();
        if out.status.success() {
            Ok(stdout)
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err
                .lines()
                .rfind(|l| !l.starts_with("**") && !l.trim().is_empty())
                .unwrap_or("sftp failed")
                .to_string())
        }
    }

    /// The remote home directory (where sftp starts).
    pub fn home(&self) -> Result<String, String> {
        let out = self.run("pwd\n")?;
        out.lines()
            .find_map(|l| l.strip_prefix("Remote working directory: "))
            .map(|s| s.trim().to_string())
            .ok_or_else(|| "sftp did not report a directory".into())
    }

    pub fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, String> {
        let out = self.run(&format!("ls -la {}\n", quote(dir)))?;
        Ok(parse_ls(dir, &out))
    }

    /// Download a file or (recursively) a directory into `local_dir`.
    pub fn download(&self, remote: &str, local_dir: &std::path::Path) -> Result<(), String> {
        self.run(&format!(
            "get -R -p {} {}\n",
            quote(remote),
            quote(&local_dir.to_string_lossy())
        ))
        .map(|_| ())
    }

    /// Upload a local file or directory into `remote_dir`.
    pub fn upload(&self, local: &std::path::Path, remote_dir: &str) -> Result<(), String> {
        self.run(&format!(
            "put -R -p {} {}\n",
            quote(&local.to_string_lossy()),
            quote(remote_dir)
        ))
        .map(|_| ())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.run(&format!("rename {} {}\n", quote(from), quote(to)))
            .map(|_| ())
    }

    /// `mode` is octal, e.g. `644`.
    pub fn chmod(&self, mode: &str, path: &str) -> Result<(), String> {
        if mode.is_empty() || mode.len() > 4 || !mode.chars().all(|c| ('0'..='7').contains(&c)) {
            return Err("use an octal mode such as 644 or 755".into());
        }
        self.run(&format!("chmod {mode} {}\n", quote(path)))
            .map(|_| ())
    }

    pub fn mkdir(&self, path: &str) -> Result<(), String> {
        self.run(&format!("mkdir {}\n", quote(path))).map(|_| ())
    }

    /// Remove a file, or an empty directory.
    pub fn remove(&self, path: &str, is_dir: bool) -> Result<(), String> {
        let verb = if is_dir { "rmdir" } else { "rm" };
        self.run(&format!("{verb} {}\n", quote(path))).map(|_| ())
    }
}

/// Where the file browser points: a host over SFTP, or an FTP/FTPS server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Sftp(Remote),
    Ftp(crate::ftp::Remote),
}

macro_rules! each {
    ($self:ident, $r:ident => $e:expr) => {
        match $self {
            Endpoint::Sftp($r) => $e,
            Endpoint::Ftp($r) => $e,
        }
    };
}

impl Endpoint {
    /// Plain FTP: shown with a warning in the browser.
    pub fn is_plaintext(&self) -> bool {
        matches!(self, Endpoint::Ftp(r) if r.is_plaintext())
    }
    pub fn home(&self) -> Result<String, String> {
        each!(self, r => r.home())
    }
    pub fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, String> {
        each!(self, r => r.list(dir))
    }
    pub fn download(&self, remote: &str, local_dir: &std::path::Path) -> Result<(), String> {
        each!(self, r => r.download(remote, local_dir))
    }
    pub fn upload(&self, local: &std::path::Path, remote_dir: &str) -> Result<(), String> {
        each!(self, r => r.upload(local, remote_dir))
    }
    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        each!(self, r => r.rename(from, to))
    }
    pub fn chmod(&self, mode: &str, path: &str) -> Result<(), String> {
        each!(self, r => r.chmod(mode, path))
    }
    pub fn mkdir(&self, path: &str) -> Result<(), String> {
        each!(self, r => r.mkdir(path))
    }
    pub fn remove(&self, path: &str, is_dir: bool) -> Result<(), String> {
        each!(self, r => r.remove(path, is_dir))
    }
}

/// Parse `ls -la <dir>` output. Columns: perms, links, user, group, size,
/// month, day, time-or-year, then the path (which may contain spaces).
pub fn parse_ls(dir: &str, text: &str) -> Vec<RemoteEntry> {
    let prefix = join(dir, "");
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut cols = Vec::with_capacity(8);
        for _ in 0..8 {
            let Some(end) = rest.find(char::is_whitespace) else {
                break;
            };
            cols.push(&rest[..end]);
            rest = rest[end..].trim_start();
        }
        if cols.len() < 8 || rest.is_empty() {
            continue;
        }
        let perms = cols[0];
        if !perms.starts_with(['-', 'd', 'l', 'c', 'b', 's', 'p']) || perms.len() < 10 {
            continue;
        }
        let path = rest;
        let name = path.strip_prefix(&prefix).unwrap_or(path);
        // Symlinks are listed as `name -> target`.
        let name = if perms.starts_with('l') {
            name.split(" -> ").next().unwrap_or(name)
        } else {
            name
        };
        let name = name.rsplit('/').next().unwrap_or(name);
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }
        out.push(RemoteEntry {
            name: name.to_string(),
            is_dir: perms.starts_with('d'),
            is_link: perms.starts_with('l'),
            size: cols[4].parse().unwrap_or(0),
            perms: perms.to_string(),
            modified: format!("{} {} {}", cols[5], cols[6], cols[7]),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_parse_names_with_spaces_and_links() {
        let text = "\
drwxrwxrwt    ? root     root       143360 Oct  1 17:49 /tmp/.
drwxr-xr-x    ? root     root         4096 Apr 27 17:18 /tmp/..
-rw-r--r--    ? me       me             12 Oct  1 17:50 /tmp/a file 中文.txt
drwxr-xr-x    ? me       me           4096 Apr 27  2025 /tmp/proj
lrwxrwxrwx    ? me       me              9 Oct  1 17:50 /tmp/link -> /etc/hosts
";
        let entries = parse_ls("/tmp", text);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["proj", "a file 中文.txt", "link"]);
        assert!(entries[0].is_dir && entries[0].modified == "Apr 27 2025");
        assert_eq!(entries[1].size, 12);
        assert!(entries[2].is_link);
    }

    #[test]
    fn paths_are_quoted_and_joined() {
        assert_eq!(quote(r#"a "b"\c"#), r#""a \"b\"\\c""#);
        assert_eq!(join("/home/me", "x"), "/home/me/x");
        assert_eq!(join("/", "x"), "/x");
        assert_eq!(parent("/home/me/"), "/home");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
    }

    #[test]
    fn host_options_become_sftp_options() {
        let r = Remote {
            destination: "deploy@203.0.113.5".into(),
            options: vec!["-p".into(), "2222".into(), "-J".into(), "bastion".into()],
        };
        let a = r.args();
        assert!(a.windows(2).any(|w| w == ["-P", "2222"]), "{a:?}");
        assert!(a.windows(2).any(|w| w == ["-J", "bastion"]));
        assert_eq!(a.last().unwrap(), "deploy@203.0.113.5");
        assert!(r.chmod("9z", "/x").is_err());
    }

    /// Against a real host: `MTTY_SFTP_TEST_HOST=<alias>` round-trips a file
    /// with spaces and CJK in its name through a scratch directory.
    #[test]
    fn a_real_round_trip() {
        let Ok(host) = std::env::var("MTTY_SFTP_TEST_HOST") else {
            return;
        };
        let remote = Remote {
            destination: host,
            options: vec![],
        };
        let home = remote.home().unwrap();
        let dir = join(&home, &format!("mtty-sftp-test-{}", std::process::id()));
        remote.mkdir(&dir).unwrap();
        let local = std::env::temp_dir().join(format!("mtty-sftp-{}", std::process::id()));
        std::fs::create_dir_all(&local).unwrap();
        let name = "a file 中文.txt";
        std::fs::write(local.join(name), "hello sftp\n").unwrap();
        remote.upload(&local.join(name), &dir).unwrap();
        let listed = remote.list(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].name.as_str(), listed[0].size), (name, 11));
        let renamed = join(&dir, "renamed 文件.txt");
        remote.rename(&join(&dir, name), &renamed).unwrap();
        remote.chmod("600", &renamed).unwrap();
        assert_eq!(remote.list(&dir).unwrap()[0].perms, "-rw-------");
        let back = local.join("back");
        std::fs::create_dir_all(&back).unwrap();
        remote.download(&renamed, &back).unwrap();
        assert_eq!(
            std::fs::read_to_string(back.join("renamed 文件.txt")).unwrap(),
            "hello sftp\n"
        );
        remote.remove(&renamed, false).unwrap();
        remote.remove(&dir, true).unwrap();
        assert!(remote.list(&dir).is_err(), "the scratch directory is gone");
        std::fs::remove_dir_all(&local).unwrap();
    }
}

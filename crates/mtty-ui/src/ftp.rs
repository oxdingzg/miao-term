//! FTP and FTPS through the system `curl` (B3.6), for the same two-pane
//! browser as SFTP.
//!
//! The password, when one is typed, lives only in memory and reaches curl
//! as a config file on stdin (`-K -`), never on a command line other users
//! can read. Without one, curl uses `~/.netrc` for the user, or logs in
//! anonymously. curl's verbose mode prints `PASS` in clear, so it is never
//! used here. Every call blocks: run them as background jobs.

use std::io::Write;
use std::process::Stdio;

use crate::sftp::{join, RemoteEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Plain FTP: the password and the files cross the network unencrypted.
    Plain,
    /// `AUTH TLS` on the normal port, required for control and data.
    ExplicitTls,
    /// TLS from the first byte (`ftps://`, usually port 990).
    ImplicitTls,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub security: Security,
    /// Accept any server certificate (self-signed servers); off by default.
    pub insecure: bool,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("security", &self.security)
            .field("insecure", &self.insecure)
            .finish()
    }
}

/// Percent-encode one path segment for a URL.
fn encode_segment(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A string inside a curl config file's double quotes.
fn config_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// FTP takes the rest of a command line as the argument, so a path may hold
/// spaces but never a line break.
fn command_path(path: &str) -> Result<&str, String> {
    if path.contains(['\r', '\n']) {
        Err("a path with a line break cannot be sent over FTP".into())
    } else {
        Ok(path)
    }
}

impl Remote {
    /// Parse `ftp://[user@]host[:port]`, `ftpes://…` (explicit TLS) or
    /// `ftps://…` (implicit TLS); a bare host means plain FTP.
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let (security, rest) = if let Some(r) = input.strip_prefix("ftps://") {
            (Security::ImplicitTls, r)
        } else if let Some(r) = input.strip_prefix("ftpes://") {
            (Security::ExplicitTls, r)
        } else if let Some(r) = input.strip_prefix("ftp://") {
            (Security::Plain, r)
        } else {
            (Security::Plain, input)
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let (user, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u.to_string()).filter(|u| !u.is_empty()), h),
            None => (None, authority),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.starts_with('[') => (
                h.to_string(),
                Some(p.parse::<u16>().map_err(|_| format!("bad port: {p}"))?),
            ),
            _ => (hostport.to_string(), None),
        };
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err("enter a host such as ftp.example.com".into());
        }
        Ok(Self {
            host,
            port,
            user,
            password: None,
            security,
            insecure: false,
        })
    }

    /// `user@host[:port]` for titles.
    pub fn label(&self) -> String {
        let mut s = match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        };
        if let Some(p) = self.port {
            s.push_str(&format!(":{p}"));
        }
        s
    }

    pub fn is_plaintext(&self) -> bool {
        self.security == Security::Plain
    }

    fn base(&self) -> String {
        let scheme = match self.security {
            Security::ImplicitTls => "ftps",
            _ => "ftp",
        };
        let user = self
            .user
            .as_ref()
            .filter(|_| self.password.is_none())
            .map(|u| format!("{}@", encode_segment(u)))
            .unwrap_or_default();
        match self.port {
            Some(p) => format!("{scheme}://{user}{}:{p}", self.host),
            None => format!("{scheme}://{user}{}", self.host),
        }
    }

    /// The URL of an absolute path (`%2F` makes curl start at `/` rather
    /// than the login directory); directories end with `/`.
    pub fn url(&self, path: &str, dir: bool) -> String {
        let segments: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(encode_segment)
            .collect();
        let mut url = format!("{}/%2F{}", self.base(), segments.join("/"));
        if dir && !url.ends_with('/') {
            url.push('/');
        }
        url
    }

    /// The curl arguments shared by every call (the URL and the operation
    /// come after).
    pub fn args(&self) -> Vec<String> {
        let mut out: Vec<String> = ["-sS", "--connect-timeout", "15", "-K", "-"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if self.security == Security::ExplicitTls {
            out.push("--ssl-reqd".into());
        }
        if self.insecure {
            out.push("-k".into());
        }
        if self.password.is_none() {
            out.push("--netrc-optional".into());
        }
        out
    }

    /// The config curl reads on stdin: the credentials, if typed.
    fn stdin_config(&self) -> String {
        match (&self.user, &self.password) {
            (Some(u), Some(p)) => format!("user = \"{}:{}\"\n", config_string(u), config_string(p)),
            (None, Some(p)) => format!("user = \"anonymous:{}\"\n", config_string(p)),
            _ => String::new(),
        }
    }

    fn curl(&self, extra: &[String]) -> Result<Vec<u8>, String> {
        let mut child = mtty_platform::background_command("curl")
            .args(self.args())
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(self.stdin_config().as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("curl failed")
                .trim()
                .to_string())
        }
    }

    /// Run FTP commands (`-Q`) with no transfer besides a name listing of `/`.
    fn quote(&self, commands: &[String]) -> Result<(), String> {
        let mut args = Vec::new();
        for c in commands {
            args.push("-Q".to_string());
            args.push(c.clone());
        }
        args.extend(["--list-only".into(), "-o".into(), null_device().into()]);
        args.push(self.url("/", true));
        self.curl(&args).map(|_| ())
    }

    /// The directory the server starts in after login.
    pub fn home(&self) -> Result<String, String> {
        let out = self.curl(&[
            "--list-only".into(),
            "-o".into(),
            null_device().into(),
            "-w".into(),
            "%{ftp_entry_path}".into(),
            format!("{}/", self.base()),
        ])?;
        let path = String::from_utf8_lossy(&out).trim().to_string();
        Ok(if path.starts_with('/') {
            path
        } else {
            "/".into()
        })
    }

    pub fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, String> {
        let out = self.curl(&[self.url(dir, true)])?;
        Ok(parse_list(dir, &String::from_utf8_lossy(&out)))
    }

    /// Download a file, or (recursively) a directory, into `local_dir`.
    /// `is_dir` tells a directory apart from a same-named file.
    pub fn download(
        &self,
        remote: &str,
        is_dir: bool,
        local_dir: &std::path::Path,
    ) -> Result<(), String> {
        let name = remote.rsplit('/').next().unwrap_or(remote);
        download_tree(
            remote,
            &local_dir.join(name),
            is_dir,
            &mut |dir| self.list(dir),
            &mut |from, to| self.download_file(from, to),
        )
    }

    /// Fetch one remote file to the exact local `path`.
    fn download_file(&self, remote: &str, path: &std::path::Path) -> Result<(), String> {
        self.curl(&[
            "-o".into(),
            path.to_string_lossy().into_owned(),
            self.url(remote, false),
        ])
        .map(|_| ())
    }

    /// Upload a local file or (recursively) a directory into `remote_dir`.
    pub fn upload(&self, local: &std::path::Path, remote_dir: &str) -> Result<(), String> {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("no file name")?;
        upload_tree(
            local,
            &join(remote_dir, &name),
            &mut |dir| self.mkdir(dir),
            &mut |from, to| self.upload_file(from, to),
        )
    }

    /// Put one local file at the exact remote `path`.
    fn upload_file(&self, local: &std::path::Path, remote: &str) -> Result<(), String> {
        self.curl(&[
            "-T".into(),
            local.to_string_lossy().into_owned(),
            self.url(remote, false),
        ])
        .map(|_| ())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.quote(&[
            format!("RNFR {}", command_path(from)?),
            format!("RNTO {}", command_path(to)?),
        ])
    }

    pub fn chmod(&self, mode: &str, path: &str) -> Result<(), String> {
        if mode.is_empty() || mode.len() > 4 || !mode.chars().all(|c| ('0'..='7').contains(&c)) {
            return Err("use an octal mode such as 644 or 755".into());
        }
        self.quote(&[format!("SITE CHMOD {mode} {}", command_path(path)?)])
    }

    pub fn mkdir(&self, path: &str) -> Result<(), String> {
        self.quote(&[format!("MKD {}", command_path(path)?)])
    }

    /// Remove a file, or an empty directory.
    pub fn remove(&self, path: &str, is_dir: bool) -> Result<(), String> {
        let verb = if is_dir { "RMD" } else { "DELE" };
        self.quote(&[format!("{verb} {}", command_path(path)?)])
    }
}

/// Recursively download `remote` to the exact local `path`. `list` and
/// `fetch` are the network primitives; directories are created locally in
/// depth-first, name-sorted order so parents exist before their children.
fn download_tree(
    remote: &str,
    path: &std::path::Path,
    is_dir: bool,
    list: &mut dyn FnMut(&str) -> Result<Vec<RemoteEntry>, String>,
    fetch: &mut dyn FnMut(&str, &std::path::Path) -> Result<(), String>,
) -> Result<(), String> {
    if !is_dir {
        return fetch(remote, path);
    }
    std::fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries = list(remote)?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for entry in entries {
        let child = join(remote, &entry.name);
        download_tree(&child, &path.join(&entry.name), entry.is_dir, list, fetch)?;
    }
    Ok(())
}

/// Recursively upload `local` to the exact remote `path`. `mkdir` and `put`
/// are the network primitives; directories are created before their files.
fn upload_tree(
    local: &std::path::Path,
    remote: &str,
    mkdir: &mut dyn FnMut(&str) -> Result<(), String>,
    put: &mut dyn FnMut(&std::path::Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    if !local.is_dir() {
        return put(local, remote);
    }
    mkdir(remote)?;
    let mut children: Vec<std::path::PathBuf> = std::fs::read_dir(local)
        .map_err(|e| format!("{}: {e}", local.display()))?
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .collect();
    children.sort();
    for child in children {
        let Some(name) = child.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        upload_tree(&child, &join(remote, &name), mkdir, put)?;
    }
    Ok(())
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// Parse a `LIST` reply: Unix `ls -l` lines (most servers) or the MS-DOS
/// format of IIS (`10-01-26  05:49PM  <DIR>  name`).
pub fn parse_list(dir: &str, text: &str) -> Vec<RemoteEntry> {
    let mut out = crate::sftp::parse_ls(dir, text);
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut cols = Vec::with_capacity(3);
        for _ in 0..3 {
            let Some(end) = rest.find(char::is_whitespace) else {
                break;
            };
            cols.push(&rest[..end]);
            rest = rest[end..].trim_start();
        }
        let dos_date = |s: &str| {
            s.len() >= 8
                && s.chars().filter(|c| *c == '-').count() == 2
                && s.chars().all(|c| c.is_ascii_digit() || c == '-')
        };
        if cols.len() < 3 || rest.is_empty() || !dos_date(cols[0]) {
            continue;
        }
        let is_dir = cols[2].eq_ignore_ascii_case("<DIR>");
        let size = if is_dir {
            0
        } else {
            cols[2].parse().unwrap_or(0)
        };
        out.push(RemoteEntry {
            name: rest.to_string(),
            is_dir,
            is_link: false,
            size,
            perms: String::new(),
            modified: format!("{} {}", cols[0], cols[1]),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_with_scheme_user_and_port() {
        let r = Remote::parse("ftpes://deploy@ftp.example.com:2121").unwrap();
        assert_eq!(r.security, Security::ExplicitTls);
        assert_eq!(r.user.as_deref(), Some("deploy"));
        assert_eq!((r.host.as_str(), r.port), ("ftp.example.com", Some(2121)));
        assert_eq!(r.label(), "deploy@ftp.example.com:2121");
        let r = Remote::parse("ftp.example.com").unwrap();
        assert!(r.is_plaintext() && r.user.is_none() && r.port.is_none());
        assert_eq!(
            Remote::parse("ftps://h").unwrap().security,
            Security::ImplicitTls
        );
        assert!(Remote::parse("ftp://h:notaport").is_err());
        assert!(Remote::parse("").is_err());
    }

    #[test]
    fn urls_encode_paths_and_start_at_root() {
        let mut r = Remote::parse("ftp://me@h:21").unwrap();
        assert_eq!(
            r.url("/srv/a b/中.txt", false),
            "ftp://me@h:21/%2Fsrv/a%20b/%E4%B8%AD.txt"
        );
        assert_eq!(r.url("/", true), "ftp://me@h:21/%2F/");
        // A typed password goes to stdin, and the URL drops the user.
        r.password = Some("p\"w\\x".into());
        assert_eq!(r.url("/x", true), "ftp://h:21/%2Fx/");
        assert_eq!(r.stdin_config(), "user = \"me:p\\\"w\\\\x\"\n");
        assert!(r.args().iter().all(|a| !a.contains("p\"w")));
        assert!(
            !format!("{r:?}").contains("p\\\"w"),
            "Debug hides the password"
        );
        assert!(r.rename("/a\nDELE /b", "/c").is_err());
    }

    #[test]
    fn listings_parse_unix_and_dos_formats() {
        let unix = "drwxr-xr-x   2 me   wheel          64 Oct 01 10:05 sub dir\n\
                    -rw-r--r--   1 me   wheel          12 Oct 01 10:06 a.txt\n";
        let names: Vec<_> = parse_list("/", unix).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["sub dir", "a.txt"]);
        let dos = "10-01-26  05:49PM       <DIR>          My Folder\n\
                   10-01-26  05:50PM                 1234 report 1.csv\n";
        let e = parse_list("/", dos);
        assert_eq!(e.len(), 2);
        assert!(e[0].is_dir && e[0].name == "My Folder");
        assert_eq!((e[1].name.as_str(), e[1].size), ("report 1.csv", 1234));
    }

    #[test]
    fn folder_downloads_walk_the_tree_depth_first() {
        let root = std::env::temp_dir().join(format!("mtty-ftp-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut listings = |dir: &str| -> Result<Vec<RemoteEntry>, String> {
            let raw: &[(&str, bool)] = match dir {
                "/root" => &[("sub", true), ("a.txt", false)],
                "/root/sub" => &[("b.txt", false)],
                _ => &[],
            };
            Ok(raw
                .iter()
                .map(|(name, is_dir)| RemoteEntry {
                    name: (*name).into(),
                    is_dir: *is_dir,
                    is_link: false,
                    size: 0,
                    perms: String::new(),
                    modified: String::new(),
                })
                .collect())
        };
        let mut fetched: Vec<String> = Vec::new();
        let mut fetch = |remote: &str, path: &std::path::Path| -> Result<(), String> {
            fetched.push(format!(
                "{} -> {}",
                remote,
                path.file_name().unwrap().to_string_lossy()
            ));
            std::fs::write(path, b"x").map_err(|e| e.to_string())
        };
        download_tree("/root", &root, true, &mut listings, &mut fetch).unwrap();
        assert_eq!(
            fetched,
            [
                "/root/a.txt -> a.txt".to_string(),
                "/root/sub/b.txt -> b.txt".to_string()
            ]
        );
        assert!(root.join("sub").is_dir());
        assert_eq!(
            std::fs::read_to_string(root.join("sub/b.txt")).unwrap(),
            "x"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn folder_uploads_create_remote_dirs_before_files() {
        let root = std::env::temp_dir().join(format!("mtty-ftp-ul-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"b").unwrap();
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut mkdir = {
            let events = events.clone();
            move |remote: &str| -> Result<(), String> {
                events.borrow_mut().push(format!("mkdir {remote}"));
                Ok(())
            }
        };
        let mut put = {
            let events = events.clone();
            move |local: &std::path::Path, remote: &str| -> Result<(), String> {
                events.borrow_mut().push(format!(
                    "put {} {remote}",
                    local.file_name().unwrap().to_string_lossy()
                ));
                Ok(())
            }
        };
        upload_tree(&root, "/dest", &mut mkdir, &mut put).unwrap();
        assert_eq!(
            *events.borrow(),
            [
                "mkdir /dest",
                "put a.txt /dest/a.txt",
                "mkdir /dest/sub",
                "put b.txt /dest/sub/b.txt",
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Against a real server: `MTTY_FTP_TEST_URL=ftp://user@host:port` with
    /// `MTTY_FTP_TEST_PASSWORD`; `MTTY_FTP_TEST_INSECURE=1` for self-signed
    /// FTPS. Round-trips a file through a scratch folder.
    #[test]
    fn a_real_round_trip() {
        let Ok(url) = std::env::var("MTTY_FTP_TEST_URL") else {
            return;
        };
        let mut remote = Remote::parse(&url).unwrap();
        remote.password = std::env::var("MTTY_FTP_TEST_PASSWORD").ok();
        remote.insecure = std::env::var_os("MTTY_FTP_TEST_INSECURE").is_some();
        let home = remote.home().unwrap();
        let dir = join(&home, &format!("mtty ftp test {}", std::process::id()));
        remote.mkdir(&dir).unwrap();
        let local = std::env::temp_dir().join(format!("mtty-ftp-{}", std::process::id()));
        std::fs::create_dir_all(&local).unwrap();
        let name = "a file 中文.txt";
        std::fs::write(local.join(name), "hello ftp\n").unwrap();
        remote.upload(&local.join(name), &dir).unwrap();
        let listed = remote.list(&dir).unwrap();
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!((listed[0].name.as_str(), listed[0].size), (name, 10));
        let renamed = join(&dir, "renamed 文件.txt");
        remote.rename(&join(&dir, name), &renamed).unwrap();
        let back = local.join("back");
        std::fs::create_dir_all(&back).unwrap();
        remote.download(&renamed, false, &back).unwrap();
        assert_eq!(
            std::fs::read_to_string(back.join("renamed 文件.txt")).unwrap(),
            "hello ftp\n"
        );
        remote.remove(&renamed, false).unwrap();
        let tree = local.join("tree");
        std::fs::create_dir_all(tree.join("nested")).unwrap();
        std::fs::write(tree.join("top.txt"), "top\n").unwrap();
        std::fs::write(tree.join("nested/deep.txt"), "deep\n").unwrap();
        remote.upload(&tree, &dir).unwrap();
        let sub = join(&dir, "tree");
        let names: Vec<_> = remote
            .list(&sub)
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(
            names,
            [("nested".to_string(), true), ("top.txt".to_string(), false)]
        );
        let tree_back = local.join("tree-back");
        std::fs::create_dir_all(&tree_back).unwrap();
        remote.download(&sub, true, &tree_back).unwrap();
        assert_eq!(
            std::fs::read_to_string(tree_back.join("tree/nested/deep.txt")).unwrap(),
            "deep\n"
        );
        remote.remove(&join(&sub, "top.txt"), false).unwrap();
        remote
            .remove(&join(&sub, "nested/deep.txt"), false)
            .unwrap();
        remote.remove(&join(&sub, "nested"), true).unwrap();
        remote.remove(&sub, true).unwrap();
        remote.remove(&dir, true).unwrap();
        assert!(remote
            .list(&home)
            .unwrap()
            .iter()
            .all(|e| !e.name.starts_with("mtty ftp test")));
        std::fs::remove_dir_all(&local).unwrap();
    }
}

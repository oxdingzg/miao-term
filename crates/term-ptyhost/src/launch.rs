//! Starting hosts and finding them again (ADR 0041 §1, §5, §6).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::host::HostArgs;

/// The host binary's name (plus `.exe` on Windows, see [`binary_name`]).
pub const BINARY: &str = "mtty-ptyhost";

/// The host binary's file name on this platform.
pub fn binary_name() -> String {
    format!("{BINARY}{}", std::env::consts::EXE_SUFFIX)
}

/// Socket paths are limited to ~104 bytes on macOS (108 on Linux).
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 100;

/// The private directory for host sockets and metadata, created `0700`:
/// `$XDG_RUNTIME_DIR/mtty-hosts`, else `$TMPDIR/mtty-hosts`, else (when that
/// would make socket paths too long) `/tmp/mtty-hosts-<uid>`. On Windows,
/// `%TEMP%\mtty-hosts` under the user's profile, private by its ACL.
pub fn hosts_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    #[allow(unused_mut)]
    let mut dir = base.join("mtty-hosts");
    // "/<32 hex>.sock"
    #[cfg(unix)]
    if dir.as_os_str().len() + 38 > MAX_SOCKET_PATH {
        // SAFETY: getuid has no preconditions.
        dir = PathBuf::from(format!("/tmp/mtty-hosts-{}", unsafe { libc::getuid() }));
    }
    private_dir(&dir)?;
    Ok(dir)
}

/// Create `dir` for the user alone (Windows: the profile's ACL already is).
#[cfg(windows)]
fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Create `dir` as `0700`, or accept it when it already is ours and private.
/// The mode is set at creation, so no one ever sees it more open.
#[cfg(unix)]
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(dir)?;
            // SAFETY: getuid has no preconditions.
            let ours = meta.uid() == unsafe { libc::getuid() };
            if !meta.is_dir() || !ours || meta.mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is not a private directory", dir.display()),
                ));
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// A random host id: 32 hex digits.
pub fn new_id() -> io::Result<String> {
    crate::sys::random_hex()
}

/// An id is 32 hex digits; anything else (a tampered session file) is
/// refused before it becomes a path.
pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn socket_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.sock"))
}

pub fn meta_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// The host binary shipped next to the running executable.
pub fn bundled_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(binary_name());
    path.is_file().then_some(path)
}

/// Copy the host binary to `<data_dir>/ptyhost/<version>/` and return that
/// path. Hosts run from there: the install location can vanish (an AppImage
/// mount) or be replaced (an update) while they run. A copy is refreshed when
/// the source changed (development builds keep one version number); running
/// hosts keep the file they started from, since the copy is renamed into
/// place.
pub fn install(source: &Path, data_dir: &Path) -> io::Result<PathBuf> {
    let dir = data_dir.join("ptyhost").join(crate::VERSION);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(binary_name());
    let src_meta = std::fs::metadata(source)?;
    let fresh = std::fs::metadata(&dest)
        .is_ok_and(|d| d.len() == src_meta.len() && d.modified().ok() >= src_meta.modified().ok());
    if !fresh {
        let tmp = dir.join(format!("{BINARY}.{}.tmp", std::process::id()));
        std::fs::copy(source, &tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        if let Err(e) = std::fs::rename(&tmp, &dest) {
            let _ = std::fs::remove_file(&tmp);
            // Windows cannot replace a binary a running host uses (only a
            // development build changes it under one version): keep it.
            if !(cfg!(windows) && dest.exists()) {
                return Err(e);
            }
        }
    }
    Ok(dest)
}

/// Remove installed host versions that neither this build nor any running
/// host uses.
pub fn remove_unused_versions(data_dir: &Path, hosts_dir: &Path) {
    let root = data_dir.join("ptyhost");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let in_use: Vec<String> = std::fs::read_dir(hosts_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter(|meta| json_number(meta, "pid").is_some_and(alive_pid))
        .filter_map(|meta| json_string(&meta, "version"))
        .collect();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != crate::VERSION && !in_use.contains(&name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// A running host, from its metadata file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    pub id: String,
    pub socket: PathBuf,
    pub child_pid: u64,
    /// Seconds since the Unix epoch.
    pub started_at: u64,
    /// The program it was started with (the shell).
    pub program: String,
}

/// The hosts in `dir` whose process is still alive, oldest first.
pub fn running_hosts(dir: &Path) -> Vec<HostInfo> {
    let mut hosts: Vec<HostInfo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_string();
            if path.extension()? != "json" || !valid_id(&id) {
                return None;
            }
            let meta = std::fs::read_to_string(&path).ok()?;
            if !json_number(&meta, "pid").is_some_and(alive_pid) {
                return None;
            }
            Some(HostInfo {
                socket: socket_path(dir, &id),
                id,
                child_pid: json_number(&meta, "child_pid").unwrap_or(0),
                started_at: json_number(&meta, "started_at").unwrap_or(0),
                program: json_string(&meta, "program").unwrap_or_default(),
            })
        })
        .collect();
    hosts.sort_by_key(|h| h.started_at);
    hosts
}

fn alive_pid(pid: u64) -> bool {
    crate::sys::alive(pid)
}

/// `"key":123` in the host's own flat metadata.
fn json_number(json: &str, key: &str) -> Option<u64> {
    let rest = &json[json.find(&format!("\"{key}\":"))? + key.len() + 3..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// `"key":"value"` (no escapes: versions never have them).
fn json_string(json: &str, key: &str) -> Option<String> {
    let rest = &json[json.find(&format!("\"{key}\":\""))? + key.len() + 4..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Start a host detached from this process: its own session (Windows: no
/// console, outside the app's job when the job lets it go), no terminal,
/// `env` as its whole environment (the program inherits it).
pub fn spawn<'a>(
    binary: &Path,
    args: &HostArgs,
    env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> io::Result<()> {
    let env: Vec<(&str, &str)> = env.into_iter().collect();
    let command = || {
        let mut cmd = Command::new(binary);
        cmd.args(args.to_args())
            .env_clear()
            .envs(env.iter().copied())
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    };
    #[cfg(unix)]
    let mut child = {
        use std::os::unix::process::CommandExt;
        let mut cmd = command();
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn()?
    };
    #[cfg(windows)]
    let mut child = {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Not CREATE_NEW_PROCESS_GROUP: it disables Ctrl+C in the new
        // process, and the shell and its programs would inherit that.
        let flags = DETACHED_PROCESS;
        // A job that does not allow breaking away refuses the flag: then the
        // host stays in it (and ends with the app if the job says so).
        match command()
            .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => command().creation_flags(flags).spawn()?,
        }
    };
    // Reap it whenever it ends, so it never lingers as a zombie of ours.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_hex_and_checked() {
        let id = new_id().unwrap();
        assert!(valid_id(&id), "{id}");
        assert_ne!(id, new_id().unwrap());
        assert!(!valid_id("../../etc/passwd0000000000000000000"));
        assert!(!valid_id("abc"));
    }

    #[test]
    fn running_hosts_are_listed_from_their_metadata() {
        let dir = std::env::temp_dir().join(format!("mtty-hosts-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let alive = "0123456789abcdef0123456789abcdef";
        let gone = "fedcba9876543210fedcba9876543210";
        let me = std::process::id();
        std::fs::write(
            meta_path(&dir, alive),
            format!(
                r#"{{"version":"x","pid":{me},"child_pid":7,"started_at":5,"program":"/bin/zsh"}}"#
            ),
        )
        .unwrap();
        // A pid that cannot exist.
        std::fs::write(meta_path(&dir, gone), r#"{"version":"x","pid":999999999}"#).unwrap();
        std::fs::write(dir.join("not-an-id.json"), format!(r#"{{"pid":{me}}}"#)).unwrap();
        let hosts = running_hosts(&dir);
        assert_eq!(hosts.len(), 1, "{hosts:?}");
        assert_eq!(hosts[0].id, alive);
        assert_eq!(hosts[0].program, "/bin/zsh");
        assert_eq!(hosts[0].child_pid, 7);
        assert_eq!(hosts[0].socket, socket_path(&dir, alive));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn metadata_fields() {
        let meta = r#"{"version":"0.0.20","proto":1,"pid":42,"socket":"/x"}"#;
        assert_eq!(json_number(meta, "pid"), Some(42));
        assert_eq!(json_string(meta, "version").as_deref(), Some("0.0.20"));
        assert_eq!(json_number(meta, "missing"), None);
    }

    #[cfg(unix)]
    #[test]
    fn installs_a_versioned_copy_and_refreshes_it() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!("mtty-ptyhost-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src-bin");
        std::fs::write(&src, b"v1").unwrap();
        let dest = install(&src, &root).unwrap();
        assert_eq!(
            dest,
            root.join("ptyhost")
                .join(crate::VERSION)
                .join(binary_name())
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"v1");
        assert_eq!(std::fs::metadata(&dest).unwrap().mode() & 0o777, 0o755);
        std::fs::write(&src, b"v22").unwrap();
        install(&src, &root).unwrap();
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"v22",
            "a changed source is copied again"
        );
        std::fs::create_dir_all(root.join("ptyhost").join("0.0.1")).unwrap();
        remove_unused_versions(&root, &root.join("no-hosts"));
        assert!(!root.join("ptyhost").join("0.0.1").exists());
        assert!(dest.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_directories_are_enforced() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("mtty-hosts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        private_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        private_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            private_dir(&dir).is_err(),
            "a readable directory is refused"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

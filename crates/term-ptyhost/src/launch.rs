//! Starting hosts and finding them again (ADR 0041 §1, §5, §6; Unix).

use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::host::HostArgs;

/// The host binary's file name.
pub const BINARY: &str = "mtty-ptyhost";

/// Socket paths are limited to ~104 bytes on macOS (108 on Linux).
const MAX_SOCKET_PATH: usize = 100;

/// The private directory for host sockets and metadata, created `0700`:
/// `$XDG_RUNTIME_DIR/mtty-hosts`, else `$TMPDIR/mtty-hosts`, else (when that
/// would make socket paths too long) `/tmp/mtty-hosts-<uid>`.
pub fn hosts_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let mut dir = base.join("mtty-hosts");
    // "/<32 hex>.sock"
    if dir.as_os_str().len() + 38 > MAX_SOCKET_PATH {
        // SAFETY: getuid has no preconditions.
        dir = PathBuf::from(format!("/tmp/mtty-hosts-{}", unsafe { libc::getuid() }));
    }
    private_dir(&dir)?;
    Ok(dir)
}

/// Create `dir` as `0700`, or accept it when it already is ours and private.
/// The mode is set at creation, so no one ever sees it more open.
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
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
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
    let path = exe.parent()?.join(BINARY);
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
    let dest = dir.join(BINARY);
    let src_meta = std::fs::metadata(source)?;
    let fresh = std::fs::metadata(&dest)
        .is_ok_and(|d| d.len() == src_meta.len() && d.modified().ok() >= src_meta.modified().ok());
    if !fresh {
        let tmp = dir.join(format!("{BINARY}.{}.tmp", std::process::id()));
        std::fs::copy(source, &tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &dest)?;
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

fn alive_pid(pid: u64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
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

/// Start a host detached from this process: its own session, no terminal,
/// `env` as its whole environment (the program inherits it).
pub fn spawn<'a>(
    binary: &Path,
    args: &HostArgs,
    env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> io::Result<()> {
    let mut cmd = Command::new(binary);
    cmd.args(args.to_args())
        .env_clear()
        .envs(env)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
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
    fn metadata_fields() {
        let meta = r#"{"version":"0.0.20","proto":1,"pid":42,"socket":"/x"}"#;
        assert_eq!(json_number(meta, "pid"), Some(42));
        assert_eq!(json_string(meta, "version").as_deref(), Some("0.0.20"));
        assert_eq!(json_number(meta, "missing"), None);
    }

    #[test]
    fn installs_a_versioned_copy_and_refreshes_it() {
        let root = std::env::temp_dir().join(format!("mtty-ptyhost-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src-bin");
        std::fs::write(&src, b"v1").unwrap();
        let dest = install(&src, &root).unwrap();
        assert_eq!(dest, root.join("ptyhost").join(crate::VERSION).join(BINARY));
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

    #[test]
    fn private_directories_are_enforced() {
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

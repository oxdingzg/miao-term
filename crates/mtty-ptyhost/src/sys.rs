//! What differs between platforms (ADR 0041; architecture D6): the socket
//! type, who may connect, and how processes are detached and ended.
//!
//! Windows uses AF_UNIX sockets too (Windows 10 1803+, below ConPTY's own
//! 1809 floor): they are full duplex, so one thread can read while another
//! writes, which synchronous named pipes do not allow on a single handle.

use std::io;
use std::path::Path;

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};
#[cfg(windows)]
pub use uds_windows::{UnixListener as Listener, UnixStream as Stream};

/// The peer runs as the same user. On Windows the socket lives in the
/// user's own temporary directory, whose ACL keeps other users out.
pub fn same_user(stream: &Stream) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::io::AsRawFd;
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred`/`len` describe a valid buffer for SO_PEERCRED.
        let ok = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        } == 0;
        // SAFETY: getuid has no preconditions.
        ok && cred.uid == unsafe { libc::getuid() }
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    {
        use std::os::unix::io::AsRawFd;
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: getpeereid writes the two ids on success.
        let ok = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0;
        // SAFETY: getuid has no preconditions.
        ok && uid == unsafe { libc::getuid() }
    }
    #[cfg(windows)]
    {
        let _ = stream;
        true
    }
}

/// Owner-only access to a file the host creates (Unix; Windows relies on the
/// directory's ACL).
pub fn owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    let _ = path;
}

/// Leave the app's session and ignore its hang-up (Unix). A Windows host is
/// created detached and outside the app's job instead (see `launch::spawn`);
/// it turns Ctrl+C handling back on, which processes inherit, so Ctrl+C
/// typed in the pane reaches the shell's programs even if whoever started
/// mtty had it off.
pub fn detach_self() {
    #[cfg(unix)]
    // SAFETY: plain libc calls without preconditions.
    unsafe {
        libc::setsid();
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    #[cfg(windows)]
    // SAFETY: a null handler with FALSE only clears the ignore flag.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 0);
    }
}

/// End `pid` and what it started: `SIGHUP` to its process group (it leads
/// its own session), then `SIGKILL` after `grace`; on Windows the whole tree
/// at once (ADR 0027).
pub fn hang_up(pid: u32, grace: std::time::Duration) {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return;
        };
        if pid <= 0 {
            return;
        }
        // SAFETY: plain signal delivery to a known process group.
        unsafe {
            libc::kill(-pid, libc::SIGHUP);
            libc::kill(pid, libc::SIGHUP);
        }
        std::thread::spawn(move || {
            std::thread::sleep(grace);
            // SAFETY: as above.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = grace;
        if pid == 0 {
            return;
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
}

/// Whether process `pid` is running.
pub fn alive(pid: u64) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists.
        pid > 0 && unsafe { libc::kill(pid, 0) } == 0
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let Ok(pid) = u32::try_from(pid) else {
            return false;
        };
        if pid == 0 {
            return false;
        }
        // SAFETY: the handle is checked and closed; the exit code is written
        // into a local.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            ok && code == STILL_ACTIVE as u32
        }
    }
}

/// 128 random bits as 32 hex digits, for host ids.
pub fn random_hex() -> io::Result<String> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(windows)]
    {
        // `RandomState` keys come from the OS generator; two hashers of
        // fresh states give 128 bits. Ids name sockets in a private
        // directory, so they need to be unique, not secret.
        use std::hash::{BuildHasher, Hasher};
        let half = || {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            h.write_u32(std::process::id());
            h.finish()
        };
        Ok(format!("{:016x}{:016x}", half(), half()))
    }
}

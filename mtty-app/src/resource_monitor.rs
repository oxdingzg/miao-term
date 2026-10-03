//! Bounded, durable process samples. No terminal contents or command arguments.
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INTERVAL: Duration = Duration::from_secs(30);
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RUNS: usize = 16;

pub fn start() {
    if std::env::var("MTTY_MONITOR").as_deref() == Ok("0") {
        return;
    }
    let Some(dir) =
        super::panic_log::default_path().and_then(|p| p.parent().map(|p| p.join("monitor")))
    else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("resource-monitor".into())
        .spawn(move || record(dir));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn record(dir: PathBuf) {
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("process-{}-{}.jsonl", now_ms(), std::process::id()));
    let _ = append(
        &path,
        &json!({
            "type": "start", "schema": 1, "t": now_ms(), "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"), "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH, "intervalMs": INTERVAL.as_millis(),
            "logicalCpus": std::thread::available_parallelism().ok().map(|n| n.get()),
        }),
    );
    prune(&dir);
    let start = Instant::now();
    let mut previous: Option<(Instant, u64)> = None;
    loop {
        let sampled = Instant::now();
        let mut sample = metrics();
        let cpu = sample["cpuTotalUs"].as_u64();
        let percent = previous.zip(cpu).map(|((when, old), current)| {
            current.saturating_sub(old) as f64
                / sampled.duration_since(when).as_secs_f64()
                / 10_000.0
        });
        previous = cpu.map(|cpu| (sampled, cpu));
        let (frames, render_us) = miao_term_widget::resource_metrics::snapshot();
        sample["type"] = json!("sample");
        sample["t"] = json!(now_ms());
        sample["pid"] = json!(std::process::id());
        sample["uptimeSecs"] = json!(start.elapsed().as_secs());
        sample["cpuPercent"] = json!(percent);
        sample["renderCalls"] = json!(frames);
        sample["renderWallUs"] = json!(render_us);
        let _ = append(&path, &sample);
        std::thread::sleep(INTERVAL);
    }
}

fn append(path: &Path, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    append_record(path, &bytes, MAX_BYTES, 64 * 1024 * 1024)
}

struct Lease(PathBuf);

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn lease(dir: &Path) -> io::Result<Lease> {
    let lock = dir.join(".diagnostic-write-lock");
    if let Err(error) = fs::create_dir(&lock) {
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        let owner = fs::read_to_string(lock.join("pid"))
            .ok()
            .and_then(|s| s.parse::<u32>().ok());
        let stale = owner.map(|pid| !process_alive(pid)).unwrap_or_else(|| {
            fs::metadata(&lock)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(60))
        });
        if !stale {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "diagnostic writer busy",
            ));
        }
        fs::remove_dir_all(&lock)?;
        fs::create_dir(&lock)?;
    }
    let guard = Lease(lock);
    fs::write(guard.0.join("pid"), std::process::id().to_string())?;
    Ok(guard)
}

/// Append under a cross-process lease, with a strict managed-directory budget.
/// If only active logs remain and the budget is full, drop this record.
pub(crate) fn append_record(
    path: &Path,
    bytes: &[u8],
    max_file: u64,
    max_total: u64,
) -> io::Result<()> {
    if bytes.len() as u64 > max_file {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "diagnostic record exceeds disk quota",
        ));
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let _lease = lease(dir)?;
    if fs::metadata(path).is_ok_and(|m| m.len().saturating_add(bytes.len() as u64) > max_file) {
        let backup = path.with_extension("previous.jsonl");
        // Windows rename does not replace an existing destination.
        match fs::remove_file(&backup) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fs::rename(path, backup)?;
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let managed = if path.file_name().and_then(|n| n.to_str()) == Some("panic.log") {
            name == "panic.log" || name == "panic.previous.jsonl"
        } else {
            name.starts_with("process-") && name.ends_with(".jsonl")
        };
        if !managed || !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
        let pid = name
            .strip_suffix(".previous.jsonl")
            .or_else(|| name.strip_suffix(".jsonl"))
            .and_then(|n| n.rsplit('-').next())
            .and_then(|n| n.parse::<u32>().ok());
        let removable = entry.path() != path
            && (name.ends_with(".previous.jsonl") || pid.is_some_and(|p| !process_alive(p)));
        files.push((
            entry.path(),
            metadata.len(),
            metadata.modified()?,
            removable,
        ));
    }
    files.sort_by_key(|f| f.2);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    for (file, size, _, removable) in files {
        if total.saturating_add(bytes.len() as u64) <= max_total {
            break;
        }
        if !removable {
            continue;
        }
        fs::remove_file(file)?;
        total = total.saturating_sub(size);
    }
    if total.saturating_add(bytes.len() as u64) > max_total {
        return Err(io::Error::other("diagnostic directory disk quota exceeded"));
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("process-")
                    && n.ends_with(".jsonl")
                    && !n.ends_with(".previous.jsonl")
                    && n.strip_suffix(".jsonl")
                        .and_then(|n| n.rsplit('-').next())
                        .and_then(|pid| pid.parse::<u32>().ok())
                        .is_some_and(|pid| !process_alive(pid))
            })
        })
        .collect();
    files.sort();
    let remove = files.len().saturating_sub(MAX_RUNS);
    for path in files.into_iter().take(remove) {
        let _ = fs::remove_file(path.with_extension("previous.jsonl"));
        let _ = fs::remove_file(path);
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal zero checks existence without sending a signal.
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the owned process handle is closed after the query. Inaccessible
    // processes are conservatively retained.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0;
        let alive = GetExitCodeProcess(handle, &mut code) == 0 || code == 259;
        CloseHandle(handle);
        alive
    }
}

#[cfg(not(any(unix, windows)))]
fn process_alive(_: u32) -> bool {
    true
}

#[cfg(unix)]
fn metrics() -> Value {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the struct on success; it is read only then.
    let cpu = unsafe {
        if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) == 0 {
            let u = usage.assume_init();
            Some(
                (u.ru_utime.tv_sec as u64 + u.ru_stime.tv_sec as u64) * 1_000_000
                    + u.ru_utime.tv_usec as u64
                    + u.ru_stime.tv_usec as u64,
            )
        } else {
            None
        }
    };
    let (rss, threads, fds) = unix_memory();
    json!({"cpuTotalUs": cpu, "rssBytes": rss, "threads": threads, "fds": fds})
}

#[cfg(target_os = "macos")]
fn unix_memory() -> (Option<u64>, Option<u64>, Option<usize>) {
    // ps queries this process only; it runs on the recorder thread, not the UI.
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok();
    let rss = output.filter(|o| o.status.success()).and_then(|o| {
        String::from_utf8(o.stdout)
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?
            .checked_mul(1024)
    });
    let threads = std::process::Command::new("/bin/ps")
        .args(["-M", &std::process::id().to_string()])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().count().saturating_sub(1) as u64);
    (rss, threads, None)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unix_memory() -> (Option<u64>, Option<u64>, Option<usize>) {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |key: &str| -> Option<u64> {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    (
        field("VmRSS:").and_then(|v| v.checked_mul(1024)),
        field("Threads:"),
        fs::read_dir("/proc/self/fd").ok().map(|d| d.count()),
    )
}

#[cfg(windows)]
fn metrics() -> Value {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    // SAFETY: current-process handle is borrowed; API output buffers have their
    // documented sizes and are read only after the respective call succeeds.
    unsafe {
        let process = GetCurrentProcess();
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let us =
            |t: FILETIME| ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) / 10;
        let cpu = (GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0)
            .then(|| us(kernel) + us(user));
        let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        let size = std::mem::size_of_val(&mem) as u32;
        mem.cb = size;
        let rss =
            (GetProcessMemoryInfo(process, &mut mem, size) != 0).then_some(mem.WorkingSetSize);
        json!({"cpuTotalUs": cpu, "rssBytes": rss, "threads": null, "fds": null})
    }
}

#[cfg(not(any(unix, windows)))]
fn metrics() -> Value {
    json!({"cpuTotalUs": null, "rssBytes": null, "threads": null, "fds": null})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_latest_records_and_one_backup() {
        let dir =
            std::env::temp_dir().join(format!("mtty-monitor-{}-{}", std::process::id(), now_ms()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("process-test.jsonl");
        fs::write(&path, vec![b' '; MAX_BYTES as usize]).unwrap();
        append(&path, &json!({"sequence": 1})).unwrap();
        let backup = path.with_extension("previous.jsonl");
        assert_eq!(fs::metadata(&backup).unwrap().len(), MAX_BYTES);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"sequence\":1}\n");
        fs::write(&path, vec![b' '; MAX_BYTES as usize]).unwrap();
        append(&path, &json!({"sequence": 2})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"sequence\":2}\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retention_preserves_live_process_and_unrelated_files() {
        let dir = std::env::temp_dir().join(format!(
            "mtty-retention-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join(format!("process-000-{}.jsonl", std::process::id()));
        fs::write(&live, "live").unwrap();
        fs::write(dir.join("unrelated.jsonl"), "keep").unwrap();
        // A PID outside the supported OS PID ranges cannot designate a live process.
        for i in 0..MAX_RUNS + 3 {
            let path = dir.join(format!("process-{i:03}-2000000000.jsonl"));
            fs::write(&path, "old").unwrap();
            fs::write(path.with_extension("previous.jsonl"), "backup").unwrap();
        }
        prune(&dir);
        assert!(live.exists());
        assert!(dir.join("unrelated.jsonl").exists());
        assert!(!dir.join("process-000-2000000000.jsonl").exists());
        assert!(!dir.join("process-000-2000000000.previous.jsonl").exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), MAX_RUNS * 2 + 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_quota_removes_old_logs_and_rejects_oversized_records() {
        let dir =
            std::env::temp_dir().join(format!("mtty-quota-{}-{}", std::process::id(), now_ms()));
        fs::create_dir_all(&dir).unwrap();
        let old = dir.join("process-000-2000000000.jsonl");
        fs::write(&old, vec![b'x'; 100]).unwrap();
        fs::write(dir.join("unrelated.txt"), "keep").unwrap();
        let path = dir.join(format!("process-001-{}.jsonl", std::process::id()));
        append_record(&path, b"new\n", 8, 16).unwrap();
        assert!(!old.exists());
        assert_eq!(
            fs::read_to_string(dir.join("unrelated.txt")).unwrap(),
            "keep"
        );
        assert!(append_record(&path, b"oversized", 8, 16).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_budget_preserves_active_logs_and_drops_new_record() {
        let dir = std::env::temp_dir().join(format!(
            "mtty-active-quota-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join(format!("process-000-{}.jsonl", std::process::id()));
        fs::write(&live, b"1234567890123456").unwrap();
        let next = dir.join(format!("process-001-{}.jsonl", std::process::id()));
        assert!(append_record(&next, b"new", 8, 16).is_err());
        assert!(!next.exists());
        assert_eq!(fs::read_to_string(&live).unwrap(), "1234567890123456");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn panic_log_rotation_is_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "mtty-panic-quota-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let path = dir.join("panic.log");
        append_record(&path, b"12345678", 8, 16).unwrap();
        append_record(&path, b"abcdefgh", 8, 16).unwrap();
        append_record(&path, b"latest", 8, 16).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "latest");
        assert_eq!(
            fs::read_to_string(path.with_extension("previous.jsonl")).unwrap(),
            "abcdefgh"
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_sample_has_process_cpu_and_memory() {
        let sample = metrics();
        assert!(sample["cpuTotalUs"].as_u64().is_some());
        assert!(sample["rssBytes"].as_u64().unwrap() > 0);
    }
}

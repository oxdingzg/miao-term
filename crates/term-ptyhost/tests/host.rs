//! The real host binary, end to end (ADR 0041 P1).
#![cfg(unix)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use miao_term_ptyhost::client;
use miao_term_ptyhost::host::HostArgs;
use miao_term_ptyhost::launch;
use miao_term_ptyhost::proto::{FromHost, ToHost};
use miao_term_ptyhost::scanner::Modes;

struct Host {
    dir: PathBuf,
    socket: PathBuf,
    meta: PathBuf,
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Ok((mut s, _)) = client::connect(&self.socket, Duration::from_millis(200)) {
            let _ = ToHost::Kill.write(&mut s);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start(script: &str, ring: usize, timeout_secs: u64) -> Host {
    // Short paths: socket paths are limited to ~100 bytes.
    let dir = PathBuf::from(format!(
        "/tmp/mph-{}-{}",
        std::process::id(),
        launch::new_id().unwrap().get(..8).unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    let meta = dir.join("h.json");
    let args = HostArgs {
        socket: socket.clone(),
        meta: Some(meta.clone()),
        cols: 40,
        rows: 10,
        ring,
        timeout: Duration::from_secs(timeout_secs),
        cwd: None,
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
    };
    let env: Vec<(String, String)> = std::env::vars().collect();
    launch::spawn(
        Path::new(env!("CARGO_BIN_EXE_mtty-ptyhost")),
        &args,
        env.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    )
    .unwrap();
    Host { dir, socket, meta }
}

fn connect(host: &Host) -> (UnixStream, client::Welcome) {
    client::connect(&host.socket, Duration::from_secs(5)).unwrap()
}

/// Frames until `done` says so (or a 5 s deadline).
fn read_until(s: &mut UnixStream, mut done: impl FnMut(&[FromHost]) -> bool) -> Vec<FromHost> {
    // macOS refuses socket options once the peer has shut the socket down;
    // the frames already received can still be read.
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut frames = Vec::new();
    while !done(&frames) {
        frames.push(FromHost::read(s).expect("a frame before the deadline"));
    }
    frames
}

fn output(frames: &[FromHost]) -> String {
    frames
        .iter()
        .filter_map(|f| match f {
            FromHost::Output { bytes, .. } => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect()
}

fn live(frames: &[FromHost]) -> Option<u64> {
    frames.iter().find_map(|f| match f {
        FromHost::Live { offset } => Some(*offset),
        _ => None,
    })
}

fn gone(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

#[test]
fn attach_type_detach_and_resume_from_an_offset() {
    let host = start("echo hello; exec cat", 8 << 20, 60);
    let (mut s, welcome) = connect(&host);
    assert!(welcome.child_pid > 0);
    assert!(std::fs::read_to_string(&host.meta)
        .unwrap()
        .contains("\"child_pid\""));
    ToHost::Attach { from: 0 }.write(&mut s).unwrap();
    let frames = read_until(&mut s, |f| output(f).contains("hello") && live(f).is_some());
    let after_hello: u64 = frames
        .iter()
        .filter_map(|f| match f {
            FromHost::Output { offset, bytes, .. } => Some(offset + bytes.len() as u64),
            _ => None,
        })
        .max()
        .unwrap();
    ToHost::Input(b"abc\n".to_vec()).write(&mut s).unwrap();
    // The terminal echoes the line and cat prints it again.
    read_until(&mut s, |f| output(f).matches("abc").count() >= 2);
    ToHost::Detach.write(&mut s).unwrap();
    drop(s);

    // The program kept running: type while nobody watches, via a new client.
    let (mut s, _) = connect(&host);
    ToHost::Attach { from: after_hello }.write(&mut s).unwrap();
    let frames = read_until(&mut s, |f| live(f).is_some());
    let replay = output(&frames);
    assert!(
        !replay.contains("hello"),
        "only what came after: {replay:?}"
    );
    assert_eq!(replay.matches("abc").count(), 2, "{replay:?}");
    assert!(frames
        .iter()
        .all(|f| !matches!(f, FromHost::Truncated { .. })));

    ToHost::Kill.write(&mut s).unwrap();
    assert!(gone(&host.socket), "the host ends after Kill");
    assert!(gone(&host.meta));
}

#[test]
fn a_short_ring_reports_truncation_with_the_modes() {
    // Output beyond one frame (`MAX_BATCH`, 256 KiB), so the host splits it and
    // the 128-byte ring must evict the oldest. The final line stays short so
    // the replay's tail still names it; a shorter burst can arrive as a single
    // frame, which the ring always keeps whole, and used to make this race the
    // reader's chunking (it failed on a loaded macOS runner).
    let host = start(
        "printf '\\033[?2004h'; i=0; while [ $i -lt 59 ]; do \
         printf 'line-%s %05000d\\n' \"$i\" 0; i=$((i+1)); done; \
         echo line-59; exec cat",
        128,
        60,
    );
    // Attach from 0 until the ring has evicted; then it reports the truncation
    // first and the replay still reaches `line-59`.
    let deadline = Instant::now() + Duration::from_secs(10);
    let frames = loop {
        let (mut s, _) = connect(&host);
        ToHost::Attach { from: 0 }.write(&mut s).unwrap();
        let frames = read_until(&mut s, |f| live(f).is_some());
        if matches!(frames.first(), Some(FromHost::Truncated { .. }))
            && output(&frames).contains("line-59")
        {
            break frames;
        }
        assert!(
            Instant::now() < deadline,
            "no truncation reaching line-59 within the deadline; first frame: {:?}",
            frames.first()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let FromHost::Truncated { oldest, modes } = &frames[0] else {
        unreachable!("checked above")
    };
    assert!(*oldest > 0);
    assert_eq!(Modes::decode(modes).unwrap().get(2004), Some(true));
    match &frames[1] {
        FromHost::Output { offset, .. } => assert_eq!(offset, oldest),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_exit_while_detached_is_delivered_then_the_host_ends() {
    let host = start("echo bye; exit 3", 8 << 20, 60);
    std::thread::sleep(Duration::from_millis(300));
    let (mut s, _) = connect(&host);
    ToHost::Attach { from: 0 }.write(&mut s).unwrap();
    let frames = read_until(&mut s, |f| {
        f.iter().any(|f| matches!(f, FromHost::Exited { .. }))
    });
    assert!(output(&frames).contains("bye"));
    assert!(frames
        .iter()
        .any(|f| matches!(f, FromHost::Exited { status: 3, .. })));
    drop(s);
    assert!(
        gone(&host.socket),
        "it ends once a client has seen the exit"
    );
}

#[test]
fn nobody_returning_ends_the_program_after_the_timeout() {
    let host = start("exec sleep 300", 8 << 20, 1);
    let (mut s, welcome) = connect(&host);
    ToHost::Attach { from: 0 }.write(&mut s).unwrap();
    read_until(&mut s, |f| live(f).is_some());
    drop(s);
    assert!(gone(&host.socket), "ends after the detach timeout");
    std::thread::sleep(Duration::from_millis(600));
    // SAFETY: signal 0 only checks existence.
    let alive = unsafe { libc::kill(welcome.child_pid as i32, 0) } == 0;
    assert!(!alive, "the program was hung up");
}

#[test]
fn a_second_client_takes_over() {
    let host = start("exec cat", 8 << 20, 60);
    let (mut a, _) = connect(&host);
    ToHost::Attach { from: 0 }.write(&mut a).unwrap();
    read_until(&mut a, |f| live(f).is_some());
    let (mut b, _) = connect(&host);
    ToHost::Attach { from: 0 }.write(&mut b).unwrap();
    read_until(&mut b, |f| live(f).is_some());
    read_until(&mut a, |f| {
        f.iter().any(|f| matches!(f, FromHost::Detached))
    });
    ToHost::Input(b"x\n".to_vec()).write(&mut b).unwrap();
    read_until(&mut b, |f| output(f).contains('x'));
}

#[test]
fn output_frames_end_on_sequence_boundaries() {
    let host = start(
        "printf 'a\\033[3'; sleep 0.3; printf '1mX\\342\\200'; sleep 0.3; printf '\\242!\\n'; exec cat",
        8 << 20,
        60,
    );
    let (mut s, _) = connect(&host);
    ToHost::Attach { from: 0 }.write(&mut s).unwrap();
    let frames = read_until(&mut s, |f| output(f).contains('!'));
    let mut text = Vec::new();
    for f in &frames {
        if let FromHost::Output {
            offset,
            boundary,
            bytes,
        } = f
        {
            assert!(boundary, "a frame ended mid-sequence: {bytes:?}");
            assert_eq!(*offset, text.len() as u64, "contiguous");
            text.extend_from_slice(bytes);
        }
    }
    assert!(String::from_utf8_lossy(&text).contains("a\x1b[31mX\u{2022}!"));
}

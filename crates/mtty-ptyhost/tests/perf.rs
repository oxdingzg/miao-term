//! Performance gate for hosted panes (ADR 0041 P4; ADR 0018/0028).
//!
//! A hosted pane adds one socket hop to every keystroke's echo and to all
//! output. These measure that hop against the same work on a PTY the test
//! owns directly. `#[ignore]`d: CI runs them in release
//! (`cargo test --release -- --ignored`). `MTTY_PERF_SCALE` relaxes the
//! budgets on slow machines.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mtty_ptyhost::client;
use mtty_ptyhost::host::HostArgs;
use mtty_ptyhost::launch;
use mtty_ptyhost::proto::{FromHost, ToHost};
use mtty_ptyhost::sys::Stream;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

fn scale() -> f64 {
    std::env::var("MTTY_PERF_SCALE")
        .or_else(|_| std::env::var("MIAOTTY_PERF_SCALE"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

/// Record a measured metric for the CI baseline comparison (ADR 0028).
fn record_metric(key: &str, value: f64) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/perf-measured.json");
    let mut map: std::collections::BTreeMap<String, f64> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.insert(key.to_string(), value);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&map).unwrap_or_default(),
    );
}

struct Host {
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Ok((mut s, _)) = client::connect(&self.socket, Duration::from_millis(200)) {
            let _ = ToHost::Kill.write(&mut s);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The client's two ends, as the app holds them: frames are read through a
/// buffer, input written to the socket.
struct Conn {
    reader: std::io::BufReader<Stream>,
    writer: Stream,
}

fn start(script: &str) -> (Host, Conn) {
    let dir = PathBuf::from(format!(
        "/tmp/mpp-{}-{}",
        std::process::id(),
        launch::new_id().unwrap().get(..8).unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    let args = HostArgs {
        socket: socket.clone(),
        meta: None,
        cols: 100,
        rows: 30,
        ring: 8 << 20,
        timeout: Duration::from_secs(60),
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
    let (mut writer, _) = client::connect(&socket, Duration::from_secs(5)).unwrap();
    ToHost::Attach { from: 0 }.write(&mut writer).unwrap();
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, writer.try_clone().unwrap());
    loop {
        if let FromHost::Live { .. } = FromHost::read(&mut reader).unwrap() {
            break;
        }
    }
    (Host { dir, socket }, Conn { reader, writer })
}

/// Read output frames until `want` appears in the bytes seen since `seen`
/// was last cleared; returns the bytes read.
fn read_until(stream: &mut impl Read, seen: &mut Vec<u8>, want: &[u8]) -> usize {
    let mut total = 0;
    loop {
        if let Some(i) = find(seen, want) {
            seen.drain(..i + want.len());
            return total;
        }
        match FromHost::read(stream).unwrap() {
            FromHost::Output { bytes, .. } => {
                total += bytes.len();
                // Keep only a tail: the marker is short.
                seen.extend_from_slice(&bytes);
                if seen.len() > 4096 {
                    seen.drain(..seen.len() - 4096);
                }
            }
            FromHost::Exited { .. } => panic!("the program ended"),
            _ => {}
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

/// A direct PTY running `script`, for comparison.
fn direct(
    script: &str,
) -> (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    Box<dyn std::any::Any>,
) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.args(["-c", script]);
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let reader = pair.master.try_clone_reader().unwrap();
    let writer = pair.master.take_writer().unwrap();
    (reader, writer, Box::new((pair.master, child)))
}

fn read_direct_until(reader: &mut dyn Read, seen: &mut Vec<u8>, want: &[u8]) -> usize {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0;
    loop {
        if let Some(i) = find(seen, want) {
            seen.drain(..i + want.len());
            return total;
        }
        let n = reader.read(&mut buf).unwrap();
        assert!(n > 0, "the program ended");
        total += n;
        seen.extend_from_slice(&buf[..n]);
        if seen.len() > 4096 {
            seen.drain(..seen.len() - 4096);
        }
    }
}

const ECHOES: usize = 300;

/// A keystroke's echo through the host, against the same echo on a PTY the
/// test owns: the terminal echoes typed bytes at once (`cat` waits for the
/// line).
#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn hosted_echo_round_trip() {
    let script = "exec cat >/dev/null";
    let (_host, mut conn) = start(script);
    let mut seen = Vec::new();
    let mut hosted = Vec::with_capacity(ECHOES);
    for i in 0..ECHOES {
        let key = [b'a' + (i % 26) as u8];
        let t = Instant::now();
        ToHost::Input(key.to_vec()).write(&mut conn.writer).unwrap();
        read_until(&mut conn.reader, &mut seen, &key);
        hosted.push(t.elapsed().as_secs_f64() * 1e3);
        if i % 60 == 59 {
            // Keep the line short.
            ToHost::Input(b"\r".to_vec())
                .write(&mut conn.writer)
                .unwrap();
            read_until(&mut conn.reader, &mut seen, b"\n");
        }
    }
    let (mut reader, mut writer, _keep) = direct(script);
    let mut seen = Vec::new();
    let mut plain = Vec::with_capacity(ECHOES);
    for i in 0..ECHOES {
        let key = [b'a' + (i % 26) as u8];
        let t = Instant::now();
        writer.write_all(&key).unwrap();
        writer.flush().unwrap();
        read_direct_until(&mut *reader, &mut seen, &key);
        plain.push(t.elapsed().as_secs_f64() * 1e3);
        if i % 60 == 59 {
            writer.write_all(b"\r").unwrap();
            read_direct_until(&mut *reader, &mut seen, b"\n");
        }
    }
    let p95 = percentile(&mut hosted, 0.95);
    let p50 = percentile(&mut hosted, 0.5);
    let direct_p95 = percentile(&mut plain, 0.95);
    println!(
        "echo round trip: hosted p50 {p50:.3} ms, p95 {p95:.3} ms; direct p95 {direct_p95:.3} ms"
    );
    // The whole keystroke-to-glyph budget is 16 ms (P95, ARCHITECTURE §1);
    // the host's share must stay a small part of it.
    assert!(p95 < 4.0 * scale(), "hosted echo p95 {p95:.3} ms");
    let measured = p95 / scale();
    record_metric("hosted_echo_p95_ms", measured);
    mtty_core::perfgate::baseline_gate("hosted_echo_p95_ms", measured, false);
}

/// Output through the host, against the same output read from a PTY the
/// test owns.
#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn hosted_output_throughput() {
    let line: &[u8] = b"\x1b[31mhello\x1b[0m world \x1b[1;32mfoo\x1b[0m bar 1234567890\r\n";
    let mut data = Vec::with_capacity((16 << 20) + line.len());
    while data.len() < 16 << 20 {
        data.extend_from_slice(line);
    }
    let file = std::env::temp_dir().join(format!("mtty-perf-{}.txt", std::process::id()));
    std::fs::write(&file, &data).unwrap();
    // Each timed transfer begins only after the shell confirms it is ready.
    // The first transfer warms file caches, PTY buffers and the host path.
    let script = format!(
        "stty -echo; while read prepare; do echo READY; read go; cat '{}'; echo END-OF-RUN; done",
        file.display()
    );
    let (_host, mut conn) = start(&script);
    let (mut reader, mut writer, _keep) = direct(&script);
    let mut hosted_seen = Vec::new();
    let mut direct_seen = Vec::new();
    let mut hosted_samples = Vec::with_capacity(3);
    let mut direct_samples = Vec::with_capacity(3);
    for round in 0..4 {
        ToHost::Input(b"prepare\n".to_vec())
            .write(&mut conn.writer)
            .unwrap();
        read_until(&mut conn.reader, &mut hosted_seen, b"READY");
        let t = Instant::now();
        ToHost::Input(b"go\n".to_vec())
            .write(&mut conn.writer)
            .unwrap();
        let bytes = read_until(&mut conn.reader, &mut hosted_seen, b"END-OF-RUN");
        let hosted = bytes as f64 / 1e6 / t.elapsed().as_secs_f64();

        writer.write_all(b"prepare\n").unwrap();
        writer.flush().unwrap();
        read_direct_until(&mut *reader, &mut direct_seen, b"READY");
        let t = Instant::now();
        writer.write_all(b"go\n").unwrap();
        writer.flush().unwrap();
        let bytes = read_direct_until(&mut *reader, &mut direct_seen, b"END-OF-RUN");
        let plain = bytes as f64 / 1e6 / t.elapsed().as_secs_f64();
        if round > 0 {
            hosted_samples.push(hosted);
            direct_samples.push(plain);
        }
    }
    let _ = std::fs::remove_file(&file);
    let samples =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/perf-hosted-output-samples.json");
    std::fs::create_dir_all(samples.parent().unwrap()).unwrap();
    std::fs::write(
        samples,
        serde_json::to_string_pretty(&serde_json::json!({
            "hosted_mbps": hosted_samples,
            "direct_mbps": direct_samples,
            "warmup_rounds": 1,
        }))
        .unwrap(),
    )
    .unwrap();
    let hosted = percentile(&mut hosted_samples, 0.5);
    let plain = percentile(&mut direct_samples, 0.5);
    println!(
        "output median: hosted {hosted:.1} MB/s, direct {plain:.1} MB/s ({:.0}%)",
        hosted / plain * 100.0
    );
    // The screen parses at ≥ 25 MB/s (vt_parse_mbps): the host must not be
    // what limits a pane.
    assert!(hosted > 25.0 / scale(), "hosted output {hosted:.1} MB/s");
    let measured = hosted * scale();
    record_metric("hosted_output_mbps", measured);
    mtty_core::perfgate::baseline_gate("hosted_output_mbps", measured, true);
}

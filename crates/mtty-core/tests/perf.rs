//! Performance gate for the VT core (ADR 0018).
//!
//! These are `#[ignore]`d so the normal test run stays fast; CI runs them in
//! release mode (`cargo test --release -- --ignored`). Budgets are absolute with
//! generous headroom so an order-of-magnitude regression fails while runner
//! jitter does not. `MTTY_PERF_SCALE` (or the former `MIAOTTY_PERF_SCALE`) relaxes them on slow machines.

use std::time::Instant;

use mtty_core::aterm::ATerm;
use mtty_core::perfgate::baseline_gate;

fn scale() -> f64 {
    std::env::var("MTTY_PERF_SCALE")
        .or_else(|_| std::env::var("MIAOTTY_PERF_SCALE"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

fn workload() -> Vec<u8> {
    let line: &[u8] = b"\x1b[31mhello\x1b[0m world \x1b[1;32mfoo\x1b[0m bar 1234567890\r\n";
    let target = 16 << 20;
    let mut data = Vec::with_capacity(target + line.len());
    while data.len() < target {
        data.extend_from_slice(line);
    }
    data
}

/// Record a measured metric for the CI baseline comparison (ADR 0028).
fn record_metric(key: &str, value: f64) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/perf-measured.json");
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

#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn vt_parse_throughput() {
    let mut screen = ATerm::new(100, 30, 10_000);
    let data = workload();
    // Warm the caches, then measure.
    screen.process(&data[..64 * 1024]);
    let start = Instant::now();
    screen.process(&data);
    let secs = start.elapsed().as_secs_f64();
    let mbps = data.len() as f64 / 1e6 / secs;
    println!(
        "vt parse: {mbps:.1} MB/s ({} MB in {secs:.3}s)",
        data.len() >> 20
    );
    assert!(
        mbps >= 25.0 / scale(),
        "VT parse throughput {mbps:.1} MB/s below the 25 MB/s budget"
    );
    baseline_gate("vt_parse_mbps", mbps / scale(), true);
    record_metric("vt_parse_mbps", mbps / scale());
}

#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn screen_snapshot_budget() {
    let mut screen = ATerm::new(100, 30, 10_000);
    for _ in 0..30 {
        screen.process(b"\x1b[31mred\x1b[0m green blue text 1234567890\r\n");
    }
    let (rows, _) = screen.size();
    let n = 1000;
    let start = Instant::now();
    for _ in 0..n {
        let mut total = 0usize;
        for row in 0..rows {
            total += screen.line_text(row).len();
        }
        std::hint::black_box(total);
    }
    let per_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("snapshot {rows} rows: {per_ms:.4} ms");
    assert!(
        per_ms <= 2.0 * scale(),
        "screen snapshot {per_ms:.4} ms exceeds the 2 ms budget"
    );
    baseline_gate("screen_snapshot_30_rows_ms", per_ms / scale(), false);
    record_metric("screen_snapshot_30_rows_ms", per_ms / scale());
}

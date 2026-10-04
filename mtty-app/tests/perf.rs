//! Release performance gates for the shared paths used by mtty.
use mtty_core::ATerm;
use mtty_ui::{build_rows, UiTheme as Theme};
use std::time::Instant;

/// Record a measured metric for the CI baseline comparison (ADR 0028).
fn record_metric(key: &str, value: f64) {
    // Walk up to the workspace root (the dir holding Cargo.lock) so both
    // crates write the same file.
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = loop {
        if dir.join("Cargo.lock").is_file() {
            break dir.join("target/perf-measured.json");
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break dir.join("target/perf-measured.json"),
        }
    };
    let mut map: std::collections::BTreeMap<String, f64> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.insert(key.to_string(), value);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&map).unwrap_or_default(),
    );
}

fn scale() -> f64 {
    std::env::var("MTTY_PERF_SCALE")
        .or_else(|_| std::env::var("MIAOTTY_PERF_SCALE"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn build_rows_frame_budget() {
    let mut screen = ATerm::new(100, 30, 10_000);
    for _ in 0..40 {
        screen.process(b"\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m text 1234567890\r\n");
    }
    let theme = Theme::nord();
    let cursor = Some(screen.cursor());
    std::hint::black_box(build_rows(&screen, &theme, cursor));
    let n = 300;
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(build_rows(&screen, &theme, cursor));
    }
    let per_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("build_rows: {per_ms:.3} ms/frame");
    assert!(
        per_ms <= 4.0 * scale(),
        "row build {per_ms:.3} ms exceeds the 4 ms/frame budget"
    );
    mtty_core::perfgate::baseline_gate("build_rows_frame_ms", per_ms / scale(), false);
    record_metric("build_rows_frame_ms", per_ms / scale());
}

#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn palette_ranking_budget() {
    let entries: Vec<(String, String)> = (0..10_000)
        .map(|i| (format!("file-{i}.rs"), "file".to_string()))
        .collect();
    let n = 20;
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(rank_entries(&entries, "file-9"));
    }
    let per_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("palette rank over 10k entries: {per_ms:.3} ms");
    assert!(
        per_ms <= 100.0 * scale(),
        "palette ranking {per_ms:.3} ms exceeds the 100 ms budget"
    );
    mtty_core::perfgate::baseline_gate("palette_rank_10k_ms", per_ms / scale(), false);
    record_metric("palette_rank_10k_ms", per_ms / scale());
}

fn rank_entries(entries: &[(String, String)], query: &str) -> Vec<(usize, usize)> {
    let mut ranked: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, (label, kind))| {
            mtty_ui::palette::score(label, kind, query).map(|s| (s, i))
        })
        .collect();
    ranked.sort_by_key(|&(s, i)| (s, i));
    ranked
}

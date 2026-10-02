//! Performance gate for the editor core (ADR 0034, phase E1).
//!
//! `#[ignore]`d like the other perf gates; CI runs them in release mode
//! (`cargo test --release -- --ignored`). Budgets are absolute with generous
//! headroom; `MTTY_PERF_SCALE` relaxes them on slow machines.

use std::time::Instant;

use miao_term_editor::{Document, Motion, SearchQuery, Selection};

fn scale() -> f64 {
    std::env::var("MTTY_PERF_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

/// About 100 MB of source-like text.
fn big_file() -> Vec<u8> {
    let line = "    let value = compute(index, &table[offset..end]); // 中文注释 needle?\n";
    let target = 100 << 20;
    let mut out = String::with_capacity(target + line.len());
    while out.len() < target {
        out.push_str(line);
    }
    out.into_bytes()
}

fn check(name: &str, ms: f64, budget_ms: f64) {
    let budget = budget_ms * scale();
    eprintln!("{name}: {ms:.4} ms (budget {budget} ms)");
    assert!(
        ms <= budget,
        "{name} took {ms:.2} ms, budget {budget:.0} ms"
    );
}

#[test]
#[ignore = "perf gate; run `cargo test --release -- --ignored`"]
fn editing_a_100_mb_file_stays_interactive() {
    let bytes = big_file();

    let t = Instant::now();
    let mut doc = Document::from_bytes(&bytes).unwrap();
    check("open 100 MB", t.elapsed().as_secs_f64() * 1e3, 1500.0);

    // Type in the middle: every keystroke must be far below a frame.
    let middle = doc.rope().len_chars() / 2;
    doc.set_selection(Selection::cursor(middle));
    let t = Instant::now();
    for _ in 0..1000 {
        doc.type_text("x");
    }
    let per_key = t.elapsed().as_secs_f64() * 1e3 / 1000.0;
    check("keystroke in the middle", per_key, 1.0);

    let t = Instant::now();
    while doc.undo() {}
    check(
        "undo 1000 keystrokes",
        t.elapsed().as_secs_f64() * 1e3,
        50.0,
    );
    assert!(!doc.is_modified());

    // Cursor motion through the file.
    let t = Instant::now();
    for _ in 0..10_000 {
        doc.move_cursor(Motion::Down, false);
    }
    let per_move = t.elapsed().as_secs_f64() * 1e3 / 10_000.0;
    check("line down", per_move, 0.5);

    let t = Instant::now();
    let hits = miao_term_editor::search::find_all(doc.rope(), &SearchQuery::literal("needle?"))
        .unwrap()
        .len();
    check(
        "find all in 100 MB",
        t.elapsed().as_secs_f64() * 1e3,
        3000.0,
    );
    assert!(hits > 1_000_000);

    let t = Instant::now();
    let out = doc.to_bytes();
    check("save 100 MB", t.elapsed().as_secs_f64() * 1e3, 1000.0);
    assert_eq!(out.len(), bytes.len());
}

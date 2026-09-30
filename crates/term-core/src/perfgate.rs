//! Shared helper for the workspace's `#[ignore]`d performance gate (ADR 0018,
//! ADR 0028).
//!
//! The baseline committed in `benches/budgets.json` was measured on one machine
//! (an Apple M-series laptop), so comparing another machine — every CI runner —
//! against it is only meaningful as a *report*: the absolute budgets asserted in
//! the tests are the gate, exactly as ADR 0028 describes the CI comparison.
//! Set `PERF_ENFORCE=1` (the dev machine, or a stable runner) to also fail on a
//! regression beyond the metric's `regression_pct` — the same convention as
//! `scripts/check-perf-baseline.py`.

/// Compare `measured` against the committed baseline for `key` and print the
/// outcome; fail only when `PERF_ENFORCE` is set.
///
/// `measured` is expected to be normalised by `MIAOTTY_PERF_SCALE` already, so
/// a deliberately slowed machine is not compared against an unscaled baseline.
pub fn baseline_gate(key: &str, measured: f64, higher_is_better: bool) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benches/budgets.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(entry) = value.get(key) else { return };
    let Some(base) = entry.get("baseline").and_then(|b| b.as_f64()) else {
        return;
    };
    let pct = entry
        .get("regression_pct")
        .and_then(|p| p.as_f64())
        .unwrap_or(25.0);
    let limit = if higher_is_better {
        base * (1.0 - pct / 100.0)
    } else {
        base * (1.0 + pct / 100.0)
    };
    let ok = if higher_is_better {
        measured >= limit
    } else {
        measured <= limit
    };
    if ok {
        println!("{key}: {measured:.4} within {pct}% of baseline {base:.4}");
        return;
    }
    let enforce = std::env::var("PERF_ENFORCE")
        .map(|v| !matches!(v.as_str(), "" | "0" | "false" | "no"))
        .unwrap_or(false);
    assert!(
        !enforce,
        "{key}: measured {measured:.4} regressed beyond {pct}% of baseline {base:.4} (limit {limit:.4})"
    );
    println!(
        "{key}: {measured:.4} regressed beyond {pct}% of baseline {base:.4} (limit {limit:.4}) \
         — reported only; set PERF_ENFORCE=1 to fail"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_without_failing_by_default() {
        // A much slower-than-baseline value must not panic unless enforced.
        let enforce = std::env::var("PERF_ENFORCE").unwrap_or_default();
        assert!(
            enforce.is_empty() || matches!(enforce.as_str(), "" | "0" | "false" | "no"),
            "this test assumes PERF_ENFORCE is unset"
        );
        baseline_gate("build_rows_frame_ms", 999.0, false); // no panic
        baseline_gate("vt_parse_mbps", 0.001, true); // no panic
    }

    #[test]
    fn unknown_keys_are_ignored() {
        baseline_gate("definitely_not_a_metric", 1.0, false);
    }
}

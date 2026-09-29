# Performance

Budgets and the CI gate (ADR 0018). The roadmap's budgets live in
`docs/private/miaotty-performance.md` §2; this file maps them onto what we can
measure deterministically in CI. Machine-readable values and measured baselines
are in [`../benches/budgets.json`](../benches/budgets.json).

## The gate

```sh
cargo test --release -p miao-term-core -p miaotty-app -- --ignored
```

Perf tests are `#[ignore]`d so the normal test run stays fast; the `perf` CI job
runs them in release on `ubuntu-latest`. Budgets are absolute with generous
headroom (an order-of-magnitude regression fails, runner jitter does not).
`MIAOTTY_PERF_SCALE` (default `1.0`) relaxes every budget by a factor on slow
machines. Each metric is also checked against the recorded **baseline** in
`budgets.json` with a `regression_pct` (ADR 0023), so drift fails, not just
cliffs. The `perf` job additionally compares against a **CI-side baseline** cached per runner OS (ADR 0028), so drift is caught in CI too — reported only, since runner variance (1.8x on the same code) dwarfs the signal; the absolute budgets are the gate.

## Budgets

| Metric | Budget | Baseline (M-series, release) | Where |
|--------|--------|------------------------------|-------|
| VT parse throughput | ≥ 25 MB/s | 73 MB/s | `crates/term-core/tests/perf.rs` |
| Screen snapshot (30 rows) | ≤ 2 ms | 0.012 ms | `crates/term-core/tests/perf.rs` |
| Row build per frame | ≤ 4 ms | 0.12 ms | `miaotty-app` `perf_tests` |
| Palette rank (10k entries) | ≤ 100 ms | 2.0 ms | `miaotty-app` `perf_tests` |
| IPC idle cost | ≈ 0 (no polling) | — | by design |
| Agent burst | 100 events → 1 repaint | — | by design |

The row-build budget is the roadmap frame-time budget (`p99 < 4 ms`) applied to
the damage-rebuild path, which is the part a benchmark can exercise without a
GPU. Cold start (`< 300 ms`), new-pane (`< 50 ms`) and memory are not CI gates
(they need a window / a live process) and are measured manually; they stay in
`budgets.json` marked `hard: false`.

## Not gated (and why)

- **Frame time / key-to-glyph latency** need a real window and GPU; the row-build
  gate approximates frame cost headlessly.
- **Throughput vs Ghostty (≥ 95%)** needs Ghostty on the same machine; we gate an
  absolute parse rate instead.
- **Power** needs `powermetrics`; reviewed manually.

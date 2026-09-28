# ADR 0018 — Performance gate

Status: accepted.

## Context

The roadmap makes every milestone's exit condition a performance budget
(`docs/private/miaotty-performance.md` §2) and says no hot-path code lands before
a baseline + CI gate exists. The Swift reference measured frame time and power
with Instruments; we need something a headless CI can run deterministically.

## Decision

A **release-mode, absolute-budget gate** rather than a statistical benchmark
runner:

- `crates/term-core/tests/perf.rs` measures VT parse throughput (≥ 25 MB/s over
  a 16 MB SGR-heavy workload) and a 30-row screen snapshot (≤ 2 ms).
- `miaotty-app` `perf_tests` measure the damage-rebuild path, `build_rows`
  (≤ 4 ms/frame — the roadmap frame budget), and Open Quickly ranking over 10k
  entries (≤ 100 ms).
- All perf tests are `#[ignore]`d so the normal `cargo test` stays fast; a
  dedicated `perf` CI job runs `cargo test --release … -- --ignored` on
  `ubuntu-latest`. `MIAOTTY_PERF_SCALE` relaxes every budget on slow machines.
- Budgets and measured baselines live in `benches/budgets.json`; the mapping from
  the roadmap's budgets and what is *not* gated (frame time, key-to-glyph,
  throughput-vs-Ghostty, power, memory, cold start) is documented in
  `docs/PERFORMANCE.md`.

Absolute budgets with large headroom were chosen over percentage-regression
tracking because CI runners are noisy and we have no shared baseline store; an
order-of-magnitude regression still fails, which is the failure mode we care
about.

## Consequences

- The hot paths (VT parse, row rebuild, palette ranking) have a real,
  reproducible gate on every push.
- Numbers are conservative: on an M-series machine the row build is ~0.12 ms
  against a 4 ms budget and ranking is ~2 ms against 100 ms, so the gate catches
  cliffs, not drift.
- GPU frame time, end-to-end latency and memory remain manual measurements;
  adding a shared baseline store later can tighten budgets without changing the
  harness.

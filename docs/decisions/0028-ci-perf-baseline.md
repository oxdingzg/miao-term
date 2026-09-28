# ADR 0028 — CI-side performance baseline

Status: accepted.

## Context

ADR 0023 compares perf metrics against a baseline recorded in
`benches/budgets.json`, measured on the maintainer's machine. In CI the
*absolute* budgets (ADR 0018) are what can bind, because an M-series baseline is
not meaningful for an `ubuntu` runner — so CI could only catch cliffs, not drift.

## Decision

The perf tests now also **write** `target/perf-measured.json` (all four gated
metrics; the app and the core merge into the same file). The `perf` job:

1. restores `.perf/baseline.json` from the actions cache
   (`perf-baseline-<os>-…`, `restore-keys` fall back to the newest for that OS);
2. runs the perf tests;
3. runs `scripts/check-perf-baseline.py`, which fails a metric that regresses
   beyond `regression_pct` (from `budgets.json`, overridden to 40% in CI so
   shared-runner noise does not flake) — direction-aware, so throughput must not
   drop and times must not grow;
4. on `main`, copies the measurements back into `.perf/baseline.json`, which the
   cache saves at job end.

The first run has no baseline: it reports and passes, then seeds the cache.

## Consequences

- CI compares a runner against *itself* over time, catching drift that the wide
  absolute budgets would hide, without depending on the maintainer's hardware.
- Baselines are per-runner-OS and cached, so they can expire (7 days without
  access); the job then reports instead of failing.
- `benches/budgets.json` remains the reviewed, machine-independent record of
  budgets and the reference baseline; the CI baseline is a cache, not source.

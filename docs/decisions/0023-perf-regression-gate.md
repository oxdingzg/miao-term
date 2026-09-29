# ADR 0023 — Performance regression gate (shared baseline)

Status: accepted.

## Context

ADR 0018 gated hot paths against *absolute* budgets with large headroom, so a
real regression (say 2×) could pass. Tightening needs a recorded baseline to
compare against.

## Decision

`benches/budgets.json` — already committed — now carries, per gated metric, both
the **absolute budget** and the **recorded baseline** plus a `regression_pct`.
The perf tests (ADR 0018) load that file and assert two things:

1. the absolute budget (unchanged), and
2. `measured` within `baseline × (1 ± regression_pct)` — worse than the baseline
   by more than the threshold fails, direction-aware (throughput must not drop,
   times must not grow).

`regression_pct` is 25% today, which is loose enough for CI jitter on the
absolute numbers but tight enough to catch a genuine slowdown that the wide
absolute budget would hide. Bumping the baseline is an explicit edit to
`budgets.json` in a PR, so a regression can never be silently absorbed.

## Consequences

- The gate catches drift, not just cliffs, and the baseline lives in the repo so
  it is reviewable and shared.
- Baselines are machine-specific (recorded on an M-series Mac); CI still runs on
  `ubuntu-latest`, so the regression check is most meaningful on the maintainer's
  hardware and the absolute budget remains the CI-safe gate. A CI-maintained
  baseline store is the next step if we want the tighter check to bind in CI.
- Update: the CI-side baseline store landed in ADR 0028, so the tighter check can
  bind on a stable runner.

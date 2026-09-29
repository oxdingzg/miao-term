#!/usr/bin/env python3
"""Compare measured perf metrics (ADR 0028) against the CI baseline.

The perf tests write `target/perf-measured.json`; CI restores a baseline from
the actions cache into `.perf/baseline.json` and prints the comparison.

By default it **reports only**: the same code measured 139 MB/s and then 79 MB/s
of VT parse throughput on `ubuntu-latest`, so a cached baseline is not a gate
there — the *absolute* budgets asserted inside the tests are. Set
`PERF_ENFORCE=1` (e.g. on the dev machine, or with a stable runner) to fail on a
regression beyond the metric's `regression_pct` (default 25%, overridable with
`PERF_REGRESSION_PCT`) once it is above the noise floor.

On the first run there is no baseline, so every metric is only reported; CI
copies the measurements into the cache afterwards on `main`.
"""

import json
import os
import pathlib
import sys

enforce = os.environ.get("PERF_ENFORCE", "0") not in ("", "0", "false", "no")

HIGHER_IS_BETTER = {"vt_parse_mbps"}

# Times below these floors are jitter-dominated on shared CI runners: the same
# code measured 0.06 then 0.10 ms between runs. There the *absolute* budget is
# the gate and the comparison is only reported.
ENFORCE_FLOOR = {
    "vt_parse_mbps": 0.0,  # a sustained measurement, always comparable
    "build_rows_frame_ms": 0.5,
    "palette_rank_10k_ms": 10.0,
    "screen_snapshot_30_rows_ms": 0.5,
}

measured_path = pathlib.Path(os.environ.get("PERF_MEASURED", "target/perf-measured.json"))
baseline_path = pathlib.Path(os.environ.get("PERF_BASELINE", ".perf/baseline.json"))
budgets = json.loads(pathlib.Path("benches/budgets.json").read_text())

if not measured_path.is_file():
    print("check-perf-baseline: no measurements; skipping")
    sys.exit(0)

measured = json.loads(measured_path.read_text())
baseline = json.loads(baseline_path.read_text()) if baseline_path.is_file() else {}

failures = []
for key, value in sorted(measured.items()):
    value = float(value)
    default_pct = float(budgets.get(key, {}).get("regression_pct", 25))
    pct = float(os.environ.get("PERF_REGRESSION_PCT", default_pct))
    base = baseline.get(key)
    if base is None:
        print(f"{key}: {value:.4f} (no baseline yet)")
        continue
    base = float(base)
    if not enforce:
        print(f"{key}: {value:.4f} (report only; baseline {base:.4f})")
        continue
    if value < ENFORCE_FLOOR.get(key, 0.0):
        print(f"{key}: {value:.4f} (reported only; below the {ENFORCE_FLOOR[key]} floor)")
        continue
    if key in HIGHER_IS_BETTER:
        limit = base * (1 - pct / 100)
        ok = value >= limit
        direction = ">="
    else:
        limit = base * (1 + pct / 100)
        ok = value <= limit
        direction = "<="
    print(
        f"{key}: {value:.4f} (baseline {base:.4f}, needs {direction} {limit:.4f}) "
        + ("ok" if ok else "REGRESSION")
    )
    if not ok:
        failures.append(key)

if failures:
    print(f"check-perf-baseline: FAILED — regressed: {', '.join(failures)}")
    sys.exit(1)
print("check-perf-baseline: ok")

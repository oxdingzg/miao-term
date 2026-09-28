#!/usr/bin/env python3
"""Compare measured perf metrics (ADR 0028) against the CI baseline.

The perf tests write `target/perf-measured.json`; CI restores a baseline from
the actions cache into `.perf/baseline.json`. This script fails when a metric
regresses beyond the `regression_pct` recorded in `benches/budgets.json`
(default 25%, overridable with `PERF_REGRESSION_PCT` so runner noise does not
flake the gate).

On the first run there is no baseline, so every metric is only reported; CI
copies the measurements into the cache afterwards on `main`.
"""

import json
import os
import pathlib
import sys

HIGHER_IS_BETTER = {"vt_parse_mbps"}

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

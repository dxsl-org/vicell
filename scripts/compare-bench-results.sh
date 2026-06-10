#!/usr/bin/env bash
# compare-bench-results.sh — Compare current perf results against historical
# median and flag regressions > 10% sustained over 3 consecutive runs.
#
# Usage: ./scripts/compare-bench-results.sh <results-dir>
#   results-dir: directory containing perf-YYYY-MM-DD.json files

set -euo pipefail

RESULTS_DIR="${1:-perf-results}"
REGRESSION_THRESHOLD=10  # percent above historical median → regression
CONSECUTIVE_REQUIRED=3   # consecutive bad runs before failing the build
STATE_FILE="${RESULTS_DIR}/regression-state.json"

if ! command -v python3 &>/dev/null; then
  echo "[compare] python3 not found — skipping regression analysis"
  exit 0
fi

if [ ! -d "$RESULTS_DIR" ]; then
  echo "[compare] Results directory not found: $RESULTS_DIR"
  exit 0
fi

RESULT_FILES=("$RESULTS_DIR"/perf-*.json)
if [ ${#RESULT_FILES[@]} -lt 2 ]; then
  echo "[compare] Fewer than 2 result files — skipping comparison (need history)"
  exit 0
fi

echo "[compare] Analysing ${#RESULT_FILES[@]} result file(s) in $RESULTS_DIR"

python3 - "$RESULTS_DIR" "$REGRESSION_THRESHOLD" "$CONSECUTIVE_REQUIRED" "$STATE_FILE" <<'PYEOF'
import json
import os
import sys
from pathlib import Path

results_dir = Path(sys.argv[1])
threshold   = int(sys.argv[2])
consec_req  = int(sys.argv[3])
state_file  = Path(sys.argv[4])

# Load all result files sorted by date
files = sorted(results_dir.glob("perf-*.json"))
history: dict[str, list[float]] = {}

for f in files:
    try:
        data = json.loads(f.read_text())
        for entry in data.get("results", []):
            name = entry.get("name", "?")
            p99  = float(entry.get("p99", 0))
            history.setdefault(name, []).append(p99)
            # RT-specific latency metrics (p999, jitter): tracked alongside p99
            for rt_key in ("p999", "jitter"):
                if rt_key in entry:
                    history.setdefault(f"{name}/{rt_key}", []).append(float(entry[rt_key]))
            # Deadline-miss count: tracked separately (special zero-baseline rule)
            if "miss" in entry:
                history.setdefault(f"{name}/miss", []).append(float(entry["miss"]))
    except Exception as e:
        print(f"[compare] Warning: could not parse {f}: {e}")

# Load regression state
state: dict[str, int] = {}
if state_file.exists():
    try:
        state = json.loads(state_file.read_text())
    except Exception:
        pass

regressions = []

for metric, samples in history.items():
    if len(samples) < 2:
        continue
    # Historical median (all but last sample)
    hist = sorted(samples[:-1])
    median = hist[len(hist) // 2]
    current = samples[-1]

    # Deadline-miss: any miss when baseline was clean is an immediate regression.
    if metric.endswith("/miss"):
        if median == 0 and current > 0:
            state[metric] = state.get(metric, 0) + 1
            print(f"[compare] REGRESSION (deadline-miss appeared): {metric} "
                  f"baseline=0 current={current:.0f} "
                  f"[{state[metric]}/{consec_req} consecutive]")
            if state[metric] >= consec_req:
                regressions.append(metric)
        else:
            if metric in state:
                print(f"[compare] RECOVERED: {metric} (was {state[metric]} consecutive runs)")
                del state[metric]
            else:
                print(f"[compare] OK: {metric} miss={current:.0f}")
        continue

    if median == 0:
        continue

    pct_change = ((current - median) / median) * 100.0
    if pct_change > threshold:
        state[metric] = state.get(metric, 0) + 1
        print(f"[compare] REGRESSION ({pct_change:.1f}%): {metric} "
              f"current={current:.0f} median={median:.0f} "
              f"[{state[metric]}/{consec_req} consecutive]")
        if state[metric] >= consec_req:
            regressions.append(metric)
    else:
        if metric in state:
            print(f"[compare] RECOVERED: {metric} (was {state[metric]} consecutive runs)")
            del state[metric]
        else:
            print(f"[compare] OK: {metric} +{pct_change:.1f}% vs median (within {threshold}% threshold)")

# Persist state
state_file.write_text(json.dumps(state, indent=2))

if regressions:
    print(f"[compare] FATAL: {len(regressions)} metric(s) regressed for "
          f">= {consec_req} consecutive runs: {', '.join(regressions)}")
    sys.exit(1)
else:
    print("[compare] No sustained regressions detected")
PYEOF

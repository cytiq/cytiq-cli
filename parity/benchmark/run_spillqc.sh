#!/usr/bin/env bash
# Benchmark `cytiq spillqc` on the real acquisitions of `just parity-real`
# (run it first; CYTIQ_REAL_SLOW=1 adds the FACSymphony controls).
# Usage: parity/benchmark/run_spillqc.sh   (also `just benchmark-spillqc`)
# Writes parity/results/benchmark_spillqc.json. Measures, never fails on a
# threshold: the numbers describe CompensAID's diagnostic value, not parity.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PARITY="$ROOT/parity"
if [[ -z "${CYTIQ_BIN:-}" ]]; then
  export CYTIQ_BIN="$ROOT/target/release/cytiq"
  (cd "$ROOT" && cargo build --release --quiet)
fi
"$PARITY/.env/py/bin/python" "$PARITY/benchmark/spillqc_benchmark.py" "$PARITY/.out/real" \
  "$PARITY/results/benchmark_spillqc.json" >/dev/null
echo "metrics written to $PARITY/results/benchmark_spillqc.json"

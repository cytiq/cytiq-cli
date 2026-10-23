#!/usr/bin/env bash
# Validate cytiq on real acquisitions against upstream R tools.
#
# Usage: parity/real/run_real.sh
#   Datasets (no download needed; installed by parity/setup_env.sh):
#   - FlowSOM's 68983.fcs: 18-colour mouse sample with a manual Gating-ML
#     strategy and FlowJo population labels for every event.
#   - PeacoQC's 111.fcs: 20-colour LSRII sample used in the PeacoQC vignette.
#   Set CYTIQ_REAL_DATA=<dir> to also run every `*.fcs` with `$SPILL` in <dir>
#   (for example internal data that cannot be committed).
# Writes parity/results/metrics_real.json and exits non-zero when a rule in
# parity/real/tolerances.json is violated.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PARITY="$ROOT/parity"
OUT="$PARITY/.out/real"
PY="$PARITY/.env/py/bin/python"
export R_LIBS_USER="$PARITY/.env/Rlib"
BIN="${CYTIQ_BIN:-$ROOT/target/release/cytiq}"
[[ -n "${CYTIQ_BIN:-}" ]] || (cd "$ROOT" && cargo build --release --quiet)
export CYTIQ_LOG=warn

rm -rf "$OUT"
mkdir -p "$OUT"
# Files over 100 MB: the writer's offset handling (ignored in CI for size).
(cd "$ROOT" && cargo test -p cytiq-fcs --release --quiet -- --ignored)
LIB="$R_LIBS_USER"

# --- FlowSOM 68983: compensate, manual gating, logicle, FlowSOM -------------
D="$OUT/flowsom_68983"
mkdir -p "$D"
cp "$LIB/FlowSOM/extdata/68983.fcs" "$D/sample.fcs"
cp "$LIB/FlowSOM/extdata/gatingResult.csv" "$D/flowjo_labels.csv"
# FlowJo names compensated channels `Comp-<detector>` and omits the
# compensation-ref that Gating-ML 2.0 requires (FlowKit rejects the file). The
# sample is compensated by `cytiq compensate` first, so the dimensions are
# plain detectors, explicitly `uncompensated`, for both cytiq and FlowKit.
sed -e 's/name="Comp-/name="/g' \
  -e 's/<gating:dimension/<gating:dimension gating:compensation-ref="uncompensated"/g' \
  "$LIB/FlowSOM/extdata/manualGating.xml" > "$D/gates.xml"
MARKERS="Pacific Blue-A,APC-A,Alexa Fluor 700-A,APC-Cy7-A,PE-A,PE-Texas Red-A,PE-Cy5-A,PE-Cy7-A"
"$BIN" compensate --input "$D/sample.fcs" --output "$D/sample.comp.fcs"
"$BIN" gate --input "$D/sample.comp.fcs" --gating-ml "$D/gates.xml" --output "$D/counts.json" \
  --format json --membership "$D/membership.parquet"
"$BIN" transform --input "$D/sample.comp.fcs" --output "$D/sample.t.fcs" --method logicle \
  --channels "$MARKERS"
"$BIN" flowsom --input "$D/sample.t.fcs" --output "$D/flowsom.csv" --channels "$MARKERS" \
  --metaclusters 10
echo "$MARKERS" > "$D/markers.txt"

# --- PeacoQC 111 and any CYTIQ_REAL_DATA files: margins, compensate, PeacoQC
run_qc_chain() {
  local source="$1" target="$2"
  mkdir -p "$target"
  cp "$source" "$target/sample.fcs"
  "$BIN" compensate --input "$target/sample.fcs" --output "$target/sample.comp.fcs"
  local channels
  channels="$("$PY" "$PARITY/real/spill_channels.py" "$target/sample.fcs")"
  echo "$channels" > "$target/channels.txt"
  "$BIN" transform --input "$target/sample.comp.fcs" --output "$target/sample.t.fcs" \
    --method logicle --channels "$channels"
  "$BIN" peacoqc --input "$target/sample.t.fcs" --output "$target/sample.peacoqc.fcs" \
    --channels "$channels"
}
run_qc_chain "$LIB/PeacoQC/extdata/111.fcs" "$OUT/peacoqc_111"
if [[ -n "${CYTIQ_REAL_DATA:-}" ]]; then
  for fcs in "$CYTIQ_REAL_DATA"/*.fcs; do
    [[ -e "$fcs" ]] || continue
    run_qc_chain "$fcs" "$OUT/user_$(basename "$fcs" .fcs)"
  done
fi

Rscript "$PARITY/real/real.R" "$OUT"
"$PY" "$PARITY/real/compare_real.py" "$OUT" "$PARITY/results/metrics_real.json"

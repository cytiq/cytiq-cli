#!/usr/bin/env bash
# Run every cytiq subcommand on the synthetic parity datasets.
# Usage: parity/run_rust.sh <data_dir> <out_dir>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA="${1:-$ROOT/parity/.data}"
OUT="${2:-$ROOT/parity/.out/rust}"
# CYTIQ_BIN lets the harness run another build (e.g. the pre-fix baseline).
if [[ -n "${CYTIQ_BIN:-}" ]]; then
  BIN="$CYTIQ_BIN"
else
  BIN="$ROOT/target/release/cytiq"
  (cd "$ROOT" && cargo build --release --quiet)
fi
mkdir -p "$OUT"

timed() {
  local name="$1"
  shift
  /usr/bin/time -l "$@" 2>"$OUT/$name.time" >"$OUT/$name.log" || {
    cat "$OUT/$name.time" >&2
    exit 1
  }
}

mkdir -p "$OUT/transform"
for method in arcsinh logicle hyperlog biexponential; do
  timed "transform_$method" "$BIN" transform -i "$DATA/transform/grid.fcs" \
    -o "$OUT/transform/$method.fcs" --method "$method" --channels X --overwrite
done

timed autospill "$BIN" autospill --controls "$DATA/autospill/controls.csv" \
  --input-dir "$DATA/autospill" --output "$OUT/autospill_spillover.csv" \
  --matrix-kind spillover --debug-dir "$OUT/autospill_debug"
timed autospread "$BIN" autospread --controls "$DATA/autospill/controls.csv" \
  --input-dir "$DATA/autospill" --output "$OUT/autospread.csv"

rm -rf "$OUT/autospectral"
timed autospectral "$BIN" autospectral --controls "$DATA/autospectral/controls.csv" \
  --samples "$DATA/autospectral/samples.csv" --input-dir "$DATA/autospectral" \
  --output "$OUT/autospectral"

for method in all mad it; do
  timed "peacoqc_$method" "$BIN" peacoqc --input "$DATA/peacoqc/sample.fcs" \
    --output "$OUT/peacoqc_$method.fcs" --channels "$(cat "$DATA/peacoqc/channels.txt")" \
    --method "$method" --debug-dir "$OUT/peacoqc_${method}_debug"
done

for dataset in flowsom flowsom_hard; do
  timed "$dataset" "$BIN" flowsom --input "$DATA/$dataset/sample.fcs" \
    --output "$OUT/$dataset.csv" --channels "$(cat "$DATA/$dataset/channels.txt")" \
    --debug-dir "$OUT/${dataset}_debug"
done

echo "rust outputs in $OUT"

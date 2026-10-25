#!/usr/bin/env bash
# Regenerate data/fixtures/parity: small synthetic datasets and what the
# upstream R tools compute on them. crates/cytiq-cli/tests/parity_smoke.rs
# checks cytiq against these outputs in CI, where R is not installed.
#
# Usage: parity/make_smoke.sh (also `just parity-smoke-fixture`)
#   Requires parity/setup_env.sh to have been run once. Rerun it when the
#   generator, an upstream pin or a dataset size changes; cytiq changes need
#   no rerun, which is the point.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PARITY="$ROOT/parity"
FIXTURE="$ROOT/data/fixtures/parity"
PY="$PARITY/.env/py/bin/python"
export R_LIBS_USER="$PARITY/.env/Rlib"
WORK="$PARITY/.out/smoke"

rm -rf "$WORK" "${FIXTURE:?}"/{autospill,autospectral,peacoqc,cytoscan_exact}
mkdir -p "$WORK"
"$PY" "$PARITY/gen/generate.py" "$FIXTURE" smoke
rm -f "$FIXTURE/seed.json"
# No cytiq outputs: the cross-checks of run_upstream.R that read them are skipped.
PARITY_RUST_OUT="$WORK/no-cytiq-outputs" \
  Rscript "$PARITY/run_upstream.R" "$FIXTURE" "$WORK" autospill autospectral peacoqc cytoscan

# Keep the outputs the CI test reads, under <dataset>/upstream/.
mkdir -p "$FIXTURE"/{autospill,autospectral,peacoqc,cytoscan_exact}/upstream
cp "$WORK/autospill_spillover.csv" "$FIXTURE/autospill/upstream/spillover.csv"
"$PY" - "$WORK/autospill_gates.csv" "$FIXTURE/autospill/upstream/gate_counts.csv" <<'PY'
import sys
import pandas as pd
gates = pd.read_csv(sys.argv[1])
gates.groupby("sample").size().rename("events").to_csv(sys.argv[2], index_label="dye")
PY
cp "$WORK/autospectral_unmix_truth_spectra.csv" "$FIXTURE/autospectral/upstream/unmix_truth_spectra.csv"
for method in all mad it; do
  cp "$WORK/peacoqc_${method}_mask.csv" "$FIXTURE/peacoqc/upstream/${method}_mask.csv"
done
for method in summary quantiles emd binning fingerprint; do
  for file in "features_test_$method.csv" "outlier_$method.csv" "novelty_$method.csv" "novelty_$method.json"; do
    cp "$WORK/cytoscan_exact/$file" "$FIXTURE/cytoscan_exact/upstream/$file"
  done
done
cp "$PARITY/versions.txt" "$FIXTURE/versions.txt"
du -sh "$FIXTURE"

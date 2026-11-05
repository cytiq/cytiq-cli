# Parity smoke fixture

Small synthetic datasets and what the upstream R tools computed on them, so
CI can check cytiq against upstream without installing R
(`crates/cytiq-engine/tests/parity_smoke.rs`, run by `cargo test`). The full
suites (`just parity`, `just parity-real`) stay local; this set covers the
results that are exact or deterministic on both sides, with the bounds of
`parity/tolerances.json`.

| Dataset | Contents | Upstream output (`upstream/`) | Checked |
| --- | --- | --- | --- |
| `autospill/` | 6 single-stain controls, 3,000 events each | `spillover.csv` (R autospill, refined), `gate_counts.csv` | spillover matrix to 1e-9; FSC/SSC gate keeps the same events per control |
| `autospectral/` | unstained + 6 single stains (2,000 events), a 3,000-event sample, 16 detectors; true spectra and abundances in `truth/` | `unmix_truth_spectra.csv` (AutoSpectral `unmix.ols` with the true spectra) | spectra cosine ≥ 0.9999 against the truth; abundance RMSE within 5% of R's on the true spectra |
| `peacoqc/` | 15,000 events, 6 channels, clog, drift and burst | `{all,mad,it}_mask.csv` (R PeacoQC) | identical good-event masks in every mode |
| `spillqc/` | 3 tubes of 6,000 events, 6 markers, `$SPILLOVER` with two injected errors; true matrix and injected pairs in `truth/` | `upstream/<tube>/{ssi_matrix,ssi_segments,cutoffs}.csv` (R CompensAID after the vignette chain) | cut-offs, segment edges, counts, merge messages to 1e-9; SSI to its 2-decimal rounding; every flag; both errors recurrent in `batch.csv` |
| `cytoscan_exact/` | 10 test and 10 reference files of 1,000 events (every event used on both sides) | `features_test_*.csv`, `outlier_*.csv`, `novelty_*.{csv,json}` (CytoScan, isotree, Mclust) | features to 1e-9 for all five methods; outlier flags; Mclust's novelty model, G and flags |

Regenerate with `just parity-smoke-fixture` (`parity/make_smoke.sh`, needs
`just parity-setup` once) when the generator (`parity/gen/generate.py`,
profile `smoke`), a dataset size or an upstream pin changes. A cytiq change
never needs it: that is what the test checks. `versions.txt` records the
upstream versions the outputs come from.

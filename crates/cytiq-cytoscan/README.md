# cytiq-cytoscan

CytoScan-style anomaly detection for cohorts of FCS files. Backs the
`cytiq cytoscan` command.

- Paper: <https://doi.org/10.1101/2025.09.24.678276>
- Upstream R reference: <https://github.com/AUMC-HEMA/CytoScan> (parity target
  CytoScan 0.1.0, `c97f403`)

Each file is reduced to one feature vector over the selected channels, then
files are flagged either as outliers within the test cohort or as novelties
against a separate high-quality reference cohort.

## Pipeline

1. Resolve inputs: `.fcs` files, directories with immediate `.fcs` children,
   or simple file-name globs.
2. Read each file like `CytoScan::readInput`: truncate values above `$PnR`,
   compensate the selected channels with `SPILL` / `$SPILL`, transform with
   `asinh(x / 150)`. Without spillover the channels stay raw and a warning is
   recorded.
3. Subsample 1,000 events per file with the generator reseeded to 42 for every
   file, so results do not depend on path spelling.
4. Compute features (`FeatureMethod`): `summary`, `quantiles` (default), `emd`,
   `binning` (`cut()` decile bins) or `fingerprint` (`flowFP` probability
   binning, 4 recursions). The last three use an aggregate of
   `ceil(10000 / n_files)` events per file, taken from the reference cohort
   when novelty is enabled.
5. Flag (`FlagMethod`):
   - `outlier` (default): isolation forest as
     `isotree::isolation.forest(sample_size = 1, ntrees = 1000, ndim = 1)`;
     score `>= 0.5` is an outlier.
   - `novelty`: Gaussian mixtures (`linfa-clustering`, full covariance,
     `G = 1..9`, mclust-style BIC) fitted on reference features; test files at
     or below the 5% quantile of reference log-densities are novelties. With no
     more reference files than features, a diagonal Gaussian is used and a
     warning is recorded.
   - `both`.

Full rules are in the root [README](../../README.md#cytoscan). Measured
agreement with R is in [`parity/REPORT.md`](../../parity/REPORT.md) §2.7:
features match R to ~5e-13 on identical events; random draws use `rand`, so
results are reproducible but not bit-identical to R.

## Usage

```bash
cytiq cytoscan --input ./test_fcs --output ./cytoscan_results --channels CD3,CD4,CD8
cytiq cytoscan --input ./test_fcs --reference ./reference_fcs \
  --output ./cytoscan_results --channel CD3 --channel CD4 \
  --method fingerprint --flag-method both --debug-dir ./cytoscan_debug
```

From Rust:

```rust
use cytiq_cytoscan::{CytoScanConfig, CytoScanRunner, FeatureMethod, FlagMethod, parse_channel_list};

let mut config = CytoScanConfig::new(
    vec!["./test_fcs".into()],
    "./cytoscan_results".into(),
    parse_channel_list("CD3,CD4,CD8")?,
);
config.reference_inputs = vec!["./reference_fcs".into()];
config.feature_method = FeatureMethod::Emd;
config.flag_method = FlagMethod::Both;
let result = CytoScanRunner::run(&config)?;
for row in &result.summary_rows { /* per-file flags and scores */ }
```

## Public API

| Item | Purpose |
| --- | --- |
| `CytoScanRunner::run` / `CytoScanConfig` | Run the pipeline and write outputs |
| `FeatureMethod`, `FlagMethod` | Feature and flagging methods |
| `CytoScanResult` | `parameters`, test/reference `CytoScanFeatureMatrix`, `summary_rows`, `diagnostics` |
| `CytoScanParameters`, `CytoScanConstants` | Effective settings, serialized into `cytoscan_config.json` |
| `CytoScanFeatureMatrix`, `CytoScanFeatureRow`, `CytoScanSummaryRow`, `CytoScanDiagnostics` | Result rows and model diagnostics |
| `NoveltyModelKind` | Which novelty model was fitted (mixture or diagonal fallback) |
| `DEFAULT_*` constants | Fixed algorithm settings (seed 42, 1,000 events, 10,000 aggregate, 1,000 trees, 0.5 / 0.05 thresholds, cofactor 150, 4 recursions) |
| `parse_channel_list` | Parse a comma-separated `--channels` value |
| `CytoScanError` | Error type for every failure mode |

## Output rules

- Always written under `output_dir` (created if missing; existing files are
  replaced): `cytoscan_summary.csv`, `cytoscan_features.csv`,
  `cytoscan_config.json`.
- With `debug_dir`: `reference_features.csv`, `aggregate_sampling.csv`,
  `model_diagnostics.json`, `pca_coordinates.csv`, `pca_loadings.csv`,
  `feature_heatmap.csv`, `boxplot_samples.csv` and a Plotly `report.html`.
- `strict` (CLI `--strict`) turns the "no spillover metadata" and "channel
  not in the spillover matrix" warnings into errors.
- `novelty` and `both` require reference inputs; `threads = Some(0)` is
  rejected.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading and compensation
- `linfa` / `linfa-clustering` (GMM), `nalgebra` / `ndarray`, `rand` /
  `rand_distr` / `rand_xoshiro`, `rayon`, `csv`, `serde_json`, `plotly`

## Tests

```bash
cargo test -p cytiq-cytoscan
cargo test -p cytiq-cli --test cytoscan_cli
```

Unit tests in `src/runner.rs` and `src/novelty.rs` pin behaviour to the R
packages (transport, `cut()`, flowFP, isotree, mclust).

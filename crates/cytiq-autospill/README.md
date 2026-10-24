# cytiq-autospill

AutoSpill spillover/compensation estimation and AutoSpread spillover-spreading
estimation from single-colour controls. Backs the `cytiq autospill` and
`cytiq autospread` commands.

- Paper: Roca et al., *AutoSpill is a principled framework that simplifies the
  analysis of multichromatic flow cytometry data*,
  [Nat. Commun. 2021](https://www.nature.com/articles/s41467-021-23126-8)
- Upstream R reference: <https://github.com/carlosproca/autospill>

## Pipeline

1. **Load controls** — read the `filename,dye` manifest, open each FCS file,
   infer the primary channel by matching `dye` against `$PnN`, then `$PnS`
   (case-insensitive, one trailing `-A`/`-H`/`-W` stripped). `dye =
   Autofluorescence` is reserved for a synthetic autofluorescence control and
   needs a detectors manifest. Values above a channel's `$PnR` are capped at
   `$PnR`, as `flowCore::read.FCS` does by default (`truncate_max_range`);
   FACSDiva float data can exceed it.
2. **FSC/SSC gate** — `cytiq_density::gate_events`, with the scatter range
   clamped to `[0, max($P1R)]` over all controls (`read.flow.control`).
3. **Initial spillover** — per-pair `secondary ~ primary` fits with a port of
   `MASS::rlm` (Huber, `k = 1.345`, IRLS, OLS fallback when it does not
   converge), as in `fit.robust.linear.model`.
4. **Iterative refinement** — first on the linear scale, then on the FlowJo
   biexponential scale (`flowjo_biexp(256, 262144, 4.418539922, 0, -100)` by
   default, per channel overridable with a transform manifest), with the
   upstream λ / Δ-history state machine.
5. **Export** — spillover or compensation matrix as CSV, TSV or Parquet.

AutoSpread reuses steps 1–2 and estimates a dye × detector spread matrix from
quantile bins of each control. Neither `autospill` nor `AutoSpectral` computes
a spread matrix, so this part is a Nguyen-2013-style heuristic without an
upstream reference.

On the synthetic parity datasets the spillover matrix matches R `autospill`
0.2.0 to `4.8e-13`, and on real single-stain controls from a FACSCelesta (11
colours) and a FACSymphony (24 colours) to `5e-13`; the gates match event for
event ([`parity/REPORT.md`](../../parity/REPORT.md) sections 2.2 and 2.10).

Manifest formats, matrix layout and flags: root
[README](../../README.md#autospill).

## Usage

```bash
cytiq autospill --controls controls.csv --input-dir ./controls --output compensation.csv
cytiq autospread --controls controls.csv --input-dir ./controls \
  --detectors detectors.csv --output spread.csv
```

From Rust:

```rust
use cytiq_autospill::{
    AutospillConfig, AutospillRunner, MatrixKind, export_matrix, read_controls_manifest,
};

let rows = read_controls_manifest("controls.csv")?;
let config = AutospillConfig { matrix_kind: MatrixKind::Spillover, ..Default::default() };
let result = AutospillRunner::run(&rows, "./controls", &config)?;
export_matrix(&result, config.matrix_kind, "spillover.parquet")?;
```

## Public API

| Item | Purpose |
| --- | --- |
| `AutospillRunner::run`, `AutospillConfig`, `AutospillResult` | AutoSpill pipeline; result holds spillover, compensation, residual slope/skewness matrices, gate stats and convergence |
| `AutospreadRunner::run`, `AutospreadConfig`, `AutospreadResult` | AutoSpread pipeline |
| `MatrixKind` | `Compensation` (default) or `Spillover` |
| `export_matrix`, `export_spread_matrix` | Write a matrix; format chosen by extension (`csv`, `tsv`, `parquet`) |
| `read_controls_manifest`, `read_detectors_manifest`, `read_transform_manifest` | Read and validate the CSV manifests |
| `ControlsManifestRow`, `DetectorSpec`, `TransformSpec` | Manifest rows |
| `GateStats`, `ConvergenceStep`, `ConvergenceSummary`, `SpreadStats` | Diagnostics |
| `AutospillError` | Error type for every failure mode |

## Output rules

- Controls must be raw acquisition files: one with `CYTIQ_COMPENSATION`,
  `CYTIQ_UNMIXING` or `CYTIQ_TRANSFORM` is `ProcessedControl`.
- Without a detectors manifest (or when detectors equal the primary channels)
  the matrix is square with a `channel` header. With detectors it is
  rectangular: spillover is dye × detector, compensation is detector × dye,
  spread is dye × detector.
- The output format comes from the file extension; anything else is
  `UnsupportedOutputFormat`. Existing files are replaced.
- `debug_dir` gets one `report.html` (gates, regressions, convergence,
  heatmaps) and `summary.json`.
- Controls, gates, spillover rows, spread rows and refinement error passes run
  on Rayon; `threads` caps the pool size.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading controls
- [`cytiq-density`](../cytiq-density) for the FSC/SSC gate
- [`cytiq-transform`](../cytiq-transform) for `scales::FlowJoBiexp`
- `nalgebra` / `ndarray` for linear algebra, `arrow` / `parquet` for Parquet
  export, `plotly` for the debug report, `rayon` for parallelism

## Tests

```bash
cargo test -p cytiq-autospill
cargo test -p cytiq-cli --test autospill_cli
```

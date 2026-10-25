# cytiq

`cytiq` is a Rust workspace for flow-cytometry tooling.

Current workspace crates:
- `cytiq-fcs`: strict FCS 3.0 / 3.1 / 3.2 reader
- `cytiq-autospill`: AutoSpill/AutoSpread workflow with optional debug plotting
- `cytiq-autospectral`: reference-based spectral unmixing workflow
- `cytiq-cytoscan`: CytoScan-style file-level anomaly detection
- `cytiq-gating`: Gating-ML 2.0 gate evaluation and export
- `cytiq-clean`: debris, doublet and dead-cell removal as per-event masks
- `cytiq-density`: shared kernel density and FSC/SSC auto-gating primitives
- `cytiq-peacoqc`: PeacoQC-style peak-based acquisition quality control
- `cytiq-flowsom`: FlowSOM-style cytometry clustering
- `cytiq-transform`: FCS signal transformation workflow
- `cytiq-compensate`: apply a spillover or compensation matrix to sample events
- `cytiq-mask`: per-event masks shared by stages (`masks.parquet`)
- `cytiq-meta`: FCS metadata export as JSON
- `cytiq-provenance`: per-run records of arguments, inputs and outputs with SHA-256
- `cytiq-manifest`: the run manifest (YAML/JSON): cytometry type, panels, samples, controls, QC policy
- `cytiq-preflight`: manifest and FCS checks before any computation
- `cytiq-qc`: panel QC policy checked on a sample's final event mask
- `cytiq-cli`: command-line entry point

## AutoSpill

The main user-facing command is:

```bash
cytiq autospill \
  --controls controls.csv \
  --input-dir ./controls \
  --output compensation.csv
```

Supported output formats:
- `csv`
- `tsv`
- `parquet`

Optional flags:

```bash
cytiq autospill \
  --controls controls.csv \
  --input-dir ./controls \
  --output spillover.parquet \
  --matrix-kind spillover \
  --detectors detectors.csv \
  --transform-file transforms.csv \
  --debug-dir ./debug \
  --fsc FSC-A \
  --ssc SSC-A \
  --threads 4
```

`cytiq autospread` uses the same manifest/input model:

```bash
cytiq autospread \
  --controls controls.csv \
  --input-dir ./controls \
  --detectors detectors.csv \
  --output spread.csv \
  --debug-dir ./debug \
  --threads 4
```

Logging:
- default level is `info` on an interactive terminal and `warn` otherwise
- override with `CYTIQ_LOG=debug` or `RUST_LOG=debug`
- logs are written to stderr

Performance:
- independent control loading, gating, spillover rows, spread rows, and refinement error passes use worker threads
- parallel execution uses Rayon defaults, which normally use all available logical CPUs
- use `--threads N` to cap worker threads for `autospill`, `autospread`, `autospectral`, `peacoqc`, or `flowsom`

Controls must be raw acquisition files. `autospill` and `autospread` apply
their own transforms, so a control another cytiq stage compensated, unmixed
or transformed is rejected.

### Controls Manifest

The controls manifest is a CSV with the following columns:

```csv
filename,dye
control_a.fcs,FITC
control_b.fcs,PE
unstained.fcs,Autofluorescence
```

Rules:
- `filename` is relative to `--input-dir`
- `dye` values must be unique
- `primary_channel` is inferred from FCS metadata by matching `dye` against `$PnN` first, then `$PnS`
- inferred channels are always exported/reported using the canonical short name from `$PnN`
- matching is case-insensitive and strips one trailing measurement suffix such as `-A`, `-H`, or `-W`
- `dye=Autofluorescence` is a reserved control name for synthetic autofluorescence handling
- for `Autofluorescence`, `--detectors` is required and the synthetic channel is inferred as the only detector not present in the FCS metadata
- legacy extra manifest columns are tolerated on read but ignored

Optional detectors manifest:

```csv
channel,label,wavelength
FL1-A,FITC detector,530
FL2-A,PE detector,585
FL3-A,APC detector,670
```

If omitted, detector order defaults to the inferred `primary_channel` order.

Optional transform manifest:

```csv
channel,length,max_range,pos,neg,width
FITC,256,262144,4.418539922,0,-100
PE,256,262144,4.418539922,0,-100
```

### Output Matrix

Default `autospill` export is the compensation matrix.

When detector order matches the inferred `primary_channel` order, export stays backward-compatible and square:
- rows and columns use the same channel names
- channel order follows the manifest order

When `--detectors` is provided, `autospill` can export rectangular matrices:
- spillover: rows are dyes, columns are detectors
- compensation/unmixing: rows are detectors, columns are dyes
- `autospread` exports a dye-by-detector spread matrix

When `--debug-dir` is provided, `cytiq` writes a single-page `report.html` plus `summary.json`; the report includes gating, scatter/regression, convergence, and matrix heatmaps in one sequential document.

## AutoSpectral

`cytiq autospectral` unmixed spectral FCS samples from single-stain controls with a compact CLI that follows the existing manifest/input model:

```bash
cytiq autospectral \
  --controls controls.csv \
  --samples samples.csv \
  --input-dir ./fcs \
  --output ./autospectral-out
```

Optional flags:

```bash
cytiq autospectral \
  --controls controls.csv \
  --samples samples.csv \
  --input-dir ./fcs \
  --output ./autospectral-out \
  --detectors detectors.csv \
  --debug-dir ./debug \
  --fsc FSC-A \
  --ssc SSC-A \
  --threads 4
```

Controls manifest:

```csv
filename,dye,primary_channel
controls/unstained.fcs,Autofluorescence,
controls/FITC.fcs,FITC,FITC
controls/PE.fcs,PE,PE
```

Samples manifest:

```csv
filename
samples/sample_01.fcs
samples/sample_02.fcs
```

Rules:
- manifest `filename` values are relative to `--input-dir`
- exactly one `dye=Autofluorescence` control is required
- controls and samples must be raw acquisition files: a file another cytiq
  stage compensated, unmixed or transformed is rejected
- single-stain `primary_channel` can be provided explicitly; otherwise it is inferred from FCS `$PnN` first, then `$PnS`
- AutoSpectral-style `fluorophore` and `channel` headers are accepted as aliases for `dye` and `primary_channel`
- if `--detectors` is omitted, spectral detectors are inferred as shared non-scatter, non-time channels
- independent FCS loading, reference spectra, sample unmixing, and per-event unmixing use worker threads
- reference spectra are peak-normalised (maximum detector = 1), as in upstream AutoSpectral, so unmixed abundances are in units of each fluorophore's peak detector
- positive events are those above the 99th percentile of the autofluorescence control's primary channel, keeping the brightest 500 (at least 20 required)
- the OLS solution is computed with a Householder QR solve (`R⁻¹Qᵀ`), not the normal equations `(MᵀM)⁻¹Mᵀ`, which square the condition number; `run.json` records `solver_method`
- autofluorescence component count, gate thresholds, minimum event counts, and condition-number limit are fixed internal constants for the MVP
- `--clean-controls` drops control events on a raw detector margin and gates FSC/SSC with the same KDE gate as `autospill` instead of the 1–99% quantile box; `run.json` records `control_gate`. It needs enough events for the KDE gate (a few hundred per control)
- unmixed FCS files keep the sample's acquisition keywords (`$CYT`, `$DATE`, ...) and the `$Pn*` keywords of passed-through channels such as scatter and time; the spillover matrix and earlier `CYTIQ_*` keywords are dropped because the detector channels they describe were replaced, except `CYTIQ_ACQUISITION`
- passed-through channels keep their exact values (the file uses doubles when one does not fit f32, for example `Original_ID` above 2^24); components are rounded to f32. `Time` and `Original_ID` are never inferred as detectors, and `Original_ID` is rejected as an explicit detector
- unmixed files record `CYTIQ_UNMIXING` (components and detectors) and the margin bounds of passed-through channels in `CYTIQ_MARGINS`. Components have no detector margins: `cytiq peacoqc --remove-margins` refuses them, and `cytiq gate` refuses to compensate them. Raw detector margins are written instead as `margin_pass` in `unmixed/<sample>.unmixed.masks.parquet`

Outputs are always written under `--output`:

```text
autospectral-out/
  references/
    fluorophore_spectra.csv
    autospectral-references.json
  unmixed/
    sample_01.unmixed.fcs
    sample_01.unmixed.masks.parquet
  diagnostics/
    run.json
```

## Transform

`cytiq transform` applies analysis-ready signal transformations to one or more
FCS files. The MVP supports FCS output and defaults to arcsinh:

```bash
cytiq transform \
  --input sample.fcs \
  --output sample.transformed.fcs
```

Multiple inputs can be passed explicitly or with a simple file-name glob:

```bash
cytiq transform \
  --input "samples/*.fcs" \
  --output transformed/

cytiq transform \
  --input sample-1.fcs \
  --input sample-2.fcs \
  --output transformed/
```

Optional flags:

```bash
cytiq transform \
  --input sample.fcs \
  --method arcsinh \
  --cofactor 150 \
  --logicle-t 262144 \
  --logicle-m 4.5 \
  --logicle-w 0.5 \
  --logicle-a 0 \
  --channels CD3,CD4,CD8 \
  --exclude-channels CD4 \
  --metadata-json transform.json \
  --debug-dir transform-debug \
  --dry-run \
  --threads 4 \
  --output sample.transformed.fcs
```

Rules:
- inputs must be provided with `--input` / `-i`; positional FCS inputs are not
  part of the public contract
- direct `.fcs` files, repeated inputs, directories with immediate `.fcs`
  children, and simple file-name globs such as `samples/*.fcs` are supported
- `--method` supports `arcsinh`, `logicle`, `biexponential`, `hyperlog`, and
  `none`; `arcsinh` uses `asinh(value / cofactor)` with default
  `--cofactor 150`
- `logicle` is the Moore & Parks logicle with the `flowCore::logicleTransform`
  output scale `[0, M]`; `hyperlog` is the Gating-ML hyperlog with the
  `flowCore::hyperlogtGml2` output scale `[0, 1]`. Both use `--logicle-t`,
  `--logicle-m`, `--logicle-w`, and `--logicle-a` for top-of-scale, decades,
  linear-region width, and additional negative decades, and reject parameters
  outside `2W <= M` and `-W <= A <= M - 2W`
- `biexponential` is the FlowJo biexponential as
  `flowWorkspace::flowjo_biexp(256, 262144, 4.418539922, 0, -100)` (the
  AutoSpill defaults), with output channels in `[0, 256]`
- FCS files and event columns are processed with Rayon; `--threads N` caps the
  worker thread count
- if `--channels` is omitted, obvious acquisition channels such as `Time`,
  `FSC-A/H/W`, and `SSC-A/H/W` are left untransformed
- `Original_ID` (event lineage written by `peacoqc`) is never transformed;
  naming it in `--channels` is an error. Untransformed columns keep their exact
  values, so the output is written with doubles when a value such as an event
  identifier above 2^24 does not fit f32; transformed values are always rounded
  to f32
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- output format is FCS for the MVP; CSV and parquet exports are follow-up work
- output files are not replaced unless `--overwrite` is passed
- transform metadata is stored in the output FCS `CYTIQ_TRANSFORM` keyword and
  can also be written as a sidecar JSON file
- an input that already has `CYTIQ_TRANSFORM` is rejected, so values are never
  transformed twice by accident; `--allow-retransform` transforms it anyway
- compensation comes first: a selected channel that is in the file's
  `$SPILLOVER`/`$SPILL` matrix (not yet applied) is rejected, because `cytiq
  compensate` refuses transformed channels afterwards. Run `cytiq compensate`
  first, or pass `--allow-uncompensated` for files that should not be
  compensated
- the output keeps the source keywords (`$CYT`, `$DATE`, `$PnV`, `$PnG`, ...)
  and the original `$PnR` of untransformed channels; transformed channels get
  a `$PnR` from their new values
- the spillover matrix (`$SPILLOVER`/`$SPILL`) no longer applies to transformed
  values, so it is moved to `CYTIQ_SOURCE_SPILL`
- detector margin bounds (flowCore `$PnR - 1` and the low bound) are mapped
  through the transform and stored per channel in `CYTIQ_MARGINS`, so
  `cytiq peacoqc --remove-margins` on the output removes the same events as on
  the raw file
- when `--debug-dir` is provided, `cytiq` writes `summary.json`,
  `metrics.csv`, and a Plotly `report.html` with per-channel transformation
  diagnostics

## Compensate

`cytiq compensate` applies a compensation matrix to the events of one FCS file
and writes the compensated FCS:

```bash
# the file's own $SPILLOVER/$SPILL keyword
cytiq compensate --input sample.fcs --output sample.comp.fcs

# a square matrix written by cytiq autospill
cytiq compensate \
  --input sample.fcs \
  --output sample.comp.fcs \
  --matrix autospill/compensation.csv \
  --matrix-kind compensation
```

Outputs:

```text
sample.comp.fcs             compensated events
sample.comp.masks.parquet   event_id, source_event_id, margin_pass, keep
```

Rules:
- `--matrix` takes a square `.csv`/`.tsv` with a corner label, the channel
  names as columns and one row per channel (the `cytiq autospill` square
  layout). `--matrix-kind` is required with it: `spillover` (inverted before
  use) or `compensation` (applied as is). Rectangular matrices are unmixing,
  not compensation, and are rejected
- matrix channels match `$PnN`, then `$PnS`; a missing channel is an error
- values are linearised first (`$PnE` log scaling and `$PnG` gain, as `cytiq
  gate` and FlowKit do), then compensated as `raw · compensation`. Other
  channels are copied unchanged, except that non-time channels with log
  scaling or gain are written linearised, because the output declares
  `$PnE=0,0`. Changed values are rounded to f32; unchanged channels keep their
  exact values, so the output uses doubles when one of them (an `Original_ID`
  above 2^24, 32-bit integer data) does not fit f32
- the output records `CYTIQ_COMPENSATION` (matrix source and channels) and
  keeps the source matrix only as `CYTIQ_SOURCE_SPILL`, so later stages do
  not compensate again. An input that `cytiq compensate` already compensated,
  that `cytiq autospectral` unmixed, or whose matrix channels `cytiq
  transform` transformed, is rejected
- compensation mixes channels, so raw detector margins cannot be identified
  afterwards. `margin_pass` in the mask flags events at or above the detector
  limit (`$PnR - 1`) on the raw values, and compensated channels are dropped
  from `CYTIQ_MARGINS`. Unlike `cytiq peacoqc --remove-margins`, which follows
  `PeacoQC::RemoveMargins` and also removes every event equal to a channel's
  observed minimum or maximum and every value at or below flowCore's low
  bound (−111), only the detector limit counts: discrete data (beads) keeps
  its extreme levels, and float data keeps the noise of negative populations,
  which on a real FACSDiva sample is 31% of events below −111. Pass the mask
  on with `--mask` (or combine
  it with `cytiq mask combine`) instead of removing margins after compensation
- existing outputs are not replaced unless `--overwrite` is passed

## Masks

Stages never delete events from the raw file. They describe per-event results
as masks, Parquet files with one row per event of the file they ran on:

| Column | Type | Meaning |
| --- | --- | --- |
| `event_id` | `UInt64` | 0-based row in that file |
| `source_event_id` | `UInt64` | 0-based row in the raw file (`Original_ID - 1` when the file has `Original_ID`) |
| filter columns | nullable `Boolean` | pass/fail; null means the filter did not assess the event |
| `keep` | `Boolean` | whether later stages should use the event |

`cytiq compensate`, `cytiq peacoqc`, `cytiq clean`, `cytiq autospectral` and
`cytiq gate --membership` write masks. `cytiq gate`, `cytiq flowsom` and
`cytiq clean` read them with `--mask` (repeatable). A mask applies to any file
derived from the same acquisition: its rows are matched through
`source_event_id`, and a file row missing from the mask is an error.

Each mask records which acquisition it belongs to in Parquet key-value
metadata:

| Key | Value |
| --- | --- |
| `cytiq.mask.schema_version` | `2` |
| `cytiq.acquisition_sha256` | SHA-256 of the raw FCS file |
| `cytiq.event_id_base` | `0` |

Every stage that writes FCS (`compensate`, `transform`, `peacoqc`,
`autospectral`) records the same hash in the `CYTIQ_ACQUISITION` keyword. A
file without the keyword is raw, and its own SHA-256 is the acquisition.
Combining masks from different acquisitions, or applying a mask to a file of
another acquisition, is an error. Masks without this metadata, written by
earlier cytiq versions, are rejected and must be regenerated.

`cytiq mask combine` joins masks of one acquisition into a single analysis
mask. An event is kept only if every mask has it and keeps it. Filter columns
are concatenated, and a repeated name gets the 1-based index of its mask as a
prefix (`2.margin_pass`):

```bash
cytiq mask combine \
  --mask sample.comp.masks.parquet \
  --mask sample.peacoqc.masks.parquet \
  --mask clean/masks.parquet \
  --output sample.analysis.parquet
```

A typical conventional chain:

```bash
cytiq meta       --input sample.fcs --output sample.meta.json
cytiq compensate --input sample.fcs --output sample.comp.fcs
cytiq transform  --input sample.comp.fcs --output sample.t.fcs
cytiq peacoqc    --input sample.t.fcs --output sample.peacoqc.fcs --channels CD3,CD4,CD8
cytiq clean      --input sample.fcs --output clean/     # raw scatter and viability
cytiq mask combine --mask sample.comp.masks.parquet --mask sample.peacoqc.masks.parquet \
  --mask clean/masks.parquet --output sample.analysis.parquet
cytiq qc         --manifest run.yaml --sample S01 --mask sample.analysis.parquet --output qc.json
cytiq gate       --input sample.t.fcs --gating-ml gates.xml --output counts.csv \
  --mask sample.analysis.parquet --membership membership.parquet
cytiq flowsom    --input sample.t.fcs --output flowsom.csv --channels CD3,CD4,CD8 \
  --mask sample.analysis.parquet
```

Each stage reads the `CYTIQ_COMPENSATION`, `CYTIQ_UNMIXING` and
`CYTIQ_TRANSFORM` keywords earlier stages wrote and checks its input:

| Stage | Expects | Otherwise |
| --- | --- | --- |
| `autospill`, `autospread`, `autospectral`, `cytoscan`, `preflight` | raw files | error |
| `compensate` | raw matrix channels | error when already compensated, unmixed, or a matrix channel is transformed |
| `transform` | compensated or unmixed channels | error when a selected channel's spillover is still pending (`--allow-uncompensated`), or the file is already transformed (`--allow-retransform`) |
| `clean` | raw scatter and viability channels | error |
| `peacoqc` | compensated, transformed channels | warning |
| `flowsom` | transformed channels | warning |
| `gate` | what the Gating-ML dimensions say | error when a dimension would compensate or transform a channel again |

Gating-ML strategies applied to `sample.t.fcs` must use `uncompensated`
dimensions without `transformation-ref` on the compensated and transformed
channels. Otherwise apply the strategy to the raw `sample.fcs`, which `cytiq
gate` compensates and transforms itself.

## CytoScan

`cytiq cytoscan` flags anomalous FCS files across a cohort. Outlier detection
uses only the test cohort; novelty detection compares test files to a separate
high-quality reference cohort.

```bash
cytiq cytoscan \
  --input ./test_fcs \
  --output ./cytoscan_results \
  --channels CD3,CD4,CD8,CD45
```

Novelty detection requires `--reference`:

```bash
cytiq cytoscan \
  --input ./test_fcs \
  --reference ./reference_fcs \
  --output ./cytoscan_results \
  --channel CD3 \
  --channel CD4 \
  --channel CD8 \
  --method quantiles \
  --flag-method novelty
```

Rules:
- `--input` and `--reference` accept direct `.fcs` files, directories with
  immediate `.fcs` children, and simple file-name globs such as `samples/*.fcs`
- pass channels either as repeated `--channel` values or as one compact
  comma-separated `--channels` value
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- `--method` supports `summary`, `quantiles`, `emd`, `binning`, and
  `fingerprint`; default is `quantiles`
- `--flag-method` supports `outlier`, `novelty`, and `both`; default is
  `outlier`
- novelty and `both` require `--reference`
- FCS preprocessing follows upstream `CytoScan::readInput`: values above
  `$PnR` are truncated on read, then selected channels are compensated with
  `SPILL` / `$SPILL` and transformed with `asinh(value / 150)`. Without
  spillover metadata (upstream fails) the channels stay raw and a warning is
  written to the summary
- each file contributes a 1000-event subsample drawn with a fixed seed (42),
  reseeded per file as upstream's `set.seed(42)`, so results do not depend on
  how paths are spelled; EMD, binning, and fingerprint use an aggregate of
  `ceiling(10000 / n_files)` events per file (reference cohort when novelty is
  enabled)
- binning uses `cut()` semantics (right-closed decile bins of the aggregate;
  values outside the aggregate range are dropped); fingerprint follows
  `flowFP` probability binning with 4 recursions
- outlier scores follow `isotree::isolation.forest(sample_size = 1, ntrees =
  1000, ndim = 1)`; files with score `>= 0.5` are outliers
- novelty fits Gaussian mixtures (`linfa-clustering`, full covariance,
  `G = 1..9`, mclust-style BIC) on the reference features; test files at or
  below the 5% quantile of reference log-densities are novelties. When there
  are no more reference files than features, a single diagonal Gaussian is
  used instead and reported as a warning
- low-level algorithm settings such as event sampling, aggregate size,
  isolation tree count, novelty quantile, and fingerprint recursions are fixed
  internal constants for the first public CLI; random draws use `rand` with a
  fixed seed, so outputs are reproducible but not bit-identical to R (see
  `parity/REPORT.md`)
- inputs must be raw FCS: CytoScan compensates with the file's spillover and
  arcsinh-transforms itself, so files from `cytiq compensate`, `cytiq
  autospectral` or `cytiq transform` are rejected
- a file without spillover metadata, or a selected channel missing from it,
  is scored on raw values with a warning (logged and in the `warnings`
  column). `--strict` makes both an error, so no file in the cohort is scored
  on a different scale

Outputs are always written under `--output`:

```text
cytoscan_results/
  cytoscan_summary.csv
  cytoscan_features.csv
  cytoscan_config.json
```

When `--debug-dir` is provided, `cytiq` also writes reference features,
aggregate sampling details, model diagnostics, PCA coordinates/loadings,
`feature_heatmap.csv`, `boxplot_samples.csv`, and a Plotly `report.html` with
PCA, top-loading, feature heatmap, and per-channel boxplot sections.

## Gating

`cytiq gate` applies an existing Gating-ML 2.0 gating strategy to one FCS file
and writes per-gate event counts. It evaluates gate boundaries that are already
defined; it does not estimate gates from the data. Automatic gate estimation
(flowDensity, openCyto) stays in the R/Python tools.

```bash
cytiq gate \
  --input sample.fcs \
  --gating-ml gates.xml \
  --output counts.csv \
  --format csv
```

Rules:
- supported gates: `RectangleGate` (range, rectangle, hyper-rectangle),
  `PolygonGate`, `EllipsoidGate` and `BooleanGate` (`and`, `or`, `not`,
  `use-as-complement`), nested through `parent_id`
- supported transformations: `flin`, `flog`, `fasinh`, `logicle` and
  `hyperlog`. Quadrant gates, `fratio` and `new-dimension` are rejected with
  an error
- `compensation-ref` can be `uncompensated`, `FCS` (the file's
  `$SPILLOVER`/`$SPILL`; treated as uncompensated when the file has neither),
  or the id of a square `transforms:spectrumMatrix`
- dimension names match `$PnN`, then `$PnS`, then the FlowJo spelling of
  `$PnN` (`/` replaced with `_`), then a fluorochrome of the referenced
  spectrum matrix
- events are pre-processed as FlowKit "raw" events: `$PnE` log scaling,
  `$PnG` gain and `$TIMESTEP` on the `Time` channel are applied before
  compensation
- on files from `cytiq transform`, `cytiq compensate` or `cytiq autospectral`,
  a dimension that would transform or compensate an already processed channel
  is an error, since Gating-ML applies its own compensation and
  transformation. Gate the raw file instead, or use `uncompensated` dimensions
  without `transformation-ref`
- `--mask` (repeatable, see [Masks](#masks)) restricts gating to events every
  mask keeps; events outside belong to no gate, and `percent_of_total` uses
  the masked event count
- `--membership membership.parquet` writes per-event membership in the mask
  layout: one nullable Boolean column per gate id in evaluation order, null
  outside the event mask, and `keep` marking events inside it
- membership semantics follow FlowKit/flowutils exactly:
  - rectangles use `min <= x < max`
  - polygons use the winding number with odd-winding interiors, so points on
    left/bottom edges are inside and points on right/top edges are outside
  - ellipsoids include their boundary
  - every gate is intersected with its parent
- gate ids must be valid XML `ID`s, so `CD3+` is invalid and something like
  `CD3pos` is needed
- `--format` is `csv` (default) or `json`. Each row holds `gate_id`,
  `gate_path`, `parent_id`, `gate_type`, `count`, `parent_count`,
  `percent_of_parent` and `percent_of_total`

The library API mirrors FlowKit:

```rust
let strategy = cytiq_gating::parse_gatingml(&std::fs::read_to_string("gates.xml")?)?;
let dataset = cytiq_fcs::FcsReader::open("sample.fcs")?.read()?;
let results = strategy.gate_sample(&dataset)?;
let mask: &[bool] = results.membership("CD3pos").unwrap();
let count = results.count("CD3pos").unwrap();
```

Strategies built in code (`GatingStrategy::new`) can be written back out with
`cytiq_gating::to_gatingml`. `logicle` and `hyperlog` transformations cannot be
written yet, because their original parameters are not kept in memory.

## Clean

`cytiq clean` removes debris, doublets and dead cells from one FCS file. It
runs three independent filters. Each one fits a gate from the data, or reports
why it could not:

| Filter | Gate id | Method | Needs |
| --- | --- | --- | --- |
| debris | `non_debris` | FSC-A threshold at the density valley above the low-FSC debris peak | `FSC-A` |
| singlets | `singlets` | `FSC-A / FSC-H` within `median ± k·MAD` (a wedge polygon), or `FSC-W <= median + k·MAD` | `FSC-H` or `FSC-W` |
| viability | `live` | threshold on the compensated, arcsinh-scaled dye: density valley between dim (live) and bright (dead) events, a control-file quantile, or an explicit value | a viability channel |

```bash
cytiq clean --input sample.fcs --output clean/
```

```
clean/
  masks.parquet       event_id, source_event_id, non_debris, singlets, live, keep
  metrics.json        per-filter status, method, fitted parameters, counts, overlaps
  gates.gatingml.xml  the fitted gates plus `clean` (Boolean AND of the applied ones)
  report.html         each gate drawn on its own dimensions
```

Rules:
- events are never copied. `event_id` is the 0-based row index in the input
  FCS, so the masks can be joined back to the original events.
  `source_event_id` is the 0-based row in the raw file: `Original_ID - 1` when
  the input carries `Original_ID` (for example PeacoQC output), otherwise
  equal to `event_id`.
- each filter reports `applied`, `not_assessed` or `disabled` in
  `metrics.json`, with a `reason`. The mask column of a filter that is not
  `applied` is null rather than all-true. `keep` is the AND of the applied
  filters only.
- scatter alone cannot identify dead cells, so without a viability dye the
  viability filter is `not_assessed`. It does not mark every event as live.
- the viability channel is auto-detected from `$PnN`/`$PnS` (Live/Dead,
  L/D, Zombie, Viability, Fixable, DAPI, 7-AAD, PI, propidium, SYTOX).
  Several candidates are an error; pass `--viability-channel` or
  `--no-viability`.
- the viability axis is `fasinh` with `T = cofactor · sinh(4.5 ln 10)`, which
  is `asinh(x / cofactor)` rescaled, after `FCS` compensation.
  `--viability-cofactor` defaults to 150.
- the viability threshold comes from one of three sources:
  - by default, the valley between the dimmest population and the next one;
    when there is no clear valley the filter is `not_assessed`
  - `--viability-control unstained.fcs` takes its
    `--viability-control-quantile` (default 0.995)
  - `--viability-threshold` sets it directly, in compensated intensity units
- adaptive debris needs a separate low-FSC peak, at most 0.25× the FSC
  position of the next population. Without one (for example when the
  acquisition threshold already cut the debris) the filter is `not_assessed`.
  `--debris-mode fixed --debris-fsc-min/--debris-ssc-min` sets the thresholds
  by hand.
- `--debris-mode population` uses the AutoSpill FSC/SSC auto-gate around the
  single dominant population. That suits beads and cell lines, but it drops
  minor cell populations in mixed samples.
- the singlet band is `median ± --singlet-mad · spread`, where spread is the
  MAD. When more than half of the values are identical (discretised scatter,
  beads) the MAD is zero, so `IQR / 1.349` is used, then `1e-3 · median`;
  `metrics.json` reports which (`spread_source`: `mad`, `iqr`, `relative`).
- `--require debris,singlets,viability` fails the run, before anything is
  written, when a listed filter is not `applied`. Without it a
  `not_assessed` filter simply does not contribute to `keep`.
- the viability axis compensates and arcsinh-scales the dye itself, and the
  debris and singlet filters need raw linear scatter, so an input whose
  viability or scatter channel `cytiq transform` transformed is rejected at
  once. Run clean on the raw file (or pass `--no-viability`) and combine the
  masks.
- singlets are fitted on non-debris events, and viability on non-debris
  singlets. Every gate is then evaluated on all events. Fitting uses at most
  `--max-fit-events` (default 50,000) evenly spaced events, so results are
  deterministic.
- masks are computed by evaluating `gates.gatingml.xml` itself, so
  `cytiq gate` on that file reproduces the counts exactly.
- `--mask` (repeatable, see [Masks](#masks)) takes masks from earlier stages:
  filters are fitted only on events every mask keeps, and `keep` is false for
  the others. Filter columns are still evaluated on every event.
  `metrics.json` gains `input_mask` with the mask paths and the kept count.

Manual corrections: edit `gates.gatingml.xml` (for example in FlowJo or by
hand), then re-apply it without refitting. The gate ids `non_debris`,
`singlets`, `live` and `clean` are looked up again. A missing gate makes its
filter `not_assessed`.

```bash
cytiq clean --input sample.fcs --gates clean/gates.gatingml.xml --output clean-v2/
```

## PeacoQC

`cytiq peacoqc` removes unstable acquisition regions from compensated and
transformed FCS files using a PeacoQC-compatible peak-based QC workflow:

```bash
cytiq peacoqc \
  --input sample.fcs \
  --output sample.peacoqc.fcs \
  --channel "355_378/28_uH-A [CD3 BUV395]" \
  --channel "405_750/40_vB-A [CD4 BV750]" \
  --channel "355_515/20_uF-A [CD8 BUV496]"
```

Optional flags:

```bash
cytiq peacoqc \
  --input sample.fcs \
  --output sample.peacoqc.fcs \
  --channel "355_378/28_uH-A [CD3 BUV395]" \
  --channel "405_750/40_vB-A [CD4 BV750]" \
  --method all \
  --remove-margins \
  --remove-zeros \
  --debug-dir ./peacoqc-debug \
  --threads 4
```

Rules:
- input and output are FCS files for the MVP
- PeacoQC expects compensated, transformed input. A file whose spillover
  matrix was not applied, or selected channels `cytiq transform` did not
  transform, still runs, with a warning in the log and in `summary.json`
- pass channels either as repeated `--channel` values or as one compact
  comma-separated `--channels` value
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- `--method` can be `all`, `mad`, `it`, or `peaks-only`
- `--remove-margins` removes detector-boundary events before PeacoQC, using
  flowCore's raw-detector bounds (`$PnR - 1` and the low bound, −111) as
  `PeacoQC::RemoveMargins` does. On float data
  the low bound flags ordinary noise around zero; the `margin_pass` mask of
  `cytiq compensate` uses the upper limit only. On files from
  `cytiq transform` the bounds come from `CYTIQ_MARGINS`, already mapped
  through the transform, so the same events are removed as on the raw file.
  A transformed file without `CYTIQ_MARGINS` (written by an older cytiq) is
  rejected rather than cut at its regenerated `$PnR`
- `--remove-zeros` ignores zero values during peak detection, which is useful
  for mass cytometry data
- PeacoQC tuning values such as MAD, IT limit, binning, peak removal, and
  consecutive-bin cleanup are fixed internal constants aligned with the
  reference defaults
- the cleaned FCS keeps every source keyword, including the spillover matrix
  and the original `$PnR`, since event values are unchanged. It adds an
  `Original_ID` column (1-based raw row) unless the input already has one.
  The file is written as 64-bit floats when 32-bit floats would change a
  value, for example `Original_ID` above 2^24
- `<output>.masks.parquet` (for example `sample.peacoqc.masks.parquet`) holds
  one row per input event: `event_id` (0-based input row),
  `source_event_id` (0-based raw row), `margin_pass`, `peacoqc_pass` and
  `keep`. A null filter value means the event was not assessed by it

When `--debug-dir` is provided, `cytiq` writes:

```text
peacoqc-debug/
  summary.json
  mask.csv
  report.html
```

The HTML report uses Plotly for interactive removal-summary and acquisition
order diagnostics.

## FlowSOM

`cytiq flowsom` clusters events from one FCS file using a deterministic
FlowSOM-style workflow and writes per-event SOM assignments:

```bash
cytiq flowsom \
  --input sample.fcs \
  --output sample.flowsom.csv \
  --channel CD3 \
  --channel CD4 \
  --channel CD8
```

The compact channel form is equivalent:

```bash
cytiq flowsom \
  --input sample.fcs \
  --output sample.flowsom.csv \
  --channels CD3,CD4,CD8 \
  --debug-dir ./flowsom-debug \
  --threads 4
```

Rules:
- input is a single FCS file for the MVP
- output is a per-event CSV assignment table with `event_index` (0-based input
  row), `source_event_id` (0-based raw row, from `Original_ID` when present),
  `flowsom_node`, and `flowsom_metacluster`
- `--mask` (repeatable, see [Masks](#masks)) clusters only events every mask
  keeps: scaling, training and assignment use those rows, and only they are
  listed in the output
- selected channels that `cytiq transform` did not transform are clustered as
  given, with a warning: FlowSOM distances assume transformed values
- pass channels either as repeated `--channel` values or as one compact
  comma-separated `--channels` value
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- `--xdim`/`--ydim` (SOM grid, default 10×10), `--metaclusters` (default 20,
  at most `xdim·ydim`), `--seed` (default 1) and `--no-scale` (skip z-scoring)
  default to the previous fixed values, so runs without them are unchanged
- `--training-events N` trains the SOM on a deterministic sample of N events
  (without replacement, from its own RNG stream) and then assigns every event
  to its nearest node; scaling statistics still use every event. It bounds
  training time on large files
- training iterations, learning-rate schedule and radius schedule stay fixed
  internal constants

When `--debug-dir` is provided, `cytiq` writes:

```text
flowsom-debug/
  summary.json
  nodes.csv
  metaclusters.csv
  report.html
```

The HTML report uses Plotly for SOM-node, metacluster, and assignment
diagnostics.

## Meta

`cytiq meta` writes the metadata of an FCS file as one JSON document: file
identity (path, size, SHA-256), FCS version, segment offsets, event count,
parameters with their `$Pn*` keywords, and every TEXT keyword in file order.

```bash
# stdout (logs go to stderr, so piping is safe)
cytiq meta --input raw.fcs | jq '.text | length'

# file
cytiq meta --input raw.fcs --output raw.meta.json
```

Rules:
- omit `--output` or pass `--output -` to write to stdout
- output files are not replaced unless `--overwrite` is passed; the file is
  written through a temporary sibling and renamed, so it is never left
  half-written
- `text` and `supplemental_text` are `[key, value]` pairs exactly as stored,
  including `$SPILLOVER` and vendor keywords

Shape:

```json
{
  "schema_version": 1,
  "tool": "cytiq meta",
  "version": "0.1.0",
  "file": { "path": "raw.fcs", "name": "raw.fcs", "bytes": 96813, "sha256": "..." },
  "fcs_version": "FCS3.1",
  "segments": { "text": [256, 812], "data": [813, 96812], "analysis": null },
  "event_count": 4000,
  "parameter_count": 6,
  "parameters": [
    { "index": 1, "name": "FSC-A", "label": null, "data_type": "float",
      "width": { "bits": 32 }, "keywords": { "$P1B": "32", "$P1N": "FSC-A", "...": "..." } }
  ],
  "text": [["$CYT", "..."], ["$DATE", "..."]],
  "supplemental_text": null
}
```

FCS files written by `transform`, `peacoqc` and `autospectral` carry over the
source's instrument and acquisition keywords and the `$Pn*` keywords of each
channel, plus their own (`CYTIQ_TRANSFORM`, `CYTIQ_MARGINS`). Keywords that no
longer describe the written values are dropped or renamed. For example
`transform` moves the spillover matrix to `CYTIQ_SOURCE_SPILL`. `$PnB`, `$PnE`
and `$DATATYPE` always describe the written float data. The
`Original_ID` column of a PeacoQC output is the 1-based event row of that run's
input file, so per-event results computed on the cleaned file (for example
FlowSOM `event_index`, which is 0-based over the cleaned rows) map back to the
input through `Original_ID`, not through their own row number.

## Preflight

`cytiq preflight` checks a run before any computation. The run is described
by one manifest, YAML or JSON:

```yaml
cytometry: conventional        # or spectral
panel:
  name: T-cell-8c
  markers:
    - { marker: CD3, channel: FL1-A }        # channel optional: matched by $PnS
    - { marker: Live/Dead, required: false }  # required defaults to true
samples:
  - { id: S01, file: fcs/s01.fcs }            # relative to the manifest
controls:
  - { file: fcs/unstained.fcs, dye: Autofluorescence }
  - { file: fcs/cd3_fitc.fcs, dye: FITC, primary_channel: FL1-A }
```

A run with several panels lists them under `panels`, each with a `name`, and
links every sample to its panel. A control without `panel` belongs to every
panel; `specimen` groups the files of one biological sample across panels:

```yaml
cytometry: conventional
panels:
  - name: T
    markers: [{ marker: CD3 }, { marker: CD4 }]
    compensation:                 # conventional only; default { source: fcs }
      source: matrix              # fcs | matrix | controls
      matrix: autospill/t.csv     # relative to the manifest
      kind: compensation          # spillover | compensation, as for compensate --matrix-kind
  - name: B
    markers: [{ marker: CD19 }]
    compensation: { source: controls }   # autospill from the panel's single stains
  - name: NK
    markers: [{ marker: CD56 }]
    signal_state: compensated     # raw (default) | compensated | unmixed
samples:
  - { id: S01-T, file: fcs/s01_t.fcs, panel: T, specimen: S01 }
  - { id: S01-B, file: fcs/s01_b.fcs, panel: B, specimen: S01 }
  - { id: S01-NK, file: export/s01_nk.fcs, panel: NK, specimen: S01 }
controls:
  - { file: fcs/unstained.fcs, dye: Autofluorescence }
  - { file: fcs/cd19_pe.fcs, dye: PE, primary_channel: FL2-A, panel: B }
```

`signal_state` says what a panel's sample files hold. cytiq stages recognise
only their own processing (the `CYTIQ_*` keywords), and FCS has no keyword for
compensation or unmixing done by other software, so files exported already
compensated (for example from FlowJo) or unmixed (by spectral acquisition
software) must be declared: `compensated` for conventional panels, `unmixed`
for spectral ones.
Such panels take no `compensation` and need no controls; controls are always
raw.

```bash
cytiq preflight --manifest run.yaml --output preflight.json
```

Checks, each reported as `pass`, `warn` or `fail` (full table in
[`cytiq-preflight`](crates/cytiq-preflight/README.md)):
- every file parses, has events, and is raw: files already processed by
  `cytiq transform`, `compensate` or `autospectral` fail
- every file fits its declared `signal_state`: a raw file with `Comp-<detector>`
  channels (FlowJo's name for compensated values) fails without the detector
  itself and warns next to it; a `compensated` file that still carries
  `$SPILLOVER`/`$SPILL` fails, because `cytiq compensate`, the viability
  filter of `cytiq clean`, `FCS` references in `cytiq gate` and `cytiq
  cytoscan` would compensate it again; an `unmixed` file with one warns
- every required marker of a sample's panel maps to exactly one channel
  (`$PnS` or `$PnN`; a marker found on several channels fails)
- the samples and controls of each panel share one detector set (`$PnN`);
  different `$PnV` voltages warn. Panels of compensated or unmixed samples
  compare their samples only
- conventional compensation of raw samples, by the panel's `compensation.source`:
  - `fcs` (default): each sample carries a spillover matrix over existing
    channels that is invertible; a 1-norm condition number above 100 warns
  - `matrix`: the matrix file (for example from `cytiq autospill`) is square
    and invertible, its condition number is checked the same way, and every
    sample has its channels; the samples need no `$SPILLOVER`
  - `controls`: every marker channel has a single-stain control with that
    `primary_channel`
- spectral panels of raw samples: every marker with a `channel` has a
  single-stain control (warns otherwise)
- a panel's controls have distinct dyes
- missing `$CYT`/`$DATE` warns

The JSON report is always written; the command exits with `1` when any check
fails, and logs every failure and warning.

## QC

`cytiq qc` checks one sample's final event mask against the QC policy of its
panel in the run manifest. It reads the mask, not each stage's report: the
combined mask already holds every filter column (`margin_pass`,
`peacoqc_pass`, `non_debris`, `singlets`, `live`, ...) and `keep`.

```yaml
panel:
  name: T-cell-8c
  markers: [...]
  qc:
    min_events: 10000             # raw events in the sample file
    min_kept_events: 3000         # events the final mask keeps
    max_removed_fraction: 0.6     # 1 - kept / raw events
    filters:                      # key = mask filter column
      margin_pass:  { required: true, max_removed_fraction: 0.05 }
      peacoqc_pass: { required: true, max_removed_fraction: 0.3 }
      non_debris:   { required: true }
      singlets:     { required: true }
      live:         { required: true, min_assessed_fraction: 0.95, max_removed_fraction: 0.5, severity: warn }
```

```bash
cytiq qc --manifest run.yaml --sample S01 --mask sample.analysis.parquet --output qc.json
```

Rules:
- the mask must belong to the sample's raw acquisition and hold exactly one
  row per raw event: none missing (`cytiq mask combine` follows the rows of
  its first mask, so put the `compensate` or `clean` mask first) and none for
  events the raw file does not have; otherwise the check fails and no
  metrics are computed
- a filter counts as removing an event when any of its columns is `false`;
  `2.margin_pass` and `margin_pass` (the names `cytiq mask combine` gives a
  repeated filter) are one filter. Its removed fraction is relative to the
  events it assessed (non-null)
- `required: true` fails when the filter is missing or assessed nothing (a
  `not_assessed` clean filter). `min_assessed_fraction` fails when it
  assessed a smaller share of the raw events, which catches a filter that ran
  on part of them only. The share is of raw events: the mask cannot tell an
  event that never reached a filter from one it skipped, so a filter that only
  saw what an earlier stage kept needs a bound that allows for it.
  `severity: warn` turns that filter's failures into warnings. `min_events`,
  `min_kept_events` and `max_removed_fraction` always fail
- a panel without `qc` passes with a note
- `qc.json` holds `metrics` (raw and kept events, removed fraction, per
  filter: columns, assessed, assessed fraction, removed, removed fraction)
  and `checks` (`scope`, `check`, `status`, `message`, `value`, `limit`). It
  is always written; the command exits with `1` when any check fails

## Provenance

Every command that writes files, except `meta`, also writes a provenance
record next to its output: `<output>.provenance.json` for a file output and
`<dir>/provenance.json` for a directory output (schema version 2). It holds:
- the tool, version and `build` (git commit, rustc, target, profile) and the
  `platform` (OS, architecture, logical CPUs, the default thread count)
- the exact command-line `arguments`, and `config`: every option of the
  command with its default applied, keyed by option name in snake_case
- `constants`: fixed algorithm constants that shape the result but are no
  options (PeacoQC, CytoScan, FlowSOM training), and `algorithms`: the
  upstream implementations and versions the command is validated against
- start and finish times
- the size, SHA-256 and `role` (`input`, `mask`, `gating_ml`, `matrix`,
  `manifest`, `control`, `reference`, ...) of every input, and the size and
  SHA-256 of every output (debug directories are not listed)

Inputs are hashed before the command runs, so a command that replaces its own
input (`transform --overwrite` with the same input and output) records what it
read; an input that changes during the run and is not also an output fails
the command:

```json
{
  "schema_version": 2,
  "tool": "cytiq gate",
  "version": "0.1.0",
  "build": { "git_commit": "b781ef4…", "rustc": "rustc 1.99.1 …", "target": "x86_64-unknown-linux-gnu", "profile": "release" },
  "platform": { "os": "linux", "arch": "x86_64", "cpus": 16 },
  "arguments": ["gate", "--input", "s.t.fcs", "--gating-ml", "gates.xml", "--output", "counts.csv"],
  "config": { "input": "s.t.fcs", "gating_ml": "gates.xml", "output": "counts.csv", "format": "csv", "masks": [], "membership": null, "threads": null },
  "constants": null,
  "algorithms": { "FlowKit": "1.3.0", "Gating-ML": "2.0" },
  "started_at_unix_ms": 1792790000000,
  "finished_at_unix_ms": 1792790000120,
  "inputs": [{ "path": "s.t.fcs", "role": "input", "bytes": 640905, "sha256": "…" }, { "path": "gates.xml", "role": "gating_ml", "…": "…" }],
  "outputs": [{ "path": "counts.csv", "bytes": 214, "sha256": "…" }]
}
```

`build.git_commit` is the commit the binary was built from (`git rev-parse
HEAD` at build time, or `CYTIQ_BUILD_COMMIT` where the sources have no
`.git`, as in the Docker build); uncommitted changes are not detected.

Following the `inputs` of each record back to the outputs of earlier records
reconstructs the whole chain from the raw FCS files.

## Algorithm Notes

`cytiq-autospill` follows the published AutoSpill workflow and upstream R
reference closely for the implemented scope (the spillover matrix matches
`autospill` to about 1e-13 on the synthetic parity datasets and on real
FACSCelesta and FACSymphony controls, `parity/REPORT.md`):
- controls read as `flowCore::read.FCS` reads them: values above `$PnR` are
  capped at `$PnR`
- FSC/SSC auto-gating
- robust spillover estimation
- iterative matrix refinement
- linear to transformed refinement
- square and rectangular spillover/unmixing export
- AutoSpread estimation
- Plotly debug artifacts

`cytiq-peacoqc` follows the PeacoQC paper and upstream R implementation for
the MVP scope:
- optional margin removal
- overlapping acquisition-order bins
- per-channel peak detection on a port of R's `stats::density` and
  peak-feature matrix construction
- MAD filtering on a port of `smooth.spline(spar = 0.5)`, deterministic
  Isolation Tree splitting, and consecutive-bin cleanup
- cleaned FCS export with `Original_ID`
- JSON/mask outputs and Plotly HTML diagnostics
- the good-event mask matches `PeacoQC::PeacoQC` exactly on the parity
  datasets (`parity/REPORT.md`)

`cytiq-flowsom` follows the FlowSOM paper and upstream R implementation for
the MVP scope:
- selected marker matrix extraction from FCS files
- fixed z-score preprocessing
- self-organizing map training following `FlowSOM`'s `som.c` (random event
  draws from a fixed-seed generator, bubble neighbourhood on the Chebyshev
  grid distance, radius decaying from the 0.67 grid-distance quantile to 0)
- deterministic hierarchical metaclustering over SOM nodes
- per-event node/metacluster assignment CSV export
- JSON, node/metacluster CSV outputs, and Plotly HTML diagnostics

`cytiq-transform` currently implements deterministic signal transforms:
- `arcsinh`, `logicle`, `biexponential`, `hyperlog`, and `none`, matching
  `flowCore`/`flowWorkspace` numerically (see `parity/REPORT.md`)
- parallel FCS file and event-column transformation with Rayon
- explicit channel inclusion and exclusion
- FCS output with names, labels, transform metadata, and optional Plotly debug
  artifacts

Primary reference:
- AutoSpill paper: <https://www.nature.com/articles/s41467-021-23126-8>
- PeacoQC paper: <https://doi.org/10.1002/cyto.a.24501>
- FlowSOM paper: <https://doi.org/10.1002/cyto.a.22625>

Upstream R implementation:
- <https://github.com/carlosproca/autospill>
- <https://github.com/saeyslab/PeacoQC>
- <https://github.com/saeyslab/FlowSOM>

## Data

### Checked-in fixtures

Everything under [`data/`](data/) is **small synthetic test data** with a
known answer; it is used by the unit and CLI tests and is safe to commit. No
real acquisitions are stored in the repository.

| Directory | Used by | Contents |
|---|---|---|
| [`data/fixtures/fcs/`](data/fixtures/fcs/) | `cytiq-fcs` reader tests | FCS 3.0 / 3.1 / 3.2 files with integer, float, and double channels ([README](data/fixtures/fcs/README.md)) |
| [`data/fixtures/autospill/`](data/fixtures/autospill/) | `cytiq autospill` | two single-colour controls, [`controls.csv`](data/fixtures/autospill/controls.csv), expected [spillover](data/fixtures/autospill/expected_spillover.csv) and [compensation](data/fixtures/autospill/expected_compensation.csv) matrices ([README](data/fixtures/autospill/README.md)) |
| [`data/fixtures/autospread/`](data/fixtures/autospread/) | `cytiq autospread` | two controls, [`controls.csv`](data/fixtures/autospread/controls.csv), [`detectors.csv`](data/fixtures/autospread/detectors.csv), expected [spread matrix](data/fixtures/autospread/expected_spread.csv) ([README](data/fixtures/autospread/README.md)) |
| [`data/fixtures/autospectral/`](data/fixtures/autospectral/) | `cytiq autospectral` | unstained + FITC + PE controls, fully stained `sample.fcs`, [`controls.csv`](data/fixtures/autospectral/controls.csv), [`samples.csv`](data/fixtures/autospectral/samples.csv) ([README](data/fixtures/autospectral/README.md)) |
| [`data/fixtures/clean/`](data/fixtures/clean/) | `cytiq clean` tests | synthetic `sample.fcs` with known debris, doublets and dead cells, `no_viability.fcs`, an unstained `control.fcs`, and per-event [`truth.json`](data/fixtures/clean/truth.json) ([README](data/fixtures/clean/README.md)) |
| [`data/fixtures/pipeline/`](data/fixtures/pipeline/) | end-to-end chain test (`crates/cytiq-cli/tests/pipeline_cli.rs`) | golden raw `sample.fcs` (15,000 events, 6×6 spillover, seven populations incl. a rare one, debris, doublets, dead cells, an unstable interval, saturation), per-event truth, population gates, a run manifest and acceptance [`thresholds.json`](data/fixtures/pipeline/thresholds.json) ([README](data/fixtures/pipeline/README.md)) |
| [`data/fixtures/gating/`](data/fixtures/gating/) | `cytiq-gating` / `cytiq gate` tests | synthetic `sample.fcs`, a Gating-ML 2.0 [`gates.xml`](data/fixtures/gating/gates.xml) covering every supported gate and transform, and FlowKit [reference membership](data/fixtures/gating/flowkit_reference.json) ([README](data/fixtures/gating/README.md)) |
| [`data/fixtures/peacoqc/`](data/fixtures/peacoqc/) | `cytiq peacoqc` tests | [`sample_events.csv`](data/fixtures/peacoqc/sample_events.csv) (turned into FCS by the CLI tests), R reference values for the [`density`](data/fixtures/peacoqc/density_reference.rs) and [`smooth.spline`](data/fixtures/peacoqc/smooth_spline_reference.rs) ports ([README](data/fixtures/peacoqc/README.md)) |
| [`data/fixtures/transform/`](data/fixtures/transform/) | `cytiq-transform` tests | [flowCore / flowWorkspace reference values](data/fixtures/transform/flowcore_reference.rs) for logicle, hyperlog, and `flowjo_biexp` |

The `*_reference.rs` files are generated by
[`parity/make_fixtures.R`](parity/make_fixtures.R); regenerate them with
`R_LIBS_USER=parity/.env/Rlib Rscript parity/make_fixtures.R` after
`parity/setup_env.sh`.

Quick runs against the fixtures:

```bash
cytiq autospill --controls data/fixtures/autospill/controls.csv \
  --input-dir data/fixtures/autospill --output compensation.csv

cytiq autospread --controls data/fixtures/autospread/controls.csv \
  --input-dir data/fixtures/autospread \
  --detectors data/fixtures/autospread/detectors.csv --output spread.csv

cytiq autospectral --controls data/fixtures/autospectral/controls.csv \
  --samples data/fixtures/autospectral/samples.csv \
  --input-dir data/fixtures/autospectral --output ./autospectral-out

cytiq transform -i data/fixtures/fcs/fcs3_2_mixed.fcs -o mixed.transformed.fcs

cytiq gate --input data/fixtures/gating/sample.fcs \
  --gating-ml data/fixtures/gating/gates.xml --output gate_counts.csv

cytiq clean --input data/fixtures/clean/sample.fcs --output ./clean-out

cytiq flowsom --input data/fixtures/autospectral/sample.fcs \
  --output sample.flowsom.csv --channels FITC,PE,AF
```

### Public real datasets

Real acquisitions are not redistributed here; download them from their
original sources:
- AutoSpill paper datasets on FlowRepository (single-colour controls plus
  `fcs_control.csv`):
  [MM1 (FR-FCM-Z2SS)](https://flowrepository.org/id/FR-FCM-Z2SS),
  [HS1 & HS2 (FR-FCM-Z2ST)](https://flowrepository.org/id/FR-FCM-Z2ST),
  [Be1 (FR-FCM-Z2SV)](https://flowrepository.org/id/FR-FCM-Z2SV).
  The `dye` column of `fcs_control.csv` names the `$PnN` channel, so
  `filename,dye` can be used directly as the `cytiq autospill` manifest.
- AutoSpectral worked example (9-colour panel, 5-laser Cytek Aurora; spleen,
  lung, and liver), linked from the
  [AutoSpectral repository](https://github.com/DrCytometer/AutoSpectral).

### Parity datasets

[`parity/`](parity/) generates larger synthetic datasets with ground truth
(spillover matrices, spectra, QC artefact masks, cluster labels, and a golden
end-to-end sample) and compares `cytiq` against the upstream R tools and
FlowKit, tool by tool and along the whole chain. It also runs on real
acquisitions that ship with the R packages, with FlowJo labels as a reference
([`parity/real/`](parity/real/README.md)). Results are in
[`parity/REPORT.md`](parity/REPORT.md); metrics are in
[`parity/results/`](parity/results/). Every reported number is checked
against [`parity/tolerances.json`](parity/tolerances.json); a violation fails
the run.

```bash
parity/setup_env.sh      # once: isolated R library and Python venv under parity/.env
just parity              # synthetic data -> cytiq -> R/FlowKit -> parity/results/metrics.json
just parity-real         # real acquisitions -> parity/results/metrics_real.json
```

The same end-to-end chain runs in CI on the committed golden fixture
(`data/fixtures/pipeline`).

## Docker

```bash
just docker-build        # docker build -f docker/Dockerfile -t cytiq:dev .
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/data" cytiq:dev autospill \
  --controls data/fixtures/autospill/controls.csv \
  --input-dir data/fixtures/autospill --output compensation.csv
```

The image runs as an unprivileged user (uid 10001) with `/data` as the
working directory; `--user "$(id -u):$(id -g)"` makes outputs in the mounted
directory belong to you.

## Development

Use `just` for the standard local workflows:

```bash
just build
just build-release
just test
just verify
```

Release builds use full LTO, one codegen unit, stripped symbols, and aborting panics for optimized CLI binaries.

`just verify` runs formatting, `cargo check`, `clippy`, and the full test suite.

# cytiq

`cytiq` is a Rust workspace for flow-cytometry tooling.

Current workspace crates:
- `cytiq-fcs`: strict FCS 3.0 / 3.1 / 3.2 reader
- `cytiq-autospill`: AutoSpill/AutoSpread workflow with optional debug plotting
- `cytiq-autospectral`: reference-based spectral unmixing workflow
- `cytiq-peacoqc`: PeacoQC-style peak-based acquisition quality control
- `cytiq-flowsom`: FlowSOM-style cytometry clustering
- `cytiq-transform`: FCS signal transformation workflow
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
- single-stain `primary_channel` can be provided explicitly; otherwise it is inferred from FCS `$PnN` first, then `$PnS`
- AutoSpectral-style `fluorophore` and `channel` headers are accepted as aliases for `dye` and `primary_channel`
- if `--detectors` is omitted, spectral detectors are inferred as shared non-scatter, non-time channels
- independent FCS loading, reference spectra, sample unmixing, and per-event unmixing use worker threads
- reference spectra are peak-normalised (maximum detector = 1), as in upstream AutoSpectral, so unmixed abundances are in units of each fluorophore's peak detector
- positive events are those above the 99th percentile of the autofluorescence control's primary channel, keeping the brightest 500 (at least 20 required)
- solver, autofluorescence component count, gate thresholds, minimum event counts, and condition-number limit are fixed internal constants for the MVP

Outputs are always written under `--output`:

```text
autospectral-out/
  references/
    fluorophore_spectra.csv
    autospectral-references.json
  unmixed/
    sample_01.unmixed.fcs
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
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- output format is FCS for the MVP; CSV and parquet exports are follow-up work
- output files are not replaced unless `--overwrite` is passed
- transform metadata is stored in the output FCS `CYTIQ_TRANSFORM` keyword and
  can also be written as a sidecar JSON file
- when `--debug-dir` is provided, `cytiq` writes `summary.json`,
  `metrics.csv`, and a Plotly `report.html` with per-channel transformation
  diagnostics

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
- pass channels either as repeated `--channel` values or as one compact
  comma-separated `--channels` value
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- `--method` can be `all`, `mad`, `it`, or `peaks-only`
- `--remove-margins` removes detector-boundary events before PeacoQC
- `--remove-zeros` ignores zero values during peak detection, which is useful
  for mass cytometry data
- PeacoQC tuning values such as MAD, IT limit, binning, peak removal, and
  consecutive-bin cleanup are fixed internal constants aligned with the
  reference defaults

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
- output is a per-event CSV assignment table with `event_index`,
  `flowsom_node`, and `flowsom_metacluster`
- pass channels either as repeated `--channel` values or as one compact
  comma-separated `--channels` value
- channel values match `$PnN` first, `$PnS` labels as a fallback, and display
  identifiers like `$PnN [$PnS]`
- SOM size, metacluster count, training iterations, learning-rate schedule,
  radius schedule, random seed, and input scaling are fixed internal constants
  for the MVP

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

## Algorithm Notes

`cytiq-autospill` follows the published AutoSpill workflow and upstream R
reference closely for the implemented scope (the spillover matrix matches
`autospill` to about 1e-13 on the parity datasets, `parity/REPORT.md`):
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

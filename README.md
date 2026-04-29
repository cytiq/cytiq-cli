# cytiq

`cytiq` is a Rust workspace for flow-cytometry tooling.

Current workspace crates:
- `cytiq-fcs`: strict FCS 3.0 / 3.1 / 3.2 reader
- `cytiq-autospill`: AutoSpill/AutoSpread workflow with optional debug plotting
- `cytiq-autospectral`: reference-based spectral unmixing workflow
- `cytiq-peacoqc`: PeacoQC-style peak-based acquisition quality control
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
- use `--threads N` to cap worker threads for `autospill`, `autospread`, or `autospectral`

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

## Algorithm Notes

`cytiq-autospill` follows the published AutoSpill workflow and upstream R
reference closely for the implemented scope:
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
- per-channel peak detection and peak-feature matrix construction
- MAD filtering, deterministic Isolation Tree-style splitting, and
  consecutive-bin cleanup
- cleaned FCS export with `Original_ID`
- JSON/mask outputs and Plotly HTML diagnostics

Primary reference:
- AutoSpill paper: <https://www.nature.com/articles/s41467-021-23126-8>
- PeacoQC paper: <https://doi.org/10.1002/cyto.a.24501>

Upstream R implementation:
- <https://github.com/carlosproca/autospill>
- <https://github.com/saeyslab/PeacoQC>

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

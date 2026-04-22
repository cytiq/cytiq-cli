# cytiq

`cytiq` is a Rust workspace for flow-cytometry tooling.

Current workspace crates:
- `cytiq-fcs`: strict FCS 3.0 / 3.1 / 3.2 reader
- `cytiq-autospill`: AutoSpill/AutoSpread workflow with optional debug plotting
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
  --ssc SSC-A
```

`cytiq autospread` uses the same manifest/input model:

```bash
cytiq autospread \
  --controls controls.csv \
  --input-dir ./controls \
  --detectors detectors.csv \
  --output spread.csv \
  --debug-dir ./debug
```

Logging:
- default level is `info` on an interactive terminal and `warn` otherwise
- override with `CYTIQ_LOG=debug` or `RUST_LOG=debug`
- logs are written to stderr

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

Primary reference:
- AutoSpill paper: <https://www.nature.com/articles/s41467-021-23126-8>

Upstream R implementation:
- <https://github.com/carlosproca/autospill>

## Development

Use `just` for the standard local workflows:

```bash
just build
just test
just verify
```

`just verify` runs formatting, `cargo check`, `clippy`, and the full test suite.

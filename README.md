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

### Controls Manifest

The controls manifest is a CSV with the following columns:

```csv
filename,dye,primary_channel,antigen,control_kind,wavelength
control_a.fcs,FITC,FL1-A,CD3,single_color,530
control_b.fcs,PE,FL2-A,CD19,single_color,585
unstained.fcs,Autofluorescence,FL3-A,,autofluorescence,670
```

Rules:
- `filename` is relative to `--input-dir`
- `dye` values must be unique
- `primary_channel` values must be unique
- `antigen` is optional metadata only
- `control_kind` is optional and defaults to `single_color`
- `wavelength` is optional metadata used in labels/debugging
- for `autofluorescence`, `primary_channel` may be a synthetic detector name present in `--detectors`

Optional detectors manifest:

```csv
channel,label,wavelength
FL1-A,FITC detector,530
FL2-A,PE detector,585
FL3-A,APC detector,670
```

If omitted, detector order defaults to the manifest `primary_channel` order.

Optional transform manifest:

```csv
channel,length,max_range,pos,neg,width
FITC,256,262144,4.418539922,0,-100
PE,256,262144,4.418539922,0,-100
```

### Output Matrix

Default `autospill` export is the compensation matrix.

When detector order matches the manifest `primary_channel` order, export stays backward-compatible and square:
- rows and columns use the same channel names
- channel order follows the manifest order

When `--detectors` is provided, `autospill` can export rectangular matrices:
- spillover: rows are dyes, columns are detectors
- compensation/unmixing: rows are detectors, columns are dyes
- `autospread` exports a dye-by-detector spread matrix

When `--debug-dir` is provided, Plotly HTML artifacts and `summary.json` are written for gating, scatter/regression, convergence, and matrix heatmaps.

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

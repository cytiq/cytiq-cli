# cytiq

`cytiq` is a Rust workspace for flow-cytometry tooling.

Current workspace crates:
- `cytiq-fcs`: strict FCS 3.0 / 3.1 / 3.2 reader
- `cytiq-autospill`: narrow Rust port of the core AutoSpill workflow
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
  --fsc FSC-A \
  --ssc SSC-A
```

### Controls Manifest

The controls manifest is a CSV with the following columns:

```csv
filename,dye,primary_channel,antigen
control_a.fcs,FITC,FL1-A,CD3
control_b.fcs,PE,FL2-A,CD19
```

Rules:
- `filename` is relative to `--input-dir`
- `dye` values must be unique
- `primary_channel` values must be unique
- `antigen` is optional metadata only

### Output Matrix

The exported matrix is square over the declared `primary_channel` set:
- rows and columns use the same channel names
- channel order follows the manifest order
- default export is the compensation matrix
- `--matrix-kind spillover` exports the pre-inversion spillover matrix

## Algorithm Notes

`cytiq-autospill` follows the published AutoSpill workflow and upstream R
reference closely for the narrow v1 scope implemented here:
- FSC/SSC auto-gating
- robust spillover estimation
- iterative matrix refinement
- square compensation / spillover export

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

# cytiq-compensate

Applies a spillover or compensation matrix to the events of one FCS file.
Backs the `cytiq compensate` command.

## Usage

```bash
cytiq compensate --input sample.fcs --output sample.comp.fcs
cytiq compensate --input sample.fcs --output sample.comp.fcs \
  --matrix autospill/spillover.csv --matrix-kind spillover
```

From Rust:

```rust
use cytiq_compensate::{CompensateConfig, CompensateRunner, MatrixKind};

let result = CompensateRunner::run(&CompensateConfig {
    input: "sample.fcs".into(),
    output: "sample.comp.fcs".into(),
    matrix: Some(("spillover.csv".into(), MatrixKind::Spillover)),
    overwrite: false,
})?;
println!("{} events on a raw detector margin", result.margin_event_count);
```

## Public API

| Item | Purpose |
| --- | --- |
| `CompensateRunner::run` / `CompensateConfig` / `CompensateResult` | The `cytiq compensate` command |
| `read_matrix_file`, `MatrixKind`, `CompensationMatrix` | Square autospill matrix files and the file's own `$SPILLOVER` |
| `masks_path` | `sample.comp.fcs` → `sample.comp.masks.parquet` |
| `CompensateError` | Error type for every failure mode |

## Output rules

- Matrix channels hold `linear · compensation`, where linear values have
  `$PnE`/`$PnG` applied (`cytiq_fcs::linear_values`, as `cytiq gate` does).
  Other non-time channels whose log scaling or gain changes their values are
  written linearised, since the output declares `$PnE=0,0`; such channels lose
  `$PnG` and get a `$PnR` from their new values. Unchanged channels keep every
  keyword.
- The output records `CYTIQ_COMPENSATION` (`{source, channels}`) and keeps the
  source matrix only as `CYTIQ_SOURCE_SPILL`. Already compensated input, and
  input whose matrix channels were transformed, are rejected.
- `<output>.masks.parquet` holds `margin_pass` per event: whether the raw
  values of the matrix channels stay inside the detector limits
  (`cytiq_fcs::raw_margin_pass`, `MarginBounds::detector_thresholds`). Unlike
  `PeacoQC::RemoveMargins`, observed minima and maxima are not removed. Compensated and linearised channels are dropped from
  `CYTIQ_MARGINS`.
- Data is written as f32, or f64 when an unchanged channel would lose
  precision in f32 (for example `Original_ID` above 2^24).
- Existing outputs are replaced only with `overwrite`.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for FCS I/O, `$SPILLOVER`, compensation and
  margin bounds
- [`cytiq-mask`](../cytiq-mask) for the margin mask
- `csv` for matrix files

## Tests

```bash
cargo test -p cytiq-compensate
cargo test -p cytiq-cli --test compensate_cli
```

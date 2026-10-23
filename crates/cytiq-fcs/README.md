# cytiq-fcs

Strict FCS 3.0 / 3.1 / 3.2 reader, a compact FCS 3.1 float32 writer, and
spillover / compensation helpers. It has no command of its own: every other
`cytiq` crate that touches FCS files goes through it.

Used by `cytiq-autospill`, `cytiq-autospectral`, `cytiq-clean`,
`cytiq-cytoscan`, `cytiq-flowsom`, `cytiq-gating`, `cytiq-meta`,
`cytiq-peacoqc`, `cytiq-transform` and `cytiq-cli`.

## What it does

Reading (`FcsReader`):

- parses the 58-byte HEADER and the primary TEXT segment, then the
  supplemental TEXT segment when `$BEGINSTEXT`/`$ENDSTEXT` point to one
- keeps TEXT keywords both as ordered `[key, value]` pairs per segment and as a
  lookup map; escaped delimiters and whitespace-padded layout keywords are
  handled
- resolves DATA from `$BEGINDATA`/`$ENDDATA` and checks it against the HEADER
  offsets; ANALYSIS is optional and returned as raw bytes
- validates that all segments lie inside the file and do not overlap
- decodes list-mode (`$MODE=L`) DATA of type `A`, `I`, `F` or `D`, with
  per-parameter widths and `$BYTEORD`, into one column per parameter
  (`ColumnData::UInt`, `F32` or `F64`)

The reader is strict by design: anything that does not match the spec is an
error rather than a best-effort guess (duplicate keywords, HEADER/TEXT offset
mismatch, non-UTF-8 TEXT, unsupported `$MODE`/`$BYTEORD`/widths, DATA length
mismatch).

Writing (`write_fcs3_1_f32*`): FCS 3.1, little-endian 32-bit float, one column
per parameter, optional `$PnS` labels and extra TEXT keywords. This is the
layout used for every FCS file `cytiq` generates; it is not a general writer.

Compensation: `Spillover` parses `$SPILLOVER` / `$SPILL` / `SPILLOVER` /
`SPILL` (case-insensitive), `invert_matrix` inverts it with Gauss-Jordan
elimination and partial pivoting, and `compensate` applies the inverse to event
columns.

## Usage

```rust
use cytiq_fcs::{
    FcsReader, FcsWriteOptions, FcsWriteParameter, Spillover, compensate,
    write_fcs3_1_f32_with_options,
};

let dataset = FcsReader::open("raw.fcs")?.read()?;
println!("{} {} events", dataset.version.as_str(), dataset.event_count());

let fsc = dataset.find_parameter("FSC-A").expect("FSC-A present"); // $PnN, then $PnS
let values: Vec<f64> = dataset.data.columns[fsc].to_f64_vec();

if let Some(spillover) = Spillover::from_dataset(&dataset)? {
    let inverse = spillover.compensation_matrix()?;
    // One Vec<f64> per channel, in spillover.channels() order.
    let columns: Vec<Vec<f64>> = load_channels(&dataset, spillover.channels());
    let compensated = compensate(&columns, &inverse);
}

write_fcs3_1_f32_with_options(
    "out.fcs",
    &[FcsWriteParameter::with_label("FSC-A", "Forward scatter")],
    &[vec![1.0_f32, 2.0, 3.0]],
    &FcsWriteOptions::new().with_text_keyword("CYTIQ_EXAMPLE", "1"),
)?;
```

## Public API

| Item | Purpose |
| --- | --- |
| `FcsReader::open` / `FcsReader::new` / `FcsReader::read` | Read a file or any `Read + Seek` into `FcsDataset` |
| `FcsDataset` | `version`, `header`, `text`, `parameters`, `data`, `analysis`; `event_count`, `find_parameter` |
| `Text` (`FcsText`), `TextSegment` | TEXT keywords: `get` lookup plus ordered pairs of primary and supplemental segments |
| `Parameter`, `ParameterDataType`, `ParameterWidth` | Per-parameter metadata including all `$Pn*` keywords |
| `EventColumns`, `ColumnData` | Decoded event columns; `ColumnData::to_f64_vec` |
| `FcsVersion` | `V3_0` / `V3_1` / `V3_2`, `as_str()` gives `FCS3.x` |
| `find_parameter_index` | Match `$PnN`, then `$PnS`, then `"$PnN [$PnS]"` |
| `write_fcs3_1_f32`, `..._with_options`, `..._to_writer`, `..._to_writer_with_options` | FCS 3.1 float32 writer |
| `FcsWriteParameter`, `FcsWriteOptions` | Writer parameters and extra TEXT keywords |
| `Spillover`, `invert_matrix`, `compensate` | Spillover parsing and compensation |
| `Error` (`FcsError`) | Error type for every failure mode |

## Writer rules

- At least one parameter and one event; all columns the same length.
- Every value must be finite; `NaN`/`inf` is an error.
- Parameter names must be non-empty.
- Extra TEXT keywords must not override generated keywords (`$P*`, `$PAR`,
  `$TOT`, `$MODE`, `$DATATYPE`, `$BYTEORD`, segment offsets, `$NEXTDATA`).
- Offsets must fit the fixed 8-digit HEADER fields (`99_999_999` bytes).

## Dependencies

`log` and `thiserror` only.

## Tests

```bash
cargo test -p cytiq-fcs
```

`tests/reader.rs` reads the fixtures in `data/fixtures/fcs/` and synthetic
files built in the test, and round-trips the writer. Spillover and inversion
unit tests live in `src/compensation.rs`.

# Contributing to cytiq-fcs

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — `FcsReader`, segment resolution and validation, re-exports.
- `src/header.rs` — fixed 58-byte HEADER parsing and version detection.
- `src/text.rs` — TEXT segment tokenizing (delimiter escaping), lookup
  construction, `$BEGIN*`/`$END*` range parsing.
- `src/layout.rs` — resolves `$MODE`, `$TOT`, `$PAR`, `$DATATYPE`, `$BYTEORD`
  and per-parameter `$Pn*` keywords into a `ResolvedLayout`.
- `src/data.rs` — DATA decoding into typed columns; raw ANALYSIS bytes.
- `src/types.rs` — public data types (`FcsDataset`, `FcsText`, `Parameter`,
  `ColumnData`, ...) and `find_parameter_index`.
- `src/writer.rs` — FCS 3.1 little-endian float32 / float64 writer and the
  exact writer that picks between them.
- `src/acquisition.rs` — `CYTIQ_ACQUISITION` and `acquisition_id`.
- `src/lineage.rs` — `Original_ID` and `source_event_ids`.
- `src/compensation.rs` — `Spillover`, `invert_matrix`, `compensate` and their
  unit tests.
- `src/limits.rs` — bounds on sizes the file declares (`TEXT_BYTES_MAX`,
  `PARAMETERS_MAX`), each with the reason for its value.
- `src/error.rs` — `FcsError`, exported as `cytiq_fcs::Error`.

## Invariants

- Strict over lenient. Reject spec violations with a specific `FcsError`
  variant instead of guessing. If a real-world file needs leniency, make it
  explicit and narrow, and add a test with that file's shape.
- Duplicate TEXT keywords are an error, across the primary and supplemental
  segments together.
- DATA offsets come from TEXT; when the HEADER also has them, they must match.
  Segments must be in bounds and must not overlap.
- No value read from the file sizes an allocation or a loop before it is
  bounded. Check a segment against the file length before reading it, a count
  against `limits.rs` before allocating for it, and DATA length against the
  layout before reading DATA. Use checked arithmetic on declared sizes, and
  keep per-keyword work linear in the TEXT size (`$Pn*` keywords are grouped in
  one pass).
- `TextSegment::pairs` preserve file order and raw values. `cytiq-meta` exports
  them verbatim, so do not normalize or reorder them.
- `Parameter::keywords` hold only that parameter's `$Pn*` keywords
  (`parameter_keywords_do_not_leak_across_indexes`).
- Integer columns decode to `u64`, `F` to `f32` and `D` to `f64`. Do not widen
  or convert at read time; consumers call `to_f64_vec`.
- The writer emits one fixed layout (FCS 3.1, `F` 32-bit or `D` 64-bit,
  `$BYTEORD=1,2,3,4`). Every derived FCS file in the workspace depends on it,
  so changing it changes every command's output. It must keep rejecting
  non-finite values, overrides of generated keywords, and TEXT keys or values
  that are empty or start with `|` (they cannot be escaped).
- Event identifiers must survive every stage exactly: stages that write FCS go
  through `write_fcs3_1_exact_with_options` and record `CYTIQ_ACQUISITION`.
- Spillover matrices are row-major, rows = fluorochromes, columns = detectors.
  `compensate` panics on shape mismatch; callers check shapes first.
- No `unsafe`, and no dependencies beyond `log`, `thiserror`, `serde` /
  `serde_json` and `sha2`. Dev-dependencies (`proptest`) are for tests only.

## Where changes belong

- Workflow logic (gating, transforms, QC) belongs in the consumer crates, not
  here. This crate stays a format library.
- Metadata export to JSON belongs in `cytiq-meta`.
- Public API changes affect every crate in the workspace: run the full
  `just verify`, not only this crate's tests.

## Testing

```bash
cargo test -p cytiq-fcs
just verify
```

- `tests/reader.rs` — fixture reads (`data/fixtures/fcs/*.fcs`), synthetic files
  built with `build_fcs`, error cases and writer round-trips.
- `tests/limits.rs` — crafted files whose keywords lie about sizes; each must
  return a specific error, not panic, abort or hang.
- `tests/characterization.rs` — digests of every fixture read
  (`data/fixtures/characterization/fcs.json`); the ignored
  `writes_real_data_digests` writes the same digests for `$CYTIQ_REAL_DATA`
  to `$CYTIQ_DIGESTS_OUT`, to compare a reader change on local data.
- `tests/common/mod.rs` — `build_fcs` and `build_fcs_overriding` builders.
- `tests/properties.rs` — proptest properties: mutated and arbitrary bytes
  never panic and accepted files are consistent; finite datasets round-trip
  through the writer; spillover parsing accepts only square finite matrices.
  Failing seeds are kept in `properties.proptest-regressions`; commit them.
  Raise `PROPTEST_CASES` for a longer run.
- `fuzz/` — cargo-fuzz targets `read_fcs` and `spillover_keyword`, outside the
  workspace. Run `just fuzz-fcs [target] [seconds]` (nightly + cargo-fuzz).
  Turn every crash into a test in `tests/limits.rs`.
- `src/compensation.rs` `#[cfg(test)]` — spillover parsing, inversion, and
  compensation.

Add a failing case for every new rejection path, and keep fixtures tiny and
deterministic (see `data/fixtures/fcs/README.md`).

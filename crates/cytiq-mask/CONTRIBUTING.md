# Contributing to cytiq-mask

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs (the schema) and re-exports only.
- `src/mask.rs` — `EventMask`: Parquet I/O, `combine`, `keep_for`; unit tests.
- `src/runner.rs` — `MaskCombineRunner`, the `cytiq mask combine` command.
- `src/error.rs` — `MaskError` (`thiserror`).

## Invariants

- The Parquet schema is an output contract of every stage that writes masks
  (`compensate`, `peacoqc`, `clean`, `gate --membership`). Changing a column
  name, type or meaning needs an update of this crate's
  [schema documentation](README.md#schema) and of each writer's docs.
- Null in a filter column means "not assessed", never "passed". Do not fill
  nulls when reading or combining.
- Rows are matched by `source_event_id` only, never by position, so masks
  from files with different row sets (raw vs PeacoQC output) combine safely.
- Writers in other crates build an `EventMask` and call `write_parquet`; keep
  Arrow/Parquet code in this crate.

## Where changes belong

- Event lineage (`Original_ID`) belongs in `cytiq-fcs::lineage`.
- What a stage puts in its filter columns belongs in that stage's crate.
- CLI flags belong in `crates/cytiq-cli/src/main.rs`. Keep this crate free of `clap`.

## Testing

```bash
cargo test -p cytiq-mask
cargo test -p cytiq-cli --test mask_cli
```

`tests/properties.rs` checks generated masks: Parquet round-trips exactly, and
`combine` matches rows by `source_event_id` whatever their order.

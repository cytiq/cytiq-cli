# Contributing to cytiq-stats

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs`: crate docs and re-exports only.
- `src/runner.rs`: `StatsRunner`, `read_stats`, the serializable report types,
  channel kinds, histogram scales and atomic output.
- `src/summary.rs`: pure statistics over one channel (summary, percentiles,
  binning, clock resets), unit-tested.
- `src/error.rs`: `StatsError` (`thiserror`).

## Invariants

- The JSON report is a public contract (cytiq-api stores it as QC metrics).
  Any change to field names, types or meaning bumps `SCHEMA_VERSION` in
  `src/runner.rs` and updates the [output shape](README.md#output).
- Record measurements, never decisions: no thresholds, no pass/fail.
- The detector limit is `cytiq_fcs`'s margin bound, so `at_detector_limit`
  agrees with the `margin_pass` mask of `cytiq compensate` on the same file.
- Scatter is `cytiq_fcs::is_scatter_channel`; `Time` is matched by `$PnN`,
  ignoring case, as `cytiq_fcs::linear_values` does.
- Output is all-or-nothing through `cytiq_publish::write_file`; check
  `overwrite` before reading the input.

## Where changes belong

- FCS parsing and margin bounds belong in `cytiq-fcs`.
- CLI flags belong in `crates/cytiq-cli/src/commands/stats.rs`.

## Testing

```bash
cargo test -p cytiq-stats
cargo test -p cytiq-cli --test stats_cli
```

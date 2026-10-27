# Contributing to cytiq-peacoqc

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs, upstream references and re-exports only.
- `src/runner.rs` — the whole pipeline: channel resolution, margins, binning,
  peak detection, Isolation Tree, MAD outliers, consecutive-bin cleanup, event
  mask, cleaned FCS output and summary; `DEFAULT_*` constants. `compute_mask`
  holds the algorithm for both `PeacoQcRunner::run` and the in-memory
  `peacoqc_mask`.
- `src/export.rs` — `<output>.masks.parquet` (through `cytiq-mask`) and debug
  artifacts: `summary.json`, `mask.csv`, `report.html`.
- `src/error.rs` — `PeacoQcError` (`thiserror`).

## Invariants

- Masks must stay identical to R PeacoQC 1.20.0 on the parity dataset in every
  mode (`parity/REPORT.md`, section `peacoqc`). Ports are line-for-line on
  purpose: `smooth.spline` (`cytiq_density::smooth_spline_fitted`) follows
  `sbart.c`/`sgram.f`/`stxwx.f`/`sslvrg.f` (including `.nknots.smspl` and the
  `0.3330` constant) and matches R to 1e-6; the KDE in `cytiq-density` matches
  `stats::density` to 1e-9. Do not replace them with "equivalent"
  approximations.
- `peacoqc_mask` and the file pipeline share `compute_mask`; a change there
  moves both `cytiq peacoqc` and `cytiq spillqc`.
- Algorithm constants are fixed to upstream defaults and are not CLI flags. If
  one must change, change the `DEFAULT_*` constant, check parity, and update
  this crate's [README](README.md#algorithm).
- The Isolation Tree is deterministic (best split, no randomness); keep it so.
- Events are processed in file order; the `Time` channel name is recorded in
  the summary but events are not re-sorted.
- `Original_ID` in the output is the 1-based row of this run's input (kept as
  is when the input already has one). Downstream stages join through
  `source_event_id` (`Original_ID - 1`), which `cytiq-mask` and FlowSOM
  output carry.
- Output FCS is written only after the mask is complete, and non-finite output
  values are an error rather than being written.
- `summary.json`, `mask.csv` and `masks.parquet` are output contracts; update
  this crate's [output documentation](README.md#output-rules) when their fields change.

## Where changes belong

- KDE / `stats::density` and `smooth.spline` live in `cytiq-density` (shared
  with `cytiq-clean` and `cytiq-spillqc`).
- FCS reading and the `write_fcs3_1_f32` writer belong in `cytiq-fcs`.
- CLI flags belong in `crates/cytiq-cli/src/main.rs`. Keep this crate free of `clap`.

## Testing

```bash
cargo test -p cytiq-peacoqc
cargo test -p cytiq-cli --test peacoqc_cli
```

Unit tests sit in `runner.rs` (binning, consecutive bins, IT split, peaks,
MAD); the `smooth.spline` reference test moved to `cytiq-density` with the
port. `peacoqc_cli.rs` builds an FCS from
`data/fixtures/peacoqc/sample_events.csv`. For algorithm changes, rerun the
full parity harness (`parity/run_all.sh`) and compare against R.

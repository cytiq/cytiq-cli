# Contributing to cytiq-compensate

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/matrix.rs` — `CompensationMatrix`, `MatrixKind`, `read_matrix_file`; unit tests.
- `src/runner.rs` — `CompensateRunner`: state checks, raw margin mask,
  compensated columns, output keywords and writing.
- `src/error.rs` — `CompensateError` (`thiserror`).

## Invariants

- Compensated values must equal what `cytiq gate` computes for an `FCS` (or
  matrix) compensation-ref: linearise with `cytiq_fcs::linear_values`, then
  `cytiq_fcs::compensate`. `compensate_cli.rs` checks that gating the output
  with `uncompensated` dimensions reproduces gating the raw file with `FCS`
  ones.
- Never compensate twice: the output must carry `CYTIQ_COMPENSATION` and must
  not carry `$SPILLOVER`/`$SPILL` (moved to `CYTIQ_SOURCE_SPILL`).
- Margins are a raw-detector notion. Compute them before compensating, and
  never leave raw bounds in `CYTIQ_MARGINS` for channels whose values changed.
- Matrices are square over one channel set. Rectangular (dye × detector)
  matrices are unmixing and belong in `cytiq-autospectral`.

## Where changes belong

- Spillover parsing, matrix inversion, linearisation and margin bounds live in
  `cytiq-fcs`.
- The mask format lives in `cytiq-mask`.
- CLI flags belong in `crates/cytiq-engine/src/commands/compensate.rs`. Keep this crate free of `clap`.

## Testing

```bash
cargo test -p cytiq-compensate
cargo test -p cytiq-engine --test compensate_cli
```

# Contributing to cytiq-spillqc

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs, re-exports and `UPSTREAM` only.
- `src/compensaid.rs` — the port of `CompensAID()`: `.DetermineCenter`,
  `.DensityGating`, `.GetPopulations`, `.WithinLimit`, `.GetSegment`,
  `.MergeSegments`/`.AdjustSegments`/`.CleanSegments`, `.UpdateSSI`, and the
  flag thresholds. Comments name the upstream function each step mirrors.
- `src/preprocess.rs` — the vignette chain on a raw `FcsDataset` (margins,
  compensation, `estimateLogicle`, PeacoQC, `markernames`), `--preprocessed`
  and the matrix id.
- `src/batch.rs` — grouping by matrix id and recurrence statuses.
- `src/runner.rs` — config, input resolution (files, directories, globs),
  per-file assessment, skipping, `summary.json` types.
- `src/export.rs` — CSV and JSON outputs and their file names.
- `src/report.rs` — the Plotly `report.html`.
- `src/error.rs` — `SpillQcError` (`thiserror`).

## Invariants

- **CompensAID 0.99.7 (`Olsman/CompensAID@db74803`) is the reference.**
  - `compensaid.rs` is a line-for-line port. Keep its quirks:
    - `max()` over both columns of the population matrix (in
      `.DetermineCenter`'s count table and in the segment range);
    - segments that start at `min(positive)` but are sized from the cut-off;
    - neighbouring segments that share their boundary event;
    - the primary `.WithinLimit` check reading the populations from before the
      secondary adjustment;
    - the all-zero event appended before everything;
    - R's `round` (half to even) for shares, and 2 decimals for the SSI.
  - Changing any of these is an algorithm change. Re-run `just parity` and
    `just parity-real`, and update `parity/REPORT.md` §2.11.
- **Preprocessing must equal the vignette chain on the spillover channels.**
  - Values stay in double precision between steps, as flowCore's
    `compensate`/`transform` do.
  - `t` is flowCore's `maxRange` (`MarginBounds::from_flowcore_parameter`).
  - Margins use `PeacoQC::RemoveMargins`' rule.
- **Constants are upstream defaults:** `DEFAULT_SEGMENTS = 4`,
  `DEFAULT_MIN_EVENTS = 50`, `|SSI| ≥ 1`, `.WithinLimit`'s 10/90%, `deGate`'s
  `tinypeak.removal` of 1/25 and 1e-4. Segments and minimum events are flags
  because CompensAID exposes them; the others are not.
- **Pair order is CompensAID's** `expand.grid` order: secondary outer, primary
  inner. Results are deterministic under `--threads`: pairs and files run in
  parallel, and results are collected in order.
- **Output files are contracts.** Their names are constants in `src/export.rs`.
  Keep `summary.json`, the CSV columns and the README tables in sync.

### Deliberate divergences from R

All of these are inputs where R errors or returns `NA` rows:

| Situation | R | cytiq |
| --- | --- | --- |
| A cut-off is `NA` | Logical `NA` subsetting yields rows of `NA` that count as events | Comparisons with NaN select no events, so the pair is `not_assessed` |
| No secondary-negative events | `.WithinLimit` divides by zero and `if (NA)` errors | The primary adjustment is skipped |
| `deGate` returns `NA` inside `.WithinLimit` | `.GetClosestLimit` errors on `numeric(0)` | The old cut-off is kept |
| An old cut-off is `NA` in `.GetClosestLimit` | `if (NA)` errors | The new cut-off is taken |

One more difference comes from `flowDensity::deGate` (see
`cytiq-density/src/degate.rs`). Across an empty gap the smoothed density is
exactly zero, and R's pick among the zero grid points follows 1e-21-scale
round-off that no port reproduces; cytiq takes the first zero. Real samples
rarely have empty gaps, and the parity datasets have none. The synthetic
generator adds intermediate events on purpose.

## Where changes belong

- `deGate`, `density` and `smooth.spline` live in `cytiq-density`.
- `estimate_logicle_width` and `Logicle` live in `cytiq-transform::scales`.
- flowCore range keywords live in `cytiq-fcs` (`MarginBounds`).
- The PeacoQC mask lives in `cytiq-peacoqc` (`peacoqc_mask`).
- CLI flags belong in `crates/cytiq-engine/src/commands/spillqc.rs` (`SpillqcArgs`). Keep this
  crate free of `clap`.
- A future refinement mode should consume `ssi_segments.csv` and the
  `matrix_id`. It should write a candidate matrix file and never rewrite
  `$SPILL` in the input.

## Testing

```bash
cargo test -p cytiq-spillqc
cargo test -p cytiq-engine --test spillqc_cli
cargo test -p cytiq-engine --test parity_smoke spillqc
```

- **Unit tests** cover the center histogram, `.MergeSegments`, the
  closest-cut-off rules, SSI rounding and status, batch statuses, and a
  two-marker over-compensation case.
- **`spillqc_cli.rs`** runs the smoke tubes:
  - the injected errors are flagged;
  - the true matrix clears them;
  - the batch view calls them recurrent;
  - files without `$SPILL` are skipped;
  - already compensated input is refused;
  - `--preprocessed`, `--fail-on-flag`, overwrite and invalid flags behave as
    documented.
- **`parity_smoke.rs`** compares cytiq with CompensAID's tables in
  `data/fixtures/parity/spillqc/upstream`.
- **For algorithm changes**, run `just parity` and `just parity-real`. The
  datasets are `spillqc` and `compensaid_68983`.

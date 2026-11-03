# Contributing to cytiq-density

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/kde.rs` — `density` (port of `stats::density.default`), `BinDist`, FFT
  convolution, `bandwidth_nrd0`, `quantile_type7`.
- `src/scatter_gate.rs` — AutoSpill FSC/SSC `gate_events`, the 2-D KDE grid and
  peak search, `convex_hull`, `point_in_polygon`, `median`, `mad`.
- `src/valley.rs` — `find_valleys` and its default thresholds.
- `src/sampling.rs` — `sample_indices`, the deterministic event subsample.
- `src/smooth_spline.rs` — `smooth_spline_fitted` / `smooth_spline_fitted_xy`,
  port of R `stats::smooth.spline(spar = ...)` (`sbart.c`, `sgram.f`,
  `stxwx.f`, `sslvrg.f`, `.nknots.smspl`, the `0.3330` constant).
- `src/degate.rs` — `degate_upper_cuts`, port of
  `flowDensity::deGate(all.cuts = TRUE, upper = TRUE)` (`.densityForPlot`,
  `.getPeaks`, `.getIntersect`, `.trackSlope`).

## Invariants

- This crate is a numeric port of R code. `density`, `bandwidth_nrd0`,
  `quantile_type7`, `mad`, `smooth_spline_fitted*` and `degate_upper_cuts` must
  keep matching R to the tolerances in the unit tests. Do not "simplify"
  steps (binning, the `2n` grid, `kords`, `approx`, the `bw.nrd0` fallbacks)
  without re-checking against R.
- The gate constants at the top of `src/scatter_gate.rs` (`TRIM_*`, `*_BW_FACTOR`,
  `*_GRID`, `*_NEIGHBORHOOD`, `LOWER_LEFT_EXCLUSION`, `REGION_MAD_FACTOR`,
  `DENSITY_THRESHOLD`) mirror the upstream `autospill` defaults. Changing any of
  them is an algorithm-compatibility change, not a refactor: re-run the
  AutoSpill parity check (`parity/run_all.sh`) and update
  `parity/REPORT.md`.
- Everything is deterministic: no randomness, no threads, no I/O. Keep it that
  way so consumers stay reproducible.
- Ties and ordering follow `f64::total_cmp`; keep sorting stable and explicit.
- `smooth_spline_fitted` keeps its own `xbar = i / (n - 1)` arithmetic:
  PeacoQC's masks depend on it bit for bit.
- `degate_upper_cuts` ranks valley points by the density itself. R ranks them
  by `splinefun` at the same points, which agrees except across an exact-zero
  gap, where R follows round-off (see the module docs). Keep the first-zero
  rule rather than imitating the round-off.
- Shared by the workflow crates (`cytiq-autospill`, `cytiq-autospectral`,
  `cytiq-peacoqc`, `cytiq-clean`, `cytiq-cytonorm`, `cytiq-transform`,
  `cytiq-spillqc`, `cytiq-plot`). A behaviour change here changes all of
  their outputs; run the whole workspace test suite, not only this crate's.
- Keep the crate dependency-free.

## Where changes belong

- Put a primitive here only when more than one workflow crate needs it.
  Workflow-specific logic (channel selection, thresholds of a single command,
  reporting) stays in that workflow crate.
- Display scales (logicle, hyperlog, biexponential) live in
  `cytiq-transform::scales`, not here.

## Testing

```bash
cargo test -p cytiq-density
just test
```

R reference values for `density` and `smooth.spline` come from
`data/fixtures/peacoqc/{density,smooth_spline}_reference.rs`, and for `deGate`
from `data/fixtures/density/degate_reference.rs`, all written by
`parity/make_fixtures.R`. When adding a new R port, generate reference values
in R, commit them under `data/fixtures/`, and note the R version and the R
snippet in a comment next to the test, as `src/kde.rs` does.

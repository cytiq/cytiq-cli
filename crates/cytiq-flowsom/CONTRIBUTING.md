# Contributing to cytiq-flowsom

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs (paper and upstream links) and re-exports only.
- `src/runner.rs` — constants, config and result types, FCS loading, channel
  resolution, scaling, the deterministic LCG, SOM training, event assignment,
  average-linkage metaclustering, summary; unit tests.
- `src/export.rs` — assignment CSV, debug `summary.json` / `nodes.csv` /
  `metaclusters.csv`, Plotly `report.html`.
- `src/error.rs` — `FlowSomError` (`thiserror`).

## Invariants

- Same input, same output. Training uses the in-crate `DeterministicRng`
  (64-bit LCG, top 53 bits for indices) seeded with `FLOWSOM_RANDOM_SEED`.
  Do not swap it for `rand` or add thread-dependent randomness; the CLI test
  `cli_runs_flowsom_with_compact_channels_deterministically` relies on it.
- SOM training is sequential by design. A per-event `rayon` dispatch inside the
  training loop once made it ~60× slower (`parity/REPORT.md` §2.5). Use
  `rayon` only for embarrassingly parallel steps (matrix assembly, assignment,
  output rows).
- The training schedule mirrors `FlowSOM::SOM` / `som.c`: bubble neighbourhood,
  Chebyshev grid distance, per-step linear radius and learning-rate decay,
  BMU-only updates below radius 1, sampling with replacement, distinct-event
  initialisation. Changes need a parity run (`parity/run_all.sh`) and an
  updated report.
- The `scale = TRUE`, 20-metacluster and average-linkage choices are documented
  MVP decisions. Changing them changes every user's clusters: update the root
  README and the report.
- IDs: `event_index` is 0-based; node, metacluster and grid coordinates are
  1-based. Downstream mapping (`Original_ID`) depends on this.
- All algorithm settings stay `FLOWSOM_*` constants, not CLI flags, unless a
  deliberate API change is made.

## Where changes belong

- FCS parsing belongs in `cytiq-fcs`. `runner.rs` carries its own
  `find_parameter_index`; prefer `cytiq_fcs::find_parameter_index`, which has
  the same matching order, when touching that code.
- Transformations belong in `cytiq-transform`; this crate clusters values as
  given.
- CLI flags belong in `crates/cytiq-cli/src/main.rs`.

## Testing

```bash
cargo test -p cytiq-flowsom
cargo test -p cytiq-cli --test flowsom_cli
```

- `src/runner.rs` `#[cfg(test)]` — channel-list parsing, deterministic
  training, radius quantile, Chebyshev distance, distinct-event
  initialisation, metacluster count.
- `crates/cytiq-cli/tests/flowsom_cli.rs` — end-to-end runs with debug
  artifacts, determinism, missing input / channel / output, zero threads.

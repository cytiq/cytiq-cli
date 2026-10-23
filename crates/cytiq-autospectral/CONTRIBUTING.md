# Contributing to cytiq-autospectral

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/manifest.rs` — CSV row types, readers and validation; also
  `is_autofluorescence_dye` and `normalize_identifier` used by the runner.
- `src/runner.rs` — `AutoSpectralRunner`, algorithm constants, detector and
  primary-channel inference, scatter gate, positive selection, spectra,
  condition number, OLS unmixing, thread pool and `parallel_try_map`.
- `src/export.rs` — `write_outputs`: reference CSV/JSON, unmixed FCS,
  `diagnostics/run.json`, debug summary.
- `src/error.rs` — `AutoSpectralError` (`thiserror`).
- `tests/autospectral.rs`, `tests/support.rs` — runner tests on the fixture set.

## Invariants

- Parity with upstream AutoSpectral OLS is the reference. The constants in
  `runner.rs` mirror `asp$*` parameters (`POSITIVITY_THRESHOLD`,
  `POSITIVE_EVENT_LIMIT`, `MIN_POSITIVE_EVENTS`); changing them, the
  normalisation (`PeakNorm::Signed` for fluorophores, `PeakNorm::Absolute` for
  AF) or the quantile definition (R type 7) changes results. Re-run parity and
  update `parity/REPORT.md` when you do.
- Spectra are columns of the reference matrix, in manifest order of single
  stains with the autofluorescence component last. Component order defines the
  FCS column order and the JSON/CSV output.
- The condition-number warning uses fluorophores only; the hard limit
  (`CONDITION_NUMBER_LIMIT`) uses the full matrix including AF.
- Parallel stages go through `parallel_try_map`, which keeps results and errors
  in input order. Keep outputs deterministic regardless of `threads`.
- `threads: Some(n)` runs in a local rayon pool; `None` uses the global pool.
- Unmixed FCS files contain only the generated parameters. Do not try to copy
  source TEXT keywords here; `cytiq-meta` is the place for that.
- Diagnostics use `log`; never print to stdout.

## Where changes belong

- FCS reading/writing and keyword handling → `cytiq-fcs`.
- Flags and `--help` text → `crates/cytiq-cli/src/main.rs`.
- A shared KDE scatter gate (the parity report suggests reusing the
  `cytiq-autospill` gate) → `cytiq-density`, not a copy in this crate.
- Fixture changes → `data/fixtures/autospectral/` and its README.

## Testing

```bash
cargo test -p cytiq-autospectral
cargo test -p cytiq-cli --test autospectral_cli
```

Unit tests live in `src/manifest.rs` (manifest validation, column aliases) and
`src/runner.rs` (reference matrix layout, OLS recovery, detector inference,
`parallel_try_map` ordering). `tests/support.rs` is also compiled into
`crates/cytiq-cli/tests/autospectral_cli.rs` via `#[path]`; keep its helpers
generic. Parity scripts live in `parity/` (`run_all.sh`, `compare.py`).

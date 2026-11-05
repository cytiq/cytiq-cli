# Contributing to cytiq-engine

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/main.rs` — `main` and the `run()` dispatcher: parse the command line,
  start the provenance recorder, hand each `Commands` variant to its module.
- `src/cli.rs` — `Cli`, `Commands` (one newtype variant per command, holding
  its `*Args`), `command_name`, and the `Cli*` value enums with `From` impls
  into library types.
- `src/commands/<name>.rs` — one module per command: its `#[derive(Args)]`
  struct and `run(args, recorder)`, plus its subcommand enum when it has one
  (`MaskCommand`, `FlowSomCommand`, `CytoNormCommand`).
- `src/parsers.rs` — shared argument parsers (`parse_threads`,
  `parse_positive_f64`, `parse_finite_f64`, `parse_non_negative_f64`, and
  `parse_channels` for every `--channels` + `--channel` pair, built on
  `cytiq_fcs::parse_channel_list`).
- `src/provenance.rs` — `build_info` and `record_provenance`.
- `src/logging.rs` — the stderr logger; the binary's own modules all log as
  `cytiq`.
- `tests/<command>_cli.rs` — integration tests per subcommand, running the
  `cytiq` binary.

The package is `cytiq-engine`; the binary is `cytiq` (`[[bin]]` in `Cargo.toml`).

## Invariants

- `build.rs` sets `CYTIQ_GIT_COMMIT` (the `CYTIQ_BUILD_COMMIT` environment
  variable, else `.cargo_vcs_info.json` in a packaged crate, else git),
  `CYTIQ_RUSTC_VERSION`, `CYTIQ_TARGET` and `CYTIQ_PROFILE` for provenance
  records. A packaged crate never asks git: its unpacked source may sit inside
  an unrelated repository.

- Keep the CLI thin. A command arm only reads manifests, maps flags into the
  library config, calls `<Crate>Runner::run` and logs. No algorithm, file
  format or output-layout logic here.
- Library crates must not depend on `clap`. Mirror library enums with a local
  `Cli*` enum deriving `ValueEnum` plus `impl From<Cli*> for <LibraryType>`.
  Large commands use a separate `#[derive(Args)]` struct with
  `impl From<Args> for <Config>` (see `CleanArgs` → `CleanConfig`).
- Defaults that the library also needs come from library constants
  (`cytiq_clean::DEFAULT_*`), not duplicated literals.
- Validate values at parse time with `value_parser` (`parse_threads` rejects
  `0`; float parsers reject NaN/inf) rather than inside the runner.
- stdout is for command output only. Everything else goes through `log`
  (`info!` for progress and results, `debug!` for details); the logger writes
  to stderr.
- `Commands` is serialized untagged into provenance, so a command's
  `config` is its `*Args` fields; keep the variant a newtype around them.
  `*Args` structs carry no doc comment: the variant's doc comment is the
  command's `--help` about text.
- Errors propagate as `Box<dyn Error>` from `run()`; `main` prints the error
  and exits with code `1`. Channel-list errors are wrapped as
  `io::ErrorKind::InvalidInput`. Do not `panic!` or call `process::exit` in a
  command arm.
- `--overwrite` is passed through to the library, which enforces it
  (`transform`, `meta`, `compensate`, `mask combine`, `plot`). `meta` maps
  `--output -` to stdout before building `MetaConfig`.

## Adding a subcommand

1. Implement the workflow in a library crate under `crates/` with a
   `<Name>Config`, a `<Name>Runner::run` and a `thiserror` error type; add it
   to the workspace `members`.
2. Add the path dependency to this crate's `Cargo.toml`.
3. Add `src/commands/<name>.rs` with a `#[derive(Debug, Serialize, Args)]`
   `<Name>Args` (field doc comments become `--help` text), reusing the shared
   parsers and the `--threads` / `--debug-dir` / `--channels` + `--channel` /
   `--mask` conventions where they apply, and a `Commands::<Name>(<Name>Args)`
   variant with a doc comment in `src/cli.rs`.
4. Add `run(args, recorder)` in that module and its arm in `main.rs`'s
   dispatcher: `info!("starting <name>: ...")`, build the config, call
   `recorder.snapshot_inputs(role, files)` for each kind of input **before**
   the runner (so inputs are hashed before anything can change them), set
   `set_algorithms(<crate>::UPSTREAM)` and `set_constants` when the crate has
   them, run, log a one-line result summary, and end with `record_provenance`
   listing the output files. When the library resolves inputs itself (globs,
   directories, manifests), expose a `<Name>Runner::input_files`. Add the
   command to `command_name`. The `<Name>Args` fields become the record's
   `config` (`Commands` derives `Serialize`), so new `Cli*` value enums
   derive it too.
5. Add `tests/<name>_cli.rs` covering success, a missing input and at least
   one invalid flag.
6. Document the command in its library crate's `README.md` and in the command
   table of this crate's README. Update the pipeline diagram in the root
   `README.md` and `PIPELINE.md` when stage order or signal-state checks change.

## Where changes belong

- New behaviour, validation of file contents, output formats → the backing
  library crate.
- New flags, flag defaults, `--help` wording, log messages →
  `src/commands/<name>.rs` (a command's about text → `src/cli.rs`).
- Shared fixtures → `data/fixtures/<command>/`.

## Testing

```bash
cargo test -p cytiq-engine
cargo test -p cytiq-engine --test <command>_cli
```

Test files: `autospill_cli.rs`, `autospectral_cli.rs`, `clean_cli.rs`,
`compensate_cli.rs`, `cytoscan_cli.rs`, `flowsom_cli.rs`, `gate_cli.rs`,
`mask_cli.rs`, `meta_cli.rs`, `parity_smoke.rs` (autospill, AutoSpectral,
PeacoQC, CytoScan and CompensAID against the R outputs in
`data/fixtures/parity`), `peacoqc_cli.rs`, `pipeline_cli.rs` (the whole chain
on `data/fixtures/pipeline` against per-event truth), `plot_cli.rs` (every
plot type, masks, gates checked against `cytiq gate`, classes, batch output,
determinism), `preflight_cli.rs`,
`qc_cli.rs`, `spillqc_cli.rs`, `transform_cli.rs`. `tests/common/mod.rs` holds `assert_provenance`,
`assert_provenance_chain` and `adjusted_rand_index`.
`channels_cli.rs` pins the channel flag errors of every command.
`src/cli.rs` has one unit test, `cli_definition_is_consistent`
(`Cli::command().debug_assert()`); logic worth unit-testing usually belongs in
a library crate. `autospill_cli.rs` and
`autospectral_cli.rs` include `crates/<crate>/tests/support.rs` via
`#[path]`, so changes to those helpers affect both crates' tests.

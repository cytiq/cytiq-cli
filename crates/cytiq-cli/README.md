# cytiq-cli

The `cytiq` binary. A thin `clap` front end that parses arguments, builds the
config of a library crate, calls its runner and logs a summary. All algorithms
live in the library crates listed below.

## Commands

| Command | Backing crate | Purpose | Docs |
| --- | --- | --- | --- |
| `cytiq autospill` | [`cytiq-autospill`](../cytiq-autospill) | Spillover or compensation matrix from single-color controls | [AutoSpill](../../README.md#autospill) |
| `cytiq autospread` | [`cytiq-autospill`](../cytiq-autospill) | Spillover spreading matrix from single-color controls | [AutoSpill](../../README.md#autospill) |
| `cytiq autospectral` | [`cytiq-autospectral`](../cytiq-autospectral) | Reference-based spectral unmixing | [AutoSpectral](../../README.md#autospectral) |
| `cytiq transform` | [`cytiq-transform`](../cytiq-transform) | arcsinh / logicle / hyperlog / biexponential signal transforms | [Transform](../../README.md#transform) |
| `cytiq compensate` | [`cytiq-compensate`](../cytiq-compensate) | Apply a spillover or compensation matrix to sample events | [Compensate](../../README.md#compensate) |
| `cytiq preflight` | [`cytiq-preflight`](../cytiq-preflight) | Check a run manifest and its FCS files before computation | [Preflight](../../README.md#preflight) |
| `cytiq qc` | [`cytiq-qc`](../cytiq-qc) | Check a sample's final event mask against its panel's QC policy | [QC](../../README.md#qc) |
| `cytiq mask combine` | [`cytiq-mask`](../cytiq-mask) | Combine per-event masks of one acquisition | [Masks](../../README.md#masks) |
| `cytiq cytoscan` | [`cytiq-cytoscan`](../cytiq-cytoscan) | File-level anomaly detection over a cohort | [CytoScan](../../README.md#cytoscan) |
| `cytiq classify` | [`cytiq-classify`](../cytiq-classify) | Supervised panel-specific event classification | [Classify](../../README.md#classify) |
| `cytiq gate` | [`cytiq-gating`](../cytiq-gating) | Gating-ML 2.0 evaluation with per-gate counts and optional per-event membership | [Gating](../../README.md#gating) |
| `cytiq clean` | [`cytiq-clean`](../cytiq-clean) | Debris, doublet and dead-cell removal as per-event masks | [Clean](../../README.md#clean) |
| `cytiq peacoqc` | [`cytiq-peacoqc`](../cytiq-peacoqc) | Peak-based acquisition quality control | [PeacoQC](../../README.md#peacoqc) |
| `cytiq flowsom` | [`cytiq-flowsom`](../cytiq-flowsom) | FlowSOM-style clustering | [FlowSOM](../../README.md#flowsom) |
| `cytiq meta` | [`cytiq-meta`](../cytiq-meta) | FCS metadata export as JSON | [Meta](../../README.md#meta) |

`cytiq <command> --help` lists every flag with its default.

```bash
just run meta --input data/data.fcs
cargo run -p cytiq-cli -- transform --input raw.fcs --output out.fcs --method logicle
```

## Build and distribute

`cargo build --release --locked` produces `target/release/cytiq` and its CPU
ONNX Runtime in `target/release/lib/`. The runtime is downloaded and verified
by the build; no Python installation or runtime environment variables are
needed. Keep `cytiq` and `lib/` together when distributing the build. See the
root [classify documentation](../../README.md#classify) for supported targets.

## Shared flag conventions

- `--threads N` caps worker threads; omitted means all logical CPUs, `0` is
  rejected at parse time.
- `--debug-dir DIR` writes optional diagnostics where the command supports it.
- Channels are passed either as `--channels A,B,C` or as repeated
  `--channel A --channel B` (`peacoqc`, `flowsom`, `cytoscan`; at least one is
  required). `transform` takes `--channels` / `--exclude-channels` only.
- Numeric flags are validated at parse time (positive, non-negative or finite,
  depending on the flag).
- `--overwrite` exists on `classify`, `transform`, `meta`, `compensate` and
  `mask combine`; other commands follow the output rules of their library
  crate.
- `--mask FILE` (repeatable) restricts `classify`, `gate`, `flowsom` and `clean` to events
  every mask keeps; see [Masks](../../README.md#masks).

## Provenance

Each command arm finishes with `record_provenance`, which writes
`<output>.provenance.json` (or `<dir>/provenance.json`) with the arguments and
the SHA-256 of the command's input and output files; see
[`cytiq-provenance`](../cytiq-provenance). `meta` writes none.

## Logging and output streams

- Logs go to stderr as `[LEVEL] module::path: message`; stdout is reserved for
  command output (`meta` JSON, `transform --dry-run` listing).
- The level comes from `CYTIQ_LOG`, then `RUST_LOG` (`off`, `error`, `warn`,
  `info`, `debug`, `trace`; for `RUST_LOG`-style lists the last valid token
  wins, `target=` prefixes are ignored).
- Without either variable the default is `info` when stderr is a terminal and
  `warn` otherwise.

## Exit codes

- `0` on success.
- `1` when a command fails; the error is printed to stderr.
- Argument errors are reported by `clap` with its own exit code (`2`).

## Tests

Integration tests run the built `cytiq` binary (`CARGO_BIN_EXE_cytiq`), one
file per command under `tests/`, plus `pipeline_cli.rs`, which runs the whole
documented chain on the golden fixture `data/fixtures/pipeline` and checks
every stage against per-event truth, lineage, provenance and reproducibility:

```bash
cargo test -p cytiq-cli
cargo test -p cytiq-cli --test transform_cli
```

Some tests read fixtures from `data/fixtures/`; `autospill_cli.rs` and
`autospectral_cli.rs` share the `tests/support.rs` module of their library
crate via `#[path]`.

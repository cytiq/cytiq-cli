# cytiq-engine

The `cytiq` binary. A thin `clap` front end that parses arguments, builds the
config of a library crate, calls its runner and logs a summary. All algorithms
live in the library crates listed below.

See the [pipeline guide](PIPELINE.md) for stage order, a complete conventional
chain and signal-state checks.

## Commands

| Command | Backing crate and documentation | Purpose |
| --- | --- | --- |
| `cytiq autospill` | [`cytiq-autospill`](../cytiq-autospill/README.md) | Spillover or compensation matrix from single-color controls |
| `cytiq autospread` | [`cytiq-autospill`](../cytiq-autospill/README.md) | Spillover spreading matrix from single-color controls |
| `cytiq autospectral` | [`cytiq-autospectral`](../cytiq-autospectral/README.md) | Reference-based spectral unmixing |
| `cytiq transform` | [`cytiq-transform`](../cytiq-transform/README.md) | arcsinh / logicle / hyperlog / biexponential signal transforms |
| `cytiq compensate` | [`cytiq-compensate`](../cytiq-compensate/README.md) | Apply a spillover or compensation matrix to sample events |
| `cytiq spillqc` | [`cytiq-spillqc`](../cytiq-spillqc/README.md) | Flag likely spillover-matrix errors in stained samples (CompensAID SSI), per file and across a batch |
| `cytiq preflight` | [`cytiq-preflight`](../cytiq-preflight/README.md) | Check a run manifest and its FCS files before computation |
| `cytiq qc` | [`cytiq-qc`](../cytiq-qc/README.md) | Check a sample's final event mask against its panel's QC policy |
| `cytiq mask combine` | [`cytiq-mask`](../cytiq-mask/README.md) | Combine per-event masks of one acquisition |
| `cytiq cytoscan` | [`cytiq-cytoscan`](../cytiq-cytoscan/README.md) | File-level anomaly detection over a cohort |
| `cytiq classify` | [`cytiq-classify`](../cytiq-classify/README.md) | Supervised binary or vocabulary-aware multiclass event classification |
| `cytiq gate` | [`cytiq-gating`](../cytiq-gating/README.md) | Gating-ML 2.0 evaluation with per-gate counts and optional per-event membership |
| `cytiq population validate/observe` | [`cytiq-population`](../cytiq-population/README.md) | Validate native annotations and produce typed population observations |
| `cytiq specimen validate/plan/run` | [`cytiq-specimen`](../cytiq-specimen/README.md) | Freeze acquisition selection, reconcile panels and publish specimen results |
| `cytiq specimen compare` | [`cytiq-specimen`](../cytiq-specimen/README.md#comparing-native-results) | Compare native specimen summaries with exact counts and explicit tolerances |
| `cytiq specimen report` | [`cytiq-specimen`](../cytiq-specimen/README.md#reports-and-exports) | Export a verified result or comparison as offline HTML, CSV and JSON |
| `cytiq clean` | [`cytiq-clean`](../cytiq-clean/README.md) | Debris, doublet and dead-cell removal as per-event masks |
| `cytiq peacoqc` | [`cytiq-peacoqc`](../cytiq-peacoqc/README.md) | Peak-based acquisition quality control |
| `cytiq flowsom` | [`cytiq-flowsom`](../cytiq-flowsom/README.md) | FlowSOM-style clustering |
| `cytiq flowsom train/apply` | [`cytiq-flowsom`](../cytiq-flowsom/README.md) | Persist and reuse SOM models, with optional consensus metaclustering |
| `cytiq cytonorm train/apply` | [`cytiq-cytonorm`](../cytiq-cytonorm/README.md) | Panel-specific batch normalization, controls/aggregates and QC |
| `cytiq plot` | [`cytiq-plot`](../cytiq-plot/README.md) | 2-D density, scatter and contour plots and 1-D histograms as vector SVG (or Plotly JSON), with removed events, gates and classes overlaid |
| `cytiq meta` | [`cytiq-meta`](../cytiq-meta/README.md) | FCS metadata export as JSON |
| `cytiq stats` | [`cytiq-stats`](../cytiq-stats/README.md) | Per-channel statistics for technical QC: distribution, detector-limit and zero counts, histograms, Time channel |

`cytiq <command> --help` lists every flag with its default.

```bash
just run meta --input data/fixtures/gating/sample.fcs
cargo run -p cytiq-engine -- transform --input raw.fcs --output out.fcs --method logicle
```

## Build and distribute

`cargo build --release --locked` produces `target/release/cytiq` with ONNX
Runtime linked statically. The `ort` dependency downloads and caches its prebuilt
runtime during the build. The executable needs no Python, runtime environment
variables or companion `lib/` directory. See the
[classify documentation](../cytiq-classify/README.md) for supported targets.

Releases are also published to crates.io, from source:

```bash
cargo install --locked cytiq-engine \
  --config 'profile.release.overflow-checks=true' \
  --config 'profile.release.lto="fat"' \
  --config 'profile.release.codegen-units=1' \
  --config 'profile.release.panic="abort"'
```

`--locked` uses the dependency versions the release was tested with. A
published crate does not carry the workspace's release profile, so the
`--config` flags restore it: without them integer overflow wraps silently
instead of aborting the run, and the binary is built without fat LTO. The build
downloads ONNX Runtime like any other, so it needs network access and a
supported target. Provenance records the release commit that cargo stored in
the package.

## Shared flag conventions

- `--threads N` caps worker threads; omitted means all logical CPUs, `0` is
  rejected at parse time.
- `--debug-dir DIR` writes optional diagnostics where the command supports it.
- Channels are passed either as `--channels A,B,C` or as repeated
  `--channel A --channel B` (`peacoqc`, `flowsom`, `cytoscan`; at least one is
  required). `transform` takes `--channels` / `--exclude-channels` only.
- Numeric flags are validated at parse time (positive, non-negative or finite,
  depending on the flag).
- `--overwrite` exists on `classify`, `transform`, `meta`, `stats`, `compensate`,
  `mask combine` and `plot`; other commands follow the output rules of their library
  crate.
- `--mask FILE` (repeatable) restricts `classify`, `gate`, `flowsom` and `clean` to events
  every mask keeps; `plot` shows those events and draws the removed ones over them; see [cytiq-mask](../cytiq-mask/README.md).

## Provenance

Each command arm finishes with `record_provenance`, which writes
`<output>.provenance.json` (or `<dir>/provenance.json`) with the arguments and
the SHA-256 of the command's input and output files; see
[`cytiq-provenance`](../cytiq-provenance). `meta` writes one only for a file
`--output`; the JSON it prints on stdout carries the input's SHA-256 itself.

## Logging and output streams

- Logs go to stderr as `[LEVEL] module::path: message`; stdout is reserved for
  command output (`meta` JSON, `transform --dry-run` listing, the
  `population`/`specimen` outcome line). `--outcome <PATH>` writes the outcome
  of any command to a file instead (see [Exit codes](#exit-codes)).
- The level comes from `CYTIQ_LOG`, then `RUST_LOG` (`off`, `error`, `warn`,
  `info`, `debug`, `trace`; for `RUST_LOG`-style lists the last valid token
  wins, `target=` prefixes are ignored).
- Without either variable the default is `info` when stderr is a terminal and
  `warn` otherwise.

## Exit codes

Every command exits with one of five codes; the error, if any, is the last
line on stderr.

| Code | Reason | Meaning | Orchestrator action |
| --- | --- | --- | --- |
| 0 | `success` | Completed; warnings remain visible in reports and logs | Continue |
| 1 | `runtime_error` | Internal or environmental failure (permissions, full disk, thread pool, ONNX Runtime) | Retry by policy |
| 2 | `invalid_configuration` | Invalid flags or configuration, corrupt or unsupported data, a deterministic algorithm failure on that input | Fail; do not retry |
| 3 | `checks_failed` | Completed and wrote its report, but checks failed (`qc`, `preflight`, `spillqc --fail-on-flag`, `specimen` checks) | Send to review |
| 4 | `output_conflict` / `incomplete` | An output already exists (pass `--overwrite`), a required input is missing, or a specimen result is incomplete | Inspect artifacts and run state |

Only `1` is worth retrying. Argument syntax errors are reported by `clap`
with code `2` before any command runs.

`--outcome <PATH>` (accepted before or after the subcommand) also writes the
outcome as JSON: `schema_version`, `command`, `completed` (true for `0` and
`3`, and for published incomplete specimen packages), `reason`, `exit_code`,
`message` and `artifacts` (always `[]` except for `population`/`specimen`
packages). The file is replaced atomically; it is not written for argument
errors, and failing to write it is reported on stderr without changing the
exit code. `population` and `specimen` additionally print the same JSON as one
line on stdout (see [the specimen contract](../cytiq-specimen/README.md#packages-and-orchestration-contract)).

Before 0.5, every failure exited with `1`: `qc`, `preflight` and
`spillqc --fail-on-flag` now exit with `3`, invalid input with `2`, and a
missing input or existing output with `4`. Scripts that tested for `1` should
test for a non-zero code or the specific code they handle.

## Tests

Integration tests run the built `cytiq` binary (`CARGO_BIN_EXE_cytiq`), one
file per command under `tests/`, plus `pipeline_cli.rs`, which runs the whole
documented chain on the golden fixture `data/fixtures/pipeline` and checks
every stage against per-event truth, lineage, provenance and reproducibility:

```bash
cargo test -p cytiq-engine
cargo test -p cytiq-engine --test transform_cli
```

Some tests read fixtures from `data/fixtures/`; `autospill_cli.rs` and
`autospectral_cli.rs` share the `tests/support.rs` module of their library
crate via `#[path]`.

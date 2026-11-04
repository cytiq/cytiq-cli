# Algorithm performance

Paired end-to-end benchmarks for `compensate` and `transform`, separate from
the Cargo CI benchmark. The CLI child alone is measured with `wait4`: wall
time, user/system CPU, peak RSS in bytes, and events/second. Reading, input
hashing, publication and provenance are included; fixture generation,
compensation of shared transform inputs and output comparison are excluded.
These are warm-cache measurements, without a forced durable disk flush.

## Build and run

Build the baseline from a clean, current `main` **before** editing it, then
preserve the binary. Use the same Rust toolchain, Cargo.lock, build flags and
release profile for both versions. Do not compare debug and release builds.
The workspace release profile includes overflow checks; leave them enabled.

```bash
# On clean main, before applying the optimisation:
cargo build --release --locked -p cytiq-cli
mkdir -p target/performance/baseline
cp target/release/cytiq target/performance/baseline/cytiq

# After applying the optimisation:
cargo build --release --locked -p cytiq-cli
cargo build --release --locked -p cytiq-cli --example performance_support

just performance \
  --baseline-bin target/performance/baseline/cytiq \
  --candidate-bin target/release/cytiq
```

The `performance_support` example reuses cytiq's FCS writer, FCS metadata
reader, EventMask reader and provenance schema. It is a development tool,
not a shipped CLI subcommand. Python 3.9+ needs no external packages.
macOS is the acceptance platform; Linux also has explicit RSS unit handling.
On sandboxed macOS, system metadata may be unavailable; run outside the
sandbox for acceptance measurements and record complete CPU/RAM information.
If a local `sccache` wrapper cannot run, use `RUSTC_WRAPPER=` for **both** builds.

By default, datasets include 100,000 and 1,000,000 events × 30 float32 channels
(26 spillover channels plus Time and scatter), four 100,000-event inputs, and
the SHA-256-pinned FlowSOM 68983 and PeacoQC 111 real acquisitions. Install
them with `just parity-setup` or provide their R library with `--r-library`.
The deterministic SplitMix64 generator uses seed 42 (100–103 for multiple
inputs), includes negative values and a rare bright population. Its version
is 1; input hashes, seeds, case sizes and helper hash identify the workload.

Transform arcsinh/logicle runs at 1 and 4 threads on the same inputs prepared
by the baseline compensator. Multi-file cases use 4 threads. Compensation is
serial. Every child receives `RAYON_NUM_THREADS=4`; transform's explicit local
pool overrides that setting. Benchmarks run sequentially; avoid other CPU,
memory and disk workloads and keep power settings constant.

## Parity and acceptance

Each case uses two warmup pairs followed by 20 measured pairs, alternating
A/B and B/A order. Both versions see the same paths and arguments. Outputs
are moved out of the common output path after each run; the latest pair and
all process logs remain available. Comparison runs after **every** pair:

- FCS and metric sidecars must match byte for byte, including signed zero,
  stored precision, TEXT metadata and embedded numerical metrics.
- Masks must have identical acquisition, event/source IDs, filters and keep
  values, independent of Parquet encoding.
- Provenance must match except `started_at_unix_ms`, `finished_at_unix_ms`
  and `build.git_commit`. Tool versions, compiler, profile, target, config,
  arguments, scientific constants and all input/output hashes must agree.
- Missing, extra or empty outputs and any failed CLI process fail the run.

Acceptance applies to the five `large_*` cases: compensate plus two transforms
at both thread counts. Each must reduce median peak RSS to **≤90%** of the
baseline. The upper bound of the **95% percentile bootstrap interval** for
the median paired candidate/baseline wall-time ratio must be **≤1.05**.
Bootstrap uses 10,000 resamples and seed 42. The other cases are diagnostic.
Failed thresholds remain failures in the scorecard and cause exit status 1.
Do not rerun selectively until a noisy result happens to pass.

`--smoke --pairs 2 --warmups 0` checks the full runner on small synthetic
inputs, without claiming acceptance. `--case large_compensate` selects a
case (repeat the flag for several); an incomplete or shortened series is
labelled diagnostic. `--synthetic-only` explicitly omits the real datasets;
their independent scientific gate still needs `just parity-real`.

Every run needs a fresh `--output` directory (default `performance/.out/<time>`).
It writes `results.json` with raw pairs, build/binary/dataset hashes, source
diff hash, host and environment, plus `scorecard.md`. An optional
`build.json` beside each binary is included verbatim for additional build
context. Outputs are ignored by Git: attach the report to the PR, not the
large generated files. Preserve the matching source diff when benchmarking
uncommitted changes; the commit alone does not identify that binary.

## Checks

`just performance-check` tests measurement isolation, units, bootstrap and
failure handling plus fixture/comparator behavior, without running a benchmark.
It runs in CI and in `just verify`. Run `just parity` and `just parity-real`
as the independent R/Python scientific gates; never widen their tolerances
or regenerate their golden values to make an optimisation pass.

# Simplification rollout

Source baseline: `7cee617ec0f9c01490fcdecaf4ba493e0430ceb2` (2026-11-07).
All 26 workspace packages remain. Crate mergers, FCS replacement and R/Python
backends are deferred. Changes are delivered together in the working tree; the
numbered stages follow the approved plan.

## Baseline and implementation

The tracked baseline contains 60,316 physical Rust lines under `src`, including
inline tests, and 19,871 lines under `tests`. These are physical lines, not SLOC.
A release binary and benchmark helper were built before functional edits, using
the pinned Rust 1.99.1 toolchain with `RUSTC_WRAPPER=`. Copies and raw measurements
are under ignored `target/simplification/`. Baseline CLI SHA-256:
`3b17b7fbc993e380e9b8f0b7a36c4d6d660d46fb98f5e77a866fe8dbc62d79e5`.

Shared external declarations preserve locked versions and per-consumer features.
CITATIONS now describes the independently implemented mclust-compatible search.
The optional FCS `inputs` module replaces the three resolver implementations,
retains global/group ordering and command-specific error types, and surfaces
directory-entry failures. Mutation selection moves with the implementation; the
FCS mutation run explicitly enables `inputs`, and the now-empty CytoScan-only
mutation target is retired.

The JSON Schema replacement was closed after its performance experiment; the
existing interpreter and normalization remain. Observation conversion uses
serde_arrow with a single explicit physical schema and typed remote Serde DTOs;
structured fields remain JSON text.
Clean and PeacoQC reports use local Askama templates and private Plotly HTML
wrappers. Public formats and error variants remain unchanged.

## Rejected pilots

These experiments ran against the original functions in an isolated Cargo
harness under `target/simplification/pilots`, with nalgebra 0.35.0, kodama 0.3.0
and contour 0.13.1. The separate population library experiment also evaluated
jsonschema 0.60.2 and serde_arrow 0.15.1. No rejected backend was added to
production. nalgebra remains an existing dependency of the scientific crates
that already used it.

- **JSON Schema:** runtime compilation increased startup cost. Compile-time
  jsonschema 0.60.2 validators passed all six artifact kinds, serialization vectors
  and fuzz regressions, but the 20-pair library experiment still failed admission:
  cold wall-time ratio 1.178 (upper 95% bound 1.309), peak RSS ratio 1.219.
  Repeated loads improved (wall ratio 0.723) but peak RSS remained 1.196.
  The original implementation is retained; no jsonschema dependency remains.

- **Matrix inversion:** for the row-major 4×4 matrix below, the original inverse
  returns success, while LU plus the same `EPSILON × max(1, scale) × size` guard
  rejects it. The approved refactor preserves existing acceptance boundaries;
  changing this boundary needs a separate numerical change.

  ```text
   3  0  4  3
  -2 -2 -4 -2
  -1 -3 -4  4
  -3  4 -2  2
  ```

- **Average linkage:** for the dissimilarity matrix below and k=2, the original
  labels are `[1,2,2,1]`, while kodama's average linkage yields `[1,2,2,2]` after
  numbering clusters by their smallest original member. Relabeling cannot repair
  this partition difference. Do not perturb tied distances or change saved models.

  ```text
  0 2 3 1
  2 0 0 0
  3 0 0 2
  1 0 2 0
  ```

- **Contours:** a 2×2 row-major surface `[2,0,0,2]` at level 0.5 produces one
  connected contour with the current mean-of-corners saddle rule, versus two
  with contour 0.13.1. `[1,0,0,1]` also differs. Source inspection confirms the
  candidate uses fixed saddle cases. Padding or coordinate conversion cannot
  restore topology, so the current geometry remains.

## Acceptance evidence

The benchmark default remains exact. The opt-in refactor profile verifies each
record's fingerprint and file digests before semantic comparisons, and uses
≤1.05 peak RSS ratio and ≤1.05 upper wall-time confidence bound. Tests exercise
large exact counts, all six knowledge kinds, nullable/default behavior, original
column order, reordered reader columns, structured JSON and hostile report text.

Observation write/read on 10,000 exact-count rows passed with the final code:
wall ratio **0.810**, 95% interval **0.803–0.823**, peak RSS ratio **0.605**.
The original schema-loading path was also measured diagnostically: cold upper
wall bound 1.036 and RSS 1.015; repeated-load upper wall bound 1.060 and RSS
1.016. The latter exceeds the generic timing threshold; that flag remains in
the raw report. Schema loading uses the original implementation and is not an
admission case for the observations adapter. No diagnostic was selectively rerun.
Raw pairs and binary/fixture hashes:
`target/simplification/library-bench/results-final/results.json`.

Both report pilots passed two warmups plus 20 alternating pairs. Clean's wall
ratio was 1.005 (upper 95% bound 1.027), RSS 0.978. PeacoQC's was 1.022
(upper bound 1.048), RSS 1.017. HTML tags, attributes, visible text, CSS and
Plotly script data compare after entity/whitespace normalization; every
non-report artifact also compares, and each original output hash and execution
fingerprint is verified by the Rust helper. Raw results:
`target/simplification/report-performance-final/results.json`.

`RUSTC_WRAPPER= just verify` passed: 757 Rust tests, zero failures, three existing
ignored tests, plus the Python checks and packaging of all 26 crates. The release
binary and comparison helper built successfully. Final FCS mutation verification:
29 tested, 26 caught, three unviable, zero survivors. Unviable mutations do not
count as evidence of test strength.

The first full CLI benchmark used two warmups and 20 measured pairs on each of
the five required large synthetic cases. Four passed. Compensation's median
wall ratio was 1.016, but its upper 95% bound was 1.066, exceeding 1.05; RSS was
0.999. This failed run is retained at
`target/simplification/performance-final/results.json`. A single fixed 80-pair
confirmation across the entire matrix passed all five cases, with two warmups
per case. No thresholds were changed or failed cases omitted.

| Synthetic case | Median wall ratio | 95% interval | Peak RSS ratio | Result |
| --- | ---: | --- | ---: | --- |
| Compensation | 1.012 | 0.993–1.038 | 0.992 | Pass |
| Arcsinh, one thread | 0.997 | 0.979–1.008 | 1.000 | Pass |
| Arcsinh, four threads | 1.005 | 0.993–1.028 | 1.000 | Pass |
| Logicle, one thread | 1.003 | 0.986–1.013 | 1.002 | Pass |
| Logicle, four threads | 1.007 | 0.997–1.033 | 1.000 | Pass |

Ratios are candidate / baseline. Raw pairs, host details and hashes are in
`target/simplification/performance-confirmation-80/results.json`; the scorecard
is alongside it. These are warm-cache measurements including input reading,
hashing, publication and provenance, without durable-flush latency. Each output
comparison passed the refactor profile. No scientific algorithm was replaced;
the workspace's committed-fixture parity smoke tests passed within `just verify`.
The external R/Python parity and real-data suites were not run.

## Maintained-code measurements

Counts are physical lines, including comments/blank lines. For implementation
counts, trailing `#[cfg(test)] mod tests` blocks are excluded consistently before
and after. HTML/CSS templates are included; tests and development tooling are
reported separately. These are not language-aware SLOC measurements.

| Area | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Input resolution and its existing callers/error adapters | 4,788 | 4,596 | −192 |
| Observation table | 199 | 198 | −1 |
| Clean/PeacoQC report implementation and templates | 692 | 670 | −22 |
| Product implementation total | 5,679 | 5,464 | **−215** |

The benchmark profile adds 157 Rust implementation lines and 11 Python lines,
so the combined non-test implementation/template reduction is **47 lines**.
Additional regression tests are counted separately; manifest/documentation
changes are not presented as implementation savings. The larger observations
benefit is reduced conversion work and memory, rather than line count.

All pre-existing locked package versions and per-consumer external dependency
features/default-feature settings are preserved. No scientific tolerance or
upstream fixture changed.

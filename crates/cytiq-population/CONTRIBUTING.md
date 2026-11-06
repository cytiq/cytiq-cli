# Contributing to cytiq-population

Workspace guidance lives in [CONTRIBUTING.md](../../CONTRIBUTING.md).

All documents reject unknown versions and invalid references. Keep measured integers exact, preserve missing states, and never infer biological partitions from parent links. Tests are offline and must not depend on sibling repositories. Keep public formats and CLI examples in README.md current.

## Fuzzing

`fuzz/` holds cargo-fuzz targets outside the workspace; what they check lives
in `fuzz/checks.rs`. Run one with `just fuzz cytiq-population <target> [seconds]` (the
nightly that `mise.toml` pins). `tests/fuzz_regressions.rs` includes
`fuzz/checks.rs` and replays every file in `tests/fuzz_regressions/<target>/`,
so `cargo test` covers what the fuzzer found. Hand-written seeds pin a verdict
by name: `valid*` must be accepted, other names rejected. When the fuzzer finds
a crash, copy the input to `tests/fuzz_regressions/<target>/crash-<sha1>`, fix
the bug, and add a named test for it next to the code it exercises. Never name
those directories `corpus` or `artifacts`: `fuzz/.gitignore` ignores both.

Targets: `document_json` and `document_yaml` (`serialization::parse`: never
panics; accepted documents are mappings with a stable digest) and
`observations_parquet` (`table::read_from`: accepted rows validate).

A crash inside a dependency, not this crate, goes to
`tests/fuzz_upstream/<target>/` with an ignored test naming the dependency
version. `observations_parquet` finds one in seconds: parquet 60.1.0 panics on
corrupt definition levels (`arrow-buffer`: "offset + len out of bounds"), so
the target is not in the CI fuzz matrix. Observation tables are only read
from packages whose digests `verify_package` checked, but the release
profile aborts on panic, so such a file would abort `cytiq` instead of
failing with exit code 2. After upgrading parquet, run
`cargo test -p cytiq-population --test fuzz_regressions -- --ignored`.

## Contracts

`contracts/cytiq-tree.lock.json` pins the vendored cytiq-tree schemas by
revision and raw-byte sha256; `tests/contracts.rs` checks every vendored file
against it. To bump the pin, copy the schemas from the new revision, update
the lock, and run `just contracts-upstream`.

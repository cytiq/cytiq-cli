# cytiq-inspect

Inspect FCS files through two independent APIs:

| Module | CLI command | Purpose |
| --- | --- | --- |
| [`meta`](docs/meta.md) | `cytiq meta` | Export metadata, raw TEXT keywords and file identity |
| [`stats`](docs/stats.md) | `cytiq stats` | Summarize channels, detector limits, histograms and acquisition time |

```rust,no_run
use cytiq_inspect::{meta::read_metadata, stats::{DEFAULT_BINS, read_stats}};

let metadata = read_metadata("raw.fcs")?;
let statistics = read_stats("raw.fcs", DEFAULT_BINS)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Both modules read without modifying inputs. Reports keep their existing JSON
schemas and `tool` names. `meta::FileIdentity` includes the input path;
`stats::FileIdentity` records the name, byte count and SHA-256 only.

## Rust API migration

This package replaces `cytiq-meta` and `cytiq-stats` directly, without
compatibility packages. Replace both Cargo dependencies with `cytiq-inspect`
at the matching workspace release version and update imports:

| Previous import | Replacement |
| --- | --- |
| `cytiq_meta::{…}` | `cytiq_inspect::meta::{…}` |
| `cytiq_stats::{…}` | `cytiq_inspect::stats::{…}` |

Types, functions, constants and error variants retain their names within each
module. This is a breaking Rust API change; CLI commands and report schemas
are preserved. The refactor does not bump the workspace version; the release
workflow handles the version bump and release notes.

Before the first release containing this package, follow the repository's
[first-publication procedure](../../CONTRIBUTING.md#cratesio-publication) and
configure trusted publishing for `cytiq-inspect`.

## Tests

```bash
cargo test -p cytiq-inspect
cargo test -p cytiq-engine --test meta_cli --test stats_cli
```

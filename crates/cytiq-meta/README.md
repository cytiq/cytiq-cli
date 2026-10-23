# cytiq-meta

Exports the metadata of one FCS file as a single JSON document. Backs the
`cytiq meta` command.

Derived FCS files written by other `cytiq` commands carry over the source's
instrument and acquisition keywords, but drop or rename keywords that no longer
describe their values (for example the spillover matrix after `compensate` or
`transform`). Run `cytiq meta` on raw files first to keep the full record.

## What is recorded

- file identity: path, file name, size in bytes, SHA-256 of the raw bytes
- FCS version (`FCS3.0` / `FCS3.1` / `FCS3.2`)
- TEXT / DATA / ANALYSIS segment offsets, from the HEADER with a fallback to
  `$BEGINDATA`/`$ENDDATA` and `$BEGINANALYSIS`/`$ENDANALYSIS` for large files
- event count (decoded rows, falling back to `$TOT`)
- every parameter with its name, label, data type, width and `$Pn*` keywords
- every primary and supplemental TEXT keyword as `[key, value]` pairs, in file
  order and exactly as stored

The document shape is documented in the root [README](../../README.md#meta)
and versioned by `schema_version` (currently `1`).

## Usage

```bash
cytiq meta --input raw.fcs | jq '.text | length'
cytiq meta --input raw.fcs --output raw.meta.json --overwrite
```

From Rust:

```rust
use cytiq_meta::{MetaConfig, MetaRunner, read_metadata};

// In-memory only.
let metadata = read_metadata("raw.fcs")?;
println!("{} events", metadata.event_count);

// Full command: writes pretty JSON to a file, or to stdout when `output` is `None`.
let result = MetaRunner::run(&MetaConfig {
    input: "raw.fcs".into(),
    output: Some("raw.meta.json".into()),
    overwrite: false,
})?;
```

## Public API

| Item | Purpose |
| --- | --- |
| `read_metadata` | Read and hash one FCS file into `FcsMetadata` |
| `MetaRunner::run` / `MetaConfig` / `MetaRunResult` | The `cytiq meta` command: read, serialize, write |
| `FcsMetadata`, `FileIdentity`, `SegmentOffsets`, `ParameterMetadata`, `ParameterWidthMetadata` | Serializable document types |
| `MetaError` | Error type for every failure mode |

## Output rules

- `output: None` writes to stdout; logs go to stderr, so piping is safe.
- An existing output file is an error unless `overwrite` is set.
- Files are written through a `<output>.tmp` sibling and renamed, so a failed
  run never leaves a truncated document.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for parsing
- `serde` / `serde_json` for the document, `sha2` for hashing

## Tests

End-to-end behaviour is covered by `crates/cytiq-cli/tests/meta_cli.rs`:

```bash
cargo test -p cytiq-cli --test meta_cli
```

# Contributing to cytiq-inspect

Workspace guidance lives in [CONTRIBUTING.md](../../CONTRIBUTING.md).

## Layout

- `src/lib.rs` exposes `meta` and `stats` without flattening their APIs.
- Each module has a `runner.rs` with its config, report types and runner,
  and an `error.rs` with its command-specific error enum.
- `src/stats/summary.rs` contains the pure summary, percentile, binning and
  clock-reset calculations and their unit tests.

## Invariants

- Both JSON schemas are public contracts, separately versioned by each
  runner's `SCHEMA_VERSION`. Update the relevant [metadata](docs/meta.md) or
  [statistics](docs/stats.md) document if fields or meaning change.
- Keep the two `FileIdentity` types distinct: only metadata includes a path.
- Metadata TEXT and supplemental TEXT preserve raw values and file order.
  Segment offsets prefer the HEADER, with TEXT fallback only when it is empty.
- SHA-256 and byte count describe raw bytes, hashed separately from parsing.
- Statistics record measurements, without thresholds or pass/fail decisions.
  Detector limits agree with `cytiq_fcs::MarginBounds::detector_limit`;
  scatter classification uses `is_scatter_channel`, and Time matches `$PnN`
  without case sensitivity.
- Both runners check overwrite before reading and publish atomically through
  `cytiq-publish`. Metadata stdout contains only its JSON document.
- FCS parsing belongs in `cytiq-fcs`; CLI parsing stays in `cytiq-engine`.

## Testing

```bash
cargo test -p cytiq-inspect
cargo test -p cytiq-engine --test meta_cli --test stats_cli
```

Keep numerical unit tests next to `summary.rs`; CLI tests cover fixtures,
stdout/file output, hashing, overwrite handling and errors.

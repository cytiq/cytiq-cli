# cytiq-mask

Per-event masks shared by cytiq stages, and the `cytiq mask combine` command.

Stages never delete events from the raw file. They write their per-event
results as a mask: a Parquet file with one row per event of the file they ran
on. `source_event_id` ties each row to the raw acquisition, so a mask applies
to any file derived from it, whatever rows that file kept.

## Schema

| Column | Type | Meaning |
| --- | --- | --- |
| `event_id` | `UInt64` | 0-based row in the file the mask was computed on |
| `source_event_id` | `UInt64` | 0-based row in the raw file |
| filter columns | nullable `Boolean` | pass/fail; null means "not assessed" |
| `keep` | `Boolean` | whether later stages should use the event |

Written by `cytiq compensate`, `cytiq peacoqc`, `cytiq clean` and
`cytiq gate --membership`; read by `--mask` on `gate`, `flowsom` and `clean`.

## Usage

```bash
cytiq mask combine --mask a.masks.parquet --mask b.masks.parquet --output analysis.parquet
```

From Rust:

```rust
use cytiq_mask::EventMask;

let raw = EventMask::read_parquet("sample.comp.masks.parquet".as_ref())?;
let qc = EventMask::read_parquet("sample.peacoqc.masks.parquet".as_ref())?;
let analysis = EventMask::combine(&[raw, qc])?;

let dataset = cytiq_fcs::FcsReader::open("sample.peacoqc.fcs")?.read()?;
let keep: Vec<bool> = analysis.keep_for(&dataset)?; // one entry per dataset row
```

## Public API

| Item | Purpose |
| --- | --- |
| `EventMask` | In-memory mask: `new`, `with_filter`, `read_parquet`, `write_parquet`, `combine`, `keep_for`, `kept` |
| `read_combined` | Read and combine the masks passed with `--mask` (`None` for none) |
| `MaskCombineRunner::run` / `MaskCombineConfig` | The `cytiq mask combine` command |
| `EVENT_ID_COLUMN`, `SOURCE_EVENT_ID_COLUMN`, `KEEP_COLUMN` | Reserved column names |
| `MaskError` | Error type for every failure mode |

## Rules

- `combine` keys rows by `source_event_id` and follows the first mask's rows.
  An event is kept only when every mask has it and keeps it. Filter columns
  are concatenated; a repeated name is prefixed with the 1-based index of its
  mask (`2.margin_pass`).
- `keep_for` matches dataset rows to mask rows through the dataset's
  `Original_ID` (or row number when it has none). A dataset row missing from
  the mask is an error: the mask belongs to another acquisition.
- Reading rejects masks without `keep`/`source_event_id`, with nulls in them,
  with non-Boolean filter columns, or with a repeated `source_event_id`.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for event lineage (`source_event_ids`)
- `arrow` / `parquet`

## Tests

```bash
cargo test -p cytiq-mask
cargo test -p cytiq-cli --test mask_cli
```

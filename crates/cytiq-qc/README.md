# cytiq-qc

Checks one sample's final event mask against the QC policy of its panel in
the run manifest. Backs `cytiq qc`.

```bash
cytiq qc --manifest run.yaml --sample S01 --mask sample.analysis.parquet --output qc.json
```

The policy is the panel's `qc` section (`cytiq_manifest::QcPolicy`):

```yaml
qc:
  min_events: 10000
  min_kept_events: 3000
  max_removed_fraction: 0.6
  filters:
    peacoqc_pass: { required: true, max_removed_fraction: 0.3 }
    live: { max_removed_fraction: 0.5, severity: warn }
```

## Checks

| Check | Scope | Fails when | Warns when |
| --- | --- | --- | --- |
| `mask_acquisition` | sample | the mask belongs to another raw acquisition | |
| `mask_coverage` | sample | the mask has no row for a raw event | |
| `policy` | sample | | (passes with a note when the panel has no `qc`) |
| `min_events` | sample | the raw file has fewer events | |
| `min_kept_events` | sample | the mask keeps fewer events | |
| `max_removed_fraction` | sample | `1 - kept / raw events` is larger | |
| `filter_assessed` | `filter <name>` | a `required` filter is missing or assessed no event (`severity: fail`) | the same with `severity: warn` |
| `filter_removed_fraction` | `filter <name>` | the filter removed more of the events it assessed (`severity: fail`) | the same with `severity: warn` |

Columns `cytiq mask combine` prefixed after a repeated name (`2.live`) count
as the same filter: an event is assessed when any column is non-null and
removed when any is `false`.

## Public API

| Item | Purpose |
| --- | --- |
| `QcRunner::run` / `QcConfig` | Run the checks and write `qc.json` |
| `QcRunner::input_files` | The manifest, the sample FCS and the mask, for provenance |
| `QcReport` (`ensure_passed`) | The report: status, counts, metrics, checks |
| `QcMetrics`, `FilterMetrics` | Raw and kept events; per filter: columns, assessed, removed |
| `evaluate` | The policy rules over metrics, without files |
| `QcCheck`, `CheckStatus` | One finding: `pass`, `warn`, `fail`, with `value` and `limit` |
| `QcError` | Error type |

## Dependencies

[`cytiq-fcs`](../cytiq-fcs) (acquisition identity),
[`cytiq-manifest`](../cytiq-manifest) (`QcPolicy`),
[`cytiq-mask`](../cytiq-mask), `serde` / `serde_json`.

## Tests

```bash
cargo test -p cytiq-qc
cargo test -p cytiq-cli --test qc_cli
cargo test -p cytiq-cli --test pipeline_cli   # the golden chain runs qc
```

# cytiq-preflight

Checks a run manifest and its FCS files before any computation. Backs
`cytiq preflight`.

```bash
cytiq preflight --manifest run.yaml --output preflight.json
```

The report is always written. The command exits with `1` when any check
fails; failures and warnings are also logged.

## Checks

| Check | Scope | Fails when | Warns when |
| --- | --- | --- | --- |
| `fcs_readable` | every file | the FCS cannot be parsed | |
| `events` | every file | the file has no events | |
| `raw_signal` | every file | `cytiq transform`, `compensate` or `autospectral` already processed it | |
| `acquisition_metadata` | every file | | `$CYT` or `$DATE` is missing |
| `panel_marker` | samples | a required marker matches no channel, or several | an optional marker is missing; `channel`'s `$PnS` names another marker |
| `unique_labels` | samples | | two channels share a `$PnS` |
| `spillover` | conventional samples | no, invalid, mismatched or singular spillover matrix | its 1-norm condition number exceeds `SPILLOVER_CONDITION_WARN` (100) |
| `primary_channel` | controls with one | the channel is not in the file | |
| `detector_configuration` | every file but the first | the `$PnN` set differs from the first sample | a `$PnV` voltage differs |
| `unique_files` | manifest | | a file is listed twice |

A marker without `channel` matches a parameter whose `$PnS` or `$PnN` equals
it, or whose `$PnS` starts with it as a word (`CD3 BUV395`), ignoring case;
`CD3` does not match `CD38`.

## Public API

| Item | Purpose |
| --- | --- |
| `PreflightRunner::run` / `PreflightConfig` | Run the checks and write the report |
| `PreflightReport` (`ensure_passed`) | The report: status, counts, files, checks |
| `Check`, `CheckStatus` | One finding: `pass`, `warn`, `fail` |
| `SPILLOVER_CONDITION_WARN` | Condition-number warning threshold |
| `PreflightError` | Error type |

## Dependencies

[`cytiq-fcs`](../cytiq-fcs), [`cytiq-manifest`](../cytiq-manifest),
`serde` / `serde_json`.

## Tests

```bash
cargo test -p cytiq-cli --test preflight_cli
```

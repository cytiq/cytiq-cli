# cytiq-peacoqc

PeacoQC-compatible, peak-based acquisition quality control: removes unstable
acquisition regions (clogs, drift, bursts) from compensated and transformed FCS
files. Backs the `cytiq peacoqc` command.

- Paper: Emmaneel et al., *PeacoQC* (<https://doi.org/10.1002/cyto.a.24501>)
- Upstream R implementation: <https://github.com/saeyslab/PeacoQC>

## Algorithm

Events are processed in file (acquisition) order:

1. optional margin removal: drop events at the detector boundaries
2. split events into overlapping bins
3. per bin and channel, find peaks of a `stats::density`-compatible KDE
   (`FindThemPeaks`), giving a bin × peak-feature matrix
4. Isolation Tree (`it`) on the peak matrix, when there are at least 150 bins
5. MAD outliers (`mad`) on `smooth.spline(spar = 0.5)`-smoothed peak series
6. drop good regions shorter than the consecutive-bins limit

`--method` selects `all` (IT + MAD), `mad`, `it`, or `peaks-only`. Tuning
values (MAD 6, IT limit 0.6, 5 consecutive bins, ...) are fixed `DEFAULT_*`
constants aligned with upstream defaults. On the parity dataset the mask
matches R PeacoQC 1.20.0 at 100% in every mode
([`parity/REPORT.md`](../../parity/REPORT.md), section `peacoqc`). Full CLI
rules are in the root [README](../../README.md#peacoqc).

## Usage

```bash
cytiq peacoqc --input sample.fcs --output sample.peacoqc.fcs \
  --channel "355_378/28_uH-A [CD3 BUV395]" --channel "405_750/40_vB-A [CD4 BV750]" \
  --method all --remove-margins --debug-dir ./peacoqc-debug
```

From Rust:

```rust
use cytiq_peacoqc::{PeacoQcConfig, PeacoQcMethod, PeacoQcRunner, parse_channel_list};

let mut config = PeacoQcConfig::new(parse_channel_list("CD3 BUV395,CD4 BV750")?);
config.method = PeacoQcMethod::Mad;
let result = PeacoQcRunner::run("sample.fcs", "sample.peacoqc.fcs", &config)?;
println!("kept {}", result.summary.metrics.events_after);
```

## Public API

| Item | Purpose |
| --- | --- |
| `PeacoQcRunner::run` / `PeacoQcConfig` / `PeacoQcResult` | The `cytiq peacoqc` command |
| `PeacoQcMethod` | `All`, `Mad`, `It`, `PeaksOnly` |
| `PeacoQcSummary`, `PeacoQcParameters`, `PeacoQcMetrics` | `summary.json` contents |
| `EventMaskRecord` | Per-event result (`mask.csv` row) |
| `parse_channel_list` | Split a comma-separated `--channels` value |
| `DEFAULT_*` | Fixed algorithm constants |
| `PeacoQcError` | Error type for every failure mode |

## Output rules

- Input should be compensated and transformed. A pending spillover matrix, or
  selected channels without `CYTIQ_TRANSFORM` coverage, are reported in
  `summary.warnings` and the log; the run continues.
- Input and output are FCS; an output path without a `.fcs` extension is an
  error. An existing output file is replaced.
- The output is FCS 3.1 with the retained events of every input parameter,
  plus an `Original_ID` column (the 1-based input row) unless the input already
  has one. It is float32, or float64 when float32 would change a value (for
  example `Original_ID` above 2^24). Values are unchanged, so every source
  keyword is kept, including the spillover matrix and the original `$PnR`.
- `<output>.masks.parquet` holds one row per input event in the
  [`cytiq-mask`](../cytiq-mask) layout: `event_id`, `source_event_id`,
  `margin_pass`, `peacoqc_pass` and `keep` (null = not assessed).
- `remove_margins` uses raw detector bounds, or the bounds recorded in
  `CYTIQ_MARGINS` by `cytiq transform`. Channels transformed or compensated
  without recorded bounds are an error.
- Channels match `$PnN`, then `$PnS`, then `$PnN [$PnS]`.
- With `debug_dir`, writes `summary.json`, `mask.csv` and `report.html`
  (Plotly from the CDN).
- `summary.json` `status` is `WARN` when `warnings` is not empty, otherwise
  `PASS`. It does not judge how much was removed; panel thresholds are
  checked by `cytiq qc`.
- `threads: Some(n)` runs in a local rayon pool.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading and writing FCS
- [`cytiq-mask`](../cytiq-mask) for `<output>.masks.parquet`
- [`cytiq-density`](../cytiq-density) for the `stats::density` port
- `rayon`, `csv`, `serde_json`, `plotly`

## Tests

```bash
cargo test -p cytiq-peacoqc
cargo test -p cytiq-cli --test peacoqc_cli
```

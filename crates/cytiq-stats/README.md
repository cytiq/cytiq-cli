# cytiq-stats

Per-channel statistics of one FCS file for technical QC. Backs the
`cytiq stats` command.

One pass over the decoded events records what a QC policy needs, so a policy can
change later without reading millions of events again:

- per channel (`$PnN`, `$PnS`, `kind`: `scatter`, `time` or `fluorescence`):
  finite and non-finite counts, min, max, mean, standard deviation, 1st/50th/99th
  percentiles (nearest rank);
- `at_detector_limit`: events at or above the detector limit `$PnR - 1`
  (saturated or margin events), the same limit `cytiq compensate`'s
  `margin_pass` mask uses (`cytiq_fcs::MarginBounds::detector_limit`);
- `at_or_below_zero`;
- a histogram of `bins` equal-width bins (default 256): linear over
  `[min(0, min), max($PnR, max)]` for scatter and time, arcsinh (cofactor 150)
  for fluorescence;
- for a `Time` channel: whether it is monotonic (and how often it decreases),
  first and last values, `$TIMESTEP`, the duration in seconds, and the event
  count per time bin (100 bins), from which an event-rate stability check
  follows.

Values are the stored values (`$PnE`/`$PnG` not applied): saturation is a
property of the detector's raw output. Thresholds and decisions are the caller's.

## Output

```json
{
  "schema_version": 1,
  "tool": "cytiq stats",
  "version": "0.2.1",
  "file": { "name": "sample.fcs", "bytes": 521360, "sha256": "..." },
  "fcs_version": "FCS3.1",
  "event_count": 10000,
  "parameter_count": 13,
  "bins": 256,
  "channels": [
    { "index": 1, "name": "FSC-A", "label": null, "kind": "scatter", "range": 262144.0,
      "detector_limit": 262143.0, "finite": 10000, "non_finite": 0, "min": 0.0, "max": 262143.0,
      "mean": 80012.3, "sd": 21003.1, "p1": 33410.2, "p50": 79893.2, "p99": 126400.5,
      "at_detector_limit": 38, "at_or_below_zero": 0,
      "histogram": { "scale": "linear", "cofactor": null, "lo": 0.0, "hi": 262144.0, "counts": [0, 3, "..."] } }
  ],
  "time": { "channel": "Time", "monotonic": true, "decreases": 0, "first": 0.0, "last": 9199.1,
            "timestep": 0.01, "duration_seconds": 91.99, "counts": [100, 101, "..."] }
}
```

`time` is `null` without a `Time` channel. Statistics of a channel without
finite values are `null`.

## Usage

```bash
cytiq stats --input sample.fcs --output sample.stats.json
cytiq stats --input sample.fcs --output sample.stats.json --bins 128 --overwrite
```

The command writes `sample.stats.json.provenance.json` next to the report.

From Rust:

```rust
use cytiq_stats::{DEFAULT_BINS, read_stats};

let report = read_stats("sample.fcs", DEFAULT_BINS)?;
let saturated = report.channels.iter().map(|channel| channel.at_detector_limit).max();
```

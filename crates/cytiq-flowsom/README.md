# cytiq-flowsom

Deterministic FlowSOM-style clustering of the events of one FCS file. Backs the
`cytiq flowsom` command.

- Paper: <https://doi.org/10.1002/cyto.a.22625>
- Upstream R reference: <https://github.com/saeyslab/FlowSOM> (parity target
  FlowSOM 2.18.0, checked against `src/som.c`)

## Pipeline

1. Read one FCS file and select channels (`$PnN`, then `$PnS`, then
   `"$PnN [$PnS]"`). Values are used as stored: no compensation or transform
   is applied here, so run `cytiq transform` first when needed.
2. Scale each channel to mean 0 / SD 1 (`base::scale`, n − 1).
3. Train a 10×10 SOM as `FlowSOM::SOM`: codes initialised from distinct events,
   `10 · n` random draws with replacement, linear learning rate 0.05 → 0.01,
   bubble neighbourhood on the Chebyshev grid distance with radius from the
   0.67 quantile of grid distances (6) down to 0; below 1 only the BMU is
   updated.
4. Assign every event to its nearest node.
5. Merge the 100 nodes into 20 metaclusters by average-linkage hierarchical
   clustering of the codebook.

All settings are fixed `FLOWSOM_*` constants. Deliberate differences from R
defaults: `scale = TRUE`, 20 metaclusters (R: 10), and plain average linkage
instead of ConsensusClusterPlus. After the parity fixes Rust-vs-R ARI is
0.93–0.98, the same as R seed-to-seed; see
[`parity/REPORT.md`](../../parity/REPORT.md) §2.5. CLI rules are in the root
[README](../../README.md#flowsom).

## Usage

```bash
cytiq flowsom --input sample.fcs --output sample.flowsom.csv --channels CD3,CD4,CD8
cytiq flowsom --input sample.fcs --output sample.flowsom.csv \
  --channel CD3 --channel CD4 --debug-dir ./flowsom-debug --threads 4
cytiq flowsom --input big.fcs --output big.flowsom.csv --channels CD3,CD4,CD8 \
  --xdim 12 --ydim 12 --metaclusters 25 --training-events 200000
```

From Rust:

```rust
use cytiq_flowsom::{FlowSomConfig, FlowSomRunner, parse_channel_list};

let mut config = FlowSomConfig::new(parse_channel_list("CD3,CD4,CD8")?);
config.debug_dir = Some("./flowsom-debug".into());
let result = FlowSomRunner::run("sample.fcs", "sample.flowsom.csv", &config)?;
println!("{} metaclusters", result.summary.metrics.metaclusters);
```

## Public API

| Item | Purpose |
| --- | --- |
| `UPSTREAM` | Upstream implementations and versions the crate is validated against (`parity/versions.txt`), recorded in provenance `algorithms` |
| `FlowSomConstants` | The fixed training schedule, recorded in provenance `constants` |
| `FlowSomRunner::run(input, output, &FlowSomConfig)` | Run the pipeline and write the assignment CSV |
| `FlowSomConfig` | `channels`, optional `debug_dir`, optional `threads`, `masks`, `xdim`, `ydim`, `metaclusters`, `seed`, `scale`, `training_events` (defaults from `FLOWSOM_*`) |
| `FlowSomResult` | `summary`, per-event `assignments`, `nodes`, `metaclusters` |
| `FlowSomSummary`, `FlowSomParameters`, `FlowSomMetrics`, `FlowSomScaleSummary` | Run summary, written as `summary.json` |
| `FlowSomAssignment`, `FlowSomNode`, `FlowSomMetacluster` | Output rows |
| `FLOWSOM_*` constants | Defaults for grid size, metaclusters, seed (1) and scaling; fixed iterations, learning rate and radius schedule |
| `parse_channel_list` | Parse a comma-separated `--channels` value |
| `FlowSomError` | Error type for every failure mode |

## Output rules

- `output` is a CSV with one row per clustered event: `event_index` (0-based
  row in the input file), `source_event_id` (0-based row in the raw file, from
  `Original_ID` when present), `flowsom_node` (1..=100) and
  `flowsom_metacluster` (1..=20). The file is created or replaced.
- With `masks` (CLI `--mask`), only events every mask keeps are scaled,
  trained on, assigned and listed. A mask that keeps nothing is an error.
- With `debug_dir`: `summary.json`, `nodes.csv` (grid position, metacluster,
  event count, codebook), `metaclusters.csv` and a Plotly `report.html`.
- Join results from different stages on `source_event_id`, not on
  `event_index`, which counts rows of this input only.
- Empty inputs, missing channels and `threads = Some(0)` are errors.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading and event lineage
- [`cytiq-mask`](../cytiq-mask) for `--mask`
- `rayon`, `csv`, `serde` / `serde_json`, `plotly`

## Tests

```bash
cargo test -p cytiq-flowsom
cargo test -p cytiq-cli --test flowsom_cli
```

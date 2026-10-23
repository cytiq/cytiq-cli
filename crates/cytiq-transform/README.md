# cytiq-transform

Applies analysis-ready signal transformations (arcsinh, logicle, hyperlog,
FlowJo biexponential) to the channels of one or more FCS files and writes new
FCS 3.1 files. Backs the `cytiq transform` command.

The `scales` module also exports the reference display scales used by
[`cytiq-gating`](../cytiq-gating) (`Logicle`, `Hyperlog`) and
[`cytiq-autospill`](../cytiq-autospill) (`FlowJoBiexp`).

## Methods

| `TransformMethod` | Definition | Output scale |
| --- | --- | --- |
| `Arcsinh` (default) | `asinh(value / cofactor)`, default cofactor 150 (flowCore `arcsinhTransform`) | unbounded |
| `Logicle` | Moore & Parks (2012) logicle, as `flowCore::logicleTransform` | `[0, M]` |
| `Hyperlog` | Gating-ML 2.0 hyperlog, as `flowCore::hyperlogtGml2` / FlowKit | `[0, 1]` |
| `Biexponential` | `flowWorkspace::flowjo_biexp(256, 262144, 4.418539922, 0, -100)`: cytolib's calibration table with R's natural cubic spline | `[0, 256]` |
| `None` | identity (channels are copied) | — |

Logicle and hyperlog share `LogicleParameters` (`t`, `m`, `w`, `a`, defaults
`262144`, `4.5`, `0.5`, `0`) and reject values outside `2W <= M` and
`-W <= A <= M - 2W`.

Full flag reference and rules: root [README](../../README.md#transform).
Parity against flowCore / flowWorkspace / FlowKit:
[`parity/REPORT.md`](../../parity/REPORT.md) section 2.1.

## Usage

```bash
cytiq transform --input sample.fcs --output sample.transformed.fcs
cytiq transform --input "samples/*.fcs" --method logicle --output transformed/
```

From Rust:

```rust
use cytiq_transform::{TransformConfig, TransformMethod, TransformRunner, parse_channel_list};

let mut config = TransformConfig::new(vec!["sample.fcs".into()], "out.fcs".into());
config.method = TransformMethod::Logicle;
config.channels = parse_channel_list("CD3,CD4,CD8")?;
let result = TransformRunner::run(&config)?;

// Scales on their own:
use cytiq_transform::scales::Logicle;
let logicle = Logicle::new(262_144.0, 0.5, 4.5, 0.0)?; // t, w, m, a
let y = logicle.scale(1_000.0); // [0, 1] Gating-ML scale
```

## Public API

| Item | Purpose |
| --- | --- |
| `TransformRunner::run` / `TransformConfig` | The `cytiq transform` command: resolve inputs, transform, write FCS + sidecars |
| `TransformMethod`, `LogicleParameters`, `BiexponentialParameters`, `TransformParameters`, `TransformFormat` | Method selection and parameters |
| `TransformRunResult`, `TransformFileResult`, `TransformChannelMetrics`, `TransformValueMetrics` | Per-file plan and per-channel raw/transformed statistics |
| `parse_channel_list` | Parse a comma-separated channel list |
| `scales::{Logicle, Hyperlog, FlowJoBiexp, ScaleParameterError}` | Reference scales with `scale` / `inverse` |
| `TransformError` | Error type for every failure mode |

## Inputs, channels and outputs

- Inputs: `.fcs` files, directories (immediate `.fcs` children) and simple
  file-name globs such as `samples/*.fcs`.
- One input writes to `output`; several inputs, or an existing directory as
  `output`, write `<stem><suffix>.fcs` (default suffix `.transformed`) into it.
- Without `channels`, every channel except obvious acquisition channels
  (`Time`, event/cell length, `FSC-A/H/W`, `SSC-A/H/W`) is transformed.
  Channels match `$PnN`, then `$PnS`, then `$PnN [$PnS]`.
- Only `TransformFormat::Fcs` is implemented; `Csv` and `Parquet` are rejected.
- Values are written as `f32` FCS 3.1 with `$PnN`/`$PnS` and a
  `CYTIQ_TRANSFORM` keyword holding the transform metadata JSON. Other source
  keywords are not carried over (use `cytiq meta` first).
- Existing outputs and the `metadata_json` sidecar are errors unless
  `overwrite` is set; writing over the input itself goes through a temporary
  sibling and a rename.
- Non-finite input values in a selected channel, or non-finite outputs, fail
  the run.
- `dry_run` resolves inputs, channels and outputs without writing anything.
- `debug_dir` gets `summary.json`, `metrics.csv` and a Plotly `report.html`.
- Files and columns are processed in parallel with Rayon; `threads` caps the
  pool size.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading and writing FCS
- `rayon` for parallelism, `plotly` for the debug report, `serde_json` / `csv`
  for sidecars

## Tests

```bash
cargo test -p cytiq-transform
cargo test -p cytiq-cli --test transform_cli
```

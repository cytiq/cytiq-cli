# cytiq-density

Density-estimation and scatter auto-gating primitives shared by the `cytiq`
workflow crates. It has no dependencies and no CLI command of its own.

| Consumer | Uses |
| --- | --- |
| [`cytiq-autospill`](../cytiq-autospill) | `gate_events`, `GateComputation` for the FSC/SSC gate |
| [`cytiq-peacoqc`](../cytiq-peacoqc) | `density` for per-channel peak detection |
| [`cytiq-clean`](../cytiq-clean) | `density`, `find_valleys`, `gate_events`, `quantile_type7`, `median`, `mad` |

## What it provides

- **`density`** — a step-by-step port of R's `stats::density.default` with the
  defaults PeacoQC relies on: Gaussian kernel, `bw = "nrd0"`, `adjust = 1`,
  `cut = 3`, `n = 512`. Linear binning onto a `2n` grid (R's `BinDist`), FFT
  circular convolution with the kernel sampled on R's `kords` grid, then linear
  interpolation (`approx`) back onto 512 points between
  `min(x) - 3 * bw` and `max(x) + 3 * bw`.
- **`bandwidth_nrd0`**, **`quantile_type7`**, **`median`**, **`mad`** — R's
  `bw.nrd0`, default (type 7) `quantile`, `median` and `mad` (`1.4826 *
  median(|x - center|)`).
- **`gate_events`** — the AutoSpill FSC/SSC auto-gate
  ([paper](https://www.nature.com/articles/s41467-021-23126-8), upstream R
  `autospill` `do.gate` / `get_autospill_param_minimal.r`): trims the outer 1%
  tails, a two-pass KDE peak search that ignores lower-left debris, then a
  convex hull around the dense core. Returns the gated event indices, the hull
  vertices and peak-search diagnostics in `GateComputation`.
- **`convex_hull`**, **`point_in_polygon`** — the geometry used by the gate.
- **`find_valleys`** — density minima between well-separated populations of a
  1-D KDE (for example live/dead or debris/cells). Peaks below
  `DEFAULT_MIN_PEAK_FRACTION` (5%) of the tallest are dropped, and neighbouring
  peaks joined by a valley shallower than `DEFAULT_MAX_VALLEY_RATIO` (0.5 of the
  lower peak) are merged. An empty result means "not clearly multimodal".

## Usage

```rust
use cytiq_density::{
    DEFAULT_MAX_VALLEY_RATIO, DEFAULT_MIN_PEAK_FRACTION, density, find_valleys, gate_events,
};

let estimate = density(&values); // finite values, at least two observations
let valleys = find_valleys(&estimate, DEFAULT_MIN_PEAK_FRACTION, DEFAULT_MAX_VALLEY_RATIO);

// FSC/SSC pairs; `data_max` is the instrument range ($PnR) used to clamp the scatter range.
let gate = gate_events(&points, Some(262_144.0))?;
println!("{} events in gate", gate.gated_indices.len());
```

## Public API

| Item | Purpose |
| --- | --- |
| `density`, `Density` | R `stats::density` on a 512-point grid (`x`, `y`) |
| `bandwidth_nrd0`, `quantile_type7` | R `bw.nrd0` and type-7 quantile (input must be sorted) |
| `median`, `mad` | Median and R-scaled MAD of unsorted values |
| `gate_events`, `GateComputation` | AutoSpill FSC/SSC auto-gate; errors with a `&'static str` reason on degenerate input |
| `convex_hull`, `point_in_polygon` | 2-D hull and inclusion test (points on the boundary count as inside) |
| `find_valleys`, `Valley`, `DEFAULT_MIN_PEAK_FRACTION`, `DEFAULT_MAX_VALLEY_RATIO` | Valleys between populations of a 1-D density |

## Numeric parity

`density` and `bandwidth_nrd0` are checked against R 4.5 reference values
(`|Δx| < 1e-10`, `|Δy| < 1e-9`). The AutoSpill gate reproduces the upstream
gates on the parity datasets; see [`parity/REPORT.md`](../../parity/REPORT.md)
sections 2.2 and 2.4.

## Tests

```bash
cargo test -p cytiq-density
```

Unit tests live next to the code in `src/kde.rs`, `src/valley.rs` and
`src/scatter_gate.rs`. The R reference values for `density` are included from
`data/fixtures/peacoqc/density_reference.rs`.

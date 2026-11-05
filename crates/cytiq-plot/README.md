# cytiq-plot

FlowJo-style plots of one FCS file as vector SVG (2-D plots and 1-D
histograms), and the `cytiq plot` command. Optionally writes the same plot as Plotly figure JSON for interactive
viewers.

| Type | Shows |
| --- | --- |
| `density` | Pseudocolor: events binned on a `bins × bins` grid (default 256), coloured by log count |
| `scatter` | Dots, sampled evenly to `--max-points` (default 10,000) per layer |
| `contour` | Equal-probability contours (each band holds `--contour-step` of the events, default 10%; the events outside the outer contour are drawn as dots) |
| `histogram` | One channel (`--x`): events per bin (`--bins`) as a step outline on a linear count axis, removed events as a second outline in red. It takes no `--y` and draws no gates (`--population` still applies) |

On any type:

- `--mask` (repeatable, combined with AND) plots the kept events and draws the
  removed ones over them in red, counted in the legend (`--removed hide` drops
  them). Drawn underneath, removed events that share a density cell with kept
  ones would not show at all.
- `--gate` draws Gating-ML gates (polygon, rectangle, range, 2-D ellipsoid)
  with `id xx.x%` labels, the percentage of parent with the mask applied,
  which is `cytiq gate`'s `percent_of_parent` for the same masks.
- `--population` restricts the plot to one gate's events.
- `--color-by` colours dots by class: `cytiq classify` predictions (`label`)
  or a `cytiq flowsom` CSV (`--class-column`, default `flowsom_metacluster`).

A plot never depends on QC passing. A sample whose mask removes every event
still renders, annotated "no events left after the mask", so a failed sample
can be inspected.

## Usage

```bash
# Acquisition QC on the raw file
cytiq plot --input sample.fcs --x FSC-A --y SSC-A --output fsc_ssc.svg

# Post-QC: kept events, removed events overlaid
cytiq plot --input sample.fcs --mask sample.analysis.parquet \
  --x FSC-A --y SSC-A --output fsc_ssc.qc.svg

# Post-analysis: CD4 vs CD8 of T cells with the gates drawn
cytiq plot --input sample.t.fcs --mask sample.analysis.parquet \
  --gating-ml gates.xml --population T --gate CD4T --gate CD8T \
  --type contour --output t_cd4_cd8.svg

# FlowSOM metaclusters
cytiq plot --input sample.t.fcs --color-by flowsom.csv --type scatter \
  --x CD3 --y CD19 --output clusters.svg

# Several plots of one sample, SVG and Plotly JSON
cytiq plot --input sample.t.fcs --mask sample.analysis.parquet --gating-ml gates.xml \
  --config plots.yaml --format svg --format plotly --output plots/
```

`--x`/`--y` take `$PnN`, `$PnS` or `"$PnN [$PnS]"`. With `--gate` they can
be omitted: the axes are the first gate's dimensions.

### Batch file

`--config` reads YAML (`.yaml`, `.yml`) or JSON. `--output` is then a
directory receiving `<name>.svg` and/or `<name>.plotly.json` per plot, plus
one `provenance.json`.

```yaml
version: 1
defaults: { bins: 256, palette: pseudocolor }
plots:
  - { name: fsc_ssc, type: density, x: FSC-A, y: SSC-A }
  - { name: singlets, type: scatter, gates: [singlets] }
  - { name: t_cd4_cd8, type: contour, x: CD4, y: CD8, population: T }
  - { name: classes, type: scatter, x: CD3, y: CD19, color_by: true }
  - { name: hist_cd45, type: histogram, x: CD45, bins: 128 }
```

Plot fields: `name`, `type`, `x`, `y`, `gates`, `population`, `x_scale`,
`y_scale` (`auto`, `linear`, `logicle`, `arcsinh`), `x_range`, `y_range`
(`[lo, hi]` in data units), `bins`, `max_points`, `palette` (`pseudocolor`,
`viridis`), `removed` (`overlay`, `hide`), `contour_step`, `color_by`,
`width`, `height`, `title`. `defaults` takes every field except `name` and
`type`. Unknown fields are errors. Names are `[A-Za-z0-9_-]{1,64}` and must be
unique ignoring case.

## Axes

- **Values.** A channel axis plots `linear_values` (`$PnE`, `$PnG`,
  `$TIMESTEP` applied). A gate dimension axis plots the values the gate is
  evaluated on (`GatingStrategy::dimension_values`: compensation and
  transformation per Gating-ML), so outlines and events share one space.
- **Scale.** `auto` is linear for scatter (`FSC*`, `SSC*`, a Navios's `FS INT`
  and `SS INT`: the channels `cytiq transform` leaves alone), `Time*` and for channels
  already transformed (`CYTIQ_TRANSFORM` or a Gating-ML transformation). Other
  raw channels use logicle with `T = $PnR`, `W = 0.5`, `M = 4.5`, `A = 0`.
  `logicle` and `arcsinh` (cofactor 150) are refused on transformed data.
- **Ticks.** Log-like axes are labelled in raw decades (`0`, `10³`, `−10³`)
  placed through the recorded transform, as FlowJo does. Linear axes use 1-2-5
  steps (`50K`).
- **Range.** The automatic range covers the 0.05–99.95% quantiles of all
  events of the file, padded by 5%. It ignores masks, so plots of one file
  share axes. A `--population` past it widens it to the population's own
  quantiles, so a rare bright population (mast cells, 0.002% of a bone-marrow
  sample) is drawn where it is instead of on the edge. It includes zero on
  log-like axes and starts at 0 on non-negative scatter channels. Events beyond
  it are drawn on the edge and counted in a note.

## Output

- **Vector SVG only.** Density cells are one `<path>` per colour level with
  merged runs (about 1 MB at worst for 256² cells). Points are small squares,
  one `<path>` per colour. Gate outlines are paths, densified on non-linear
  axes so straight Gating-ML edges bend with the scale. Before publishing,
  the SVG is parsed and rejected if it holds anything but plot elements: no
  `<image>`, `<script>`, `<foreignObject>`, `href` or `data:`.
- **Safe text.** Text from FCS keywords, gate ids and class labels is
  XML-escaped, in the Plotly JSON too.
- **Embeddable.** Element ids carry the plot name, so several plots can be
  inlined in one page.
- **Deterministic.** The same inputs give byte-identical files, with no
  timestamps. The plot name comes from the output file stem.
- **Plotly JSON.** The same plot in unit coordinates with explicit tick
  values. Density is a heatmap of colour levels, contours and gates are line
  traces, and gate labels are annotations.
- **Atomic.** All files of a run are published together through
  `cytiq-publish`, so a failing plot writes none. `--overwrite` replaces
  existing files.

Plotly's own static export is not used: plotly.js draws heatmaps into a canvas
and embeds them in SVG as PNG, and the export needs Chromium.

## Public API

| Item | Purpose |
| --- | --- |
| `PlotRunner::run` / `PlotConfig` / `PlotJob` | The `cytiq plot` command: read once, compose, render, publish |
| `PlotRequest`, `AxisOptions`, `PlotKind`, `ScaleChoice`, `Palette`, `RemovedLayer`, `OutputFormat` | One plot and its options; `PlotRequest::validate` |
| `ClassSource` | `--color-by` file and optional CSV column |
| `load_batch`, `BATCH_VERSION` | Batch file → jobs with output paths |
| `plot_name_for` | Plot name from an output path |
| `PlotRunSummary`, `PlotSummary`, `PlotStats` | Per plot: axes, outputs, events shown/removed/drawn/off-scale |
| `DEFAULT_*`, `MIN_*`, `MAX_*`, `DENSITY_LEVELS`, `ARCSINH_COFACTOR`, `DEFAULT_CLASS_COLUMN` | Option defaults and bounds, shared with the CLI parsers |
| `PlotError` | Error type for every failure mode |

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs): reading the FCS, lineage, signal state.
- [`cytiq-mask`](../cytiq-mask): `--mask`.
- [`cytiq-gating`](../cytiq-gating): gates and populations.
- [`cytiq-transform`](../cytiq-transform): `ValueTransform` for ticks on
  transformed axes, `scales::Logicle` for logicle axes.
- [`cytiq-density`](../cytiq-density): quantiles, sampling, robust spread.
- [`cytiq-publish`](../cytiq-publish): atomic publication.
- `plotly` (JSON only, no features), `roxmltree` (SVG self-check),
  `arrow`/`parquet` (predictions), `csv` (FlowSOM), `serde-saphyr` (batch
  YAML).

## Tests

```bash
cargo test -p cytiq-plot
cargo test -p cytiq-engine --test plot_cli
```

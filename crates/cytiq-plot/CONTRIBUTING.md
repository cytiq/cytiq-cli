# Contributing to cytiq-plot

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs`: crate docs and re-exports only.
- `src/config.rs`: `PlotConfig`, `PlotJob`, `PlotRequest` and its option
  enums, defaults and bounds (`DEFAULT_*`, `MIN_*`, `MAX_*`),
  `PlotRequest::validate`, `plot_name_for`.
- `src/batch.rs`: `load_batch`, the YAML/JSON batch schema (`version: 1`,
  `defaults`, `plots`).
- `src/sample.rs`: `Sample`, everything a run reads, loaded once. It holds
  the FCS, the combined mask, the Gating-ML strategy with unmasked results,
  and the class labels.
- `src/classes.rs`: classify predictions Parquet and FlowSOM CSV, joined on
  `source_event_id`.
- `src/compose.rs`: `PlotRequest` + `Sample` → `PlotSpec`.
  - Axis sources (channel or gate dimension).
  - Event selection.
  - Layers: density, dots, classes, contours, removed overlay, gate outlines
    and labels.
  - Notes and legend.
- `src/axis.rs`: data space (`DataTransform`), display scale (`Display`),
  automatic range, decade and 1-2-5 ticks.
- `src/bin.rs`: 2-D histogram on the unit square and colour levels.
- `src/contour.rs`:
  - Scott's-rule bandwidth and separable Gaussian smoothing.
  - Equal-probability thresholds.
  - Marching squares and stitching.
- `src/geometry.rs`: gate outlines (polygon, rectangle, range, ellipse) in
  unit coordinates.
- `src/palette.rs`: sequential palettes (pseudocolor, viridis), categorical
  classes, fixed colours.
- `src/spec.rs`: the renderer-neutral `PlotSpec`, `Frame` layout and margins.
- `src/svg.rs`: `PlotSpec` → SVG, `escape`, and `check` (the self-check).
- `src/plotly_json.rs`: `PlotSpec` → Plotly figure JSON.
- `src/runner.rs`: `PlotRunner`. Checks outputs, loads once, renders every
  job and publishes all files together.
- `src/error.rs`: `PlotError` (`thiserror`).

## Invariants

- **No raster, no active content.** The SVG writer emits only the elements in
  `svg::ALLOWED_ELEMENTS`, and `svg::check` runs on every SVG before it is
  staged. Do not add `<image>`, `<foreignObject>`, `<script>`, `<style>`,
  `href` or external `url(...)`. A new element type goes into the allow-list
  with a test.
- **Escape everything from the data.** Channel names, `$PnS`, gate ids, class
  labels and file names go through `svg::escape`, in SVG and in Plotly JSON
  (Plotly interprets HTML tags in text).
- **Deterministic output.**
  - Numbers have fixed precision.
  - Layers keep spec order.
  - Contour stitching keys on integer edge ids.
  - `HashMap`s are used for lookup, never iterated into output.
  - Sampling is `cytiq_density::sample_indices` (evenly spaced, no RNG).
  - Do not add timestamps or paths beyond the plot name and the sample name.
- **Renderers draw what `compose` decides.** `svg.rs` and `plotly_json.rs`
  only map a `PlotSpec`. Event selection, ranges, ticks, sampling and text
  belong in `compose.rs` and `axis.rs`, so both formats show the same plot.
- **Forward transforms only.** Axis ranges live in display-scale space and
  ticks map raw decades forward through the recorded transform
  (`cytiq_transform::ValueTransform`, `cytiq_gating::Transform::apply`). Do
  not re-implement transform formulas here.
- **Axes come from all events of the file.** Masks and populations never
  change a range: before/after-QC plots must line up.
- **Gate percentages use unmasked gating ANDed with the mask.** This equals
  `cytiq gate --mask` because the mask enters every gate before its parent
  (`masked_membership_equals_unmasked_and_keep`,
  `gate_labels_match_cytiq_gate_percent_of_parent`). If `cytiq-gating`
  changes that rule, this must change with it.
- **QC failure is not a plot failure.** An empty selection renders with a
  note. Errors are for unreadable or mismatched inputs and invalid requests.
- **No dependency on `cytiq-classify` or `cytiq-flowsom`.** They pull ONNX
  Runtime and clustering; their output columns are read directly
  (`classes.rs`), and the CLI tests pin the contract against real `cytiq
  flowsom` output.

## Where changes belong

- New flags, `--help` text and log lines go in
  `crates/cytiq-cli/src/commands/plot.rs`. Value parsers go in
  `crates/cytiq-cli/src/parsers.rs`. Keep this crate free of `clap`.
- A new plot option goes in three places: a `PlotRequest` field with its
  default, `PlotRequest::validate`, and the batch schema in `batch.rs` (both
  `PlotDefaults` and `PlotEntry`). Then document it in the README.
- Display scales and transform math belong in `cytiq-transform`. Gate
  semantics belong in `cytiq-gating`.

## Testing

```bash
cargo test -p cytiq-plot
cargo test -p cytiq-cli --test plot_cli
```

Unit tests sit next to the code:

- `axis.rs`: ticks and ranges.
- `bin.rs`: binning and levels.
- `contour.rs`: radial bump, mass enclosed by the outer contour, saddles,
  bandwidth.
- `geometry.rs`: ellipse on the Mahalanobis boundary, swapped dimensions,
  open bounds, densification.
- `svg.rs`: merged runs, self-check, escaping, determinism, the 1M-event size
  bound.
- `classes.rs`: joins and acquisition checks.
- `batch.rs`: schema errors.
- `compose.rs`: mask/gating equivalence.

`crates/cytiq-cli/tests/plot_cli.rs` runs the binary on the clean, gating and
pipeline fixtures. It covers every plot type, masks, gates (checked against
`cytiq gate`), populations, FlowSOM and classify classes, batch output in both
formats, byte-identical reruns, `--overwrite`, and input and flag errors.

To look at a plot while working on rendering, write it to a scratch directory
and open it in a browser. On macOS, `qlmanage -t -s 640 -o out/ plot.svg`
gives a PNG thumbnail.

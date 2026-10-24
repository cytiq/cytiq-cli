# cytiq-autospectral

Reference-based spectral unmixing from single-stain controls. Backs the
`cytiq autospectral` command.

Builds one reference spectrum per fluorophore plus one autofluorescence
spectrum, then unmixes every fully stained sample by ordinary least squares and
writes the abundances as FCS files.

## Algorithm

Follows the OLS path of AutoSpectral ([Burton et al., bioRxiv 2025](https://doi.org/10.1101/2025.10.27.684855); citation in [`CITATIONS.md`](../../CITATIONS.md#autospectral))
(parity target 1.8.7, see [`parity/REPORT.md`](../../parity/REPORT.md#23-autospectral)):

1. Load controls and samples in parallel; infer each single stain's
   `primary_channel` from `$PnN`, then `$PnS`, when the manifest omits it.
2. Resolve spectral detectors: the `--detectors` manifest, or every channel of
   the first control present in all files, excluding FSC/SSC (any `fsc*` /
   `ssc*` name) and `Time`.
3. Scatter gate each control at the 1st–99th percentile box on FSC and SSC,
   or with `clean_controls` (CLI `--clean-controls`) drop raw detector-margin
   events and keep the `AutoSpill` KDE scatter gate (`cytiq_density::gate_events`).
4. Autofluorescence spectrum = mean of the gated AF control, divided by
   `max |x|`.
5. For each single stain, positives are events above the 99th percentile
   (R type 7) of the AF control's primary channel, keeping the brightest 500
   (at least 20). Spectrum = mean of positives − AF mean, divided by its
   maximum (peak normalisation, as in `get.fluorophore.spectra`).
6. Check conditioning: warn when the fluorophore-only condition number exceeds
   the number of fluorophores; fail above `1e8` for the full matrix.
7. Unmix each sample with the least-squares matrix `M⁺ = R⁻¹Qᵀ` (Householder
   QR; equal to `(MᵀM)⁻¹Mᵀ` but without squaring the condition number) in event
   chunks of 8192 and record per-sample residual and negative-fraction QC.

Abundances are in units of each fluorophore's peak detector. Known gaps against
upstream (box gate instead of the KDE gate, mean instead of `rlm` slope, no
WLS/Poisson/per-cell AF) are listed in the parity report. Solver, gate
quantiles and event limits are fixed public constants, not configuration.

## Usage

```bash
cytiq autospectral \
  --controls controls.csv --samples samples.csv \
  --input-dir ./fcs --output ./autospectral-out \
  [--detectors detectors.csv] [--debug-dir ./debug] \
  [--fsc FSC-A] [--ssc SSC-A] [--threads 4]
```

From Rust:

```rust
use cytiq_autospectral::{
    AutoSpectralConfig, AutoSpectralRunner, read_controls_manifest, read_samples_manifest,
};

let controls = read_controls_manifest("controls.csv")?;
let samples = read_samples_manifest("samples.csv")?;
let config = AutoSpectralConfig::new("autospectral-out".into());
let summary = AutoSpectralRunner::run(&controls, &samples, "./fcs", &config)?;
println!("condition number {:.3e}", summary.condition_number);
```

Manifest formats are documented in the root
[README](../../README.md#autospectral).

## Public API

| Item | Purpose |
| --- | --- |
| `UPSTREAM` | Upstream implementations and versions the crate is validated against (`parity/versions.txt`), recorded in provenance `algorithms` |
| `AutoSpectralRunner::run` / `AutoSpectralConfig` | Full pipeline: load, build references, unmix, write outputs |
| `AutoSpectralSummary` | Returned run summary (also `diagnostics/run.json`) |
| `read_controls_manifest`, `read_samples_manifest`, `read_detectors_manifest` | CSV readers with validation |
| `ControlManifestRow`, `SampleManifestRow`, `DetectorSpec` | Manifest rows |
| `write_outputs` | Output writer used by the runner |
| `SOLVER`, `POSITIVITY_THRESHOLD`, `POSITIVE_EVENT_LIMIT`, `MIN_*`, `CONDITION_NUMBER_LIMIT`, `SCATTER_GATE_*`, `AUTO_EXCLUDED_CHANNELS`, ... | Fixed algorithm constants |
| `AutoSpectralError` | Error type for every failure mode |

## Manifest rules

- `filename` values must be relative; they resolve against `input_dir`.
- Exactly one control with `dye` `Autofluorescence` (or `AF`) and at least one
  single stain; dyes must be unique.
- `fluorophore` and `channel` are accepted as aliases for `dye` and
  `primary_channel`; an empty `primary_channel` means "infer".
- Channel matching ignores case and a trailing `-A` / `-H` / `-W`.

## Outputs

```text
<output>/
  references/fluorophore_spectra.csv        component,kind,<detectors...>
  references/autospectral-references.json   detectors, spectra, condition number
  unmixed/<sample-stem>.unmixed.fcs         non-detector columns + one column per component
  unmixed/<sample-stem>.unmixed.masks.parquet  raw detector `margin_pass` per event
  diagnostics/run.json                      AutoSpectralSummary
<debug-dir>/summary.json                    same summary, only with --debug-dir
```

- Unmixed files are FCS 3.1 float32. Detector columns are replaced by component
  columns; other parameters (scatter, time, ...) are copied with their `$Pn*`
  keywords. Acquisition keywords are carried over; the spillover matrix and
  earlier `CYTIQ_*` keywords are dropped. `CYTIQ_UNMIXING` lists the components
  and detectors, and `CYTIQ_MARGINS` the bounds of the copied channels.
- Duplicate sample stems get a `-<row>` suffix.
- Existing output files are replaced; there is no `--overwrite` check.

## Dependencies

- [`cytiq-fcs`](../cytiq-fcs) for reading and writing FCS
- `nalgebra` (SVD, inverse), `rayon` (parallel loading and unmixing),
  `csv`, `serde` / `serde_json`

## Tests

```bash
cargo test -p cytiq-autospectral
cargo test -p cytiq-cli --test autospectral_cli
```

Integration tests run on `data/fixtures/autospectral/` (unstained, FITC and PE
controls plus a stained sample).

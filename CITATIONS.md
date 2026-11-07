# Citations

This file lists the external publications and upstream references that should
be cited when using algorithmic functionality implemented in `cytiq`.

At the current stage of the repository, the published or preprint-backed
algorithmic implementations are:
- AutoSpill and AutoSpread in `crates/cytiq-autospill`, exposed through
  `cytiq autospill` and `cytiq autospread`
- AutoSpectral-style reference-based spectral unmixing in
  `crates/cytiq-autospectral`, exposed through `cytiq autospectral`
- CytoScan-style anomaly detection in `crates/cytiq-cytoscan`, exposed through
  `cytiq cytoscan`
- PeacoQC-style acquisition quality control in `crates/cytiq-peacoqc`,
  exposed through `cytiq peacoqc`
- CompensAID spillover-error detection (Secondary Stain Index) in
  `crates/cytiq-spillqc`, exposed through `cytiq spillqc`, with the
  flowDensity cut-off search in `crates/cytiq-density`
- FlowSOM-style clustering in `crates/cytiq-flowsom`, exposed through
  `cytiq flowsom`
- Gating-ML 2.0 gate evaluation in `crates/cytiq-gating`, exposed through
  `cytiq gate`
- debris/doublet/dead-cell cleaning in `crates/cytiq-clean`, exposed through
  `cytiq clean`. Its `--debris-mode population` reuses the AutoSpill FSC/SSC
  auto-gate (cite AutoSpill), and its gates are exported as Gating-ML 2.0
  (cite Gating-ML 2.0). The other filters are standard density-valley and
  median/MAD heuristics without a dedicated publication.

## AutoSpill

If you use `cytiq autospill`, `cytiq autospread`, or reuse the implementation
in `crates/cytiq-autospill`, cite the AutoSpill paper:

Roca, C.P., Burton, O.T., Gergelits, V. et al. AutoSpill is a principled
framework that simplifies the analysis of multichromatic flow cytometry data.
*Nature Communications* 12, 2890 (2021).
<https://doi.org/10.1038/s41467-021-23126-8>

Project links:
- Paper: <https://www.nature.com/articles/s41467-021-23126-8>
- Upstream R reference implementation: <https://github.com/carlosproca/autospill>

The current Rust implementation follows that publication and the upstream R
reference for the scope implemented in this repository:
- FSC/SSC automatic gating using density maxima search, tessellation-style peak
  selection, MAD-bounded refinement, and convex-hull gating
- robust linear regression for spillover coefficient estimation
- iterative spillover-matrix refinement and compensation-matrix inversion
- square and rectangular spillover / compensation matrix export
- AutoSpread-style spillover spreading matrix estimation

Relevant code:
- `crates/cytiq-autospill/src/gating.rs`
- `crates/cytiq-autospill/src/regression.rs`
- `crates/cytiq-autospill/src/runner.rs`

## AutoSpectral

If you use `cytiq autospectral`, or reuse the implementation in
`crates/cytiq-autospectral`, cite the AutoSpectral preprint:

Burton, O.T., Bücken, L., De Vuyst, L. et al. AutoSpectral improves
spectral flow cytometry accuracy through optimised spectral unmixing and
autofluorescence-matching at the cellular level. *bioRxiv* (2025).
<https://doi.org/10.1101/2025.10.27.684855>

Project links:
- Preprint: <https://www.biorxiv.org/content/10.1101/2025.10.27.684855v1>
- DOI: <https://doi.org/10.1101/2025.10.27.684855>

The current Rust implementation is an MVP reference-based spectral unmixing
pipeline aligned with the repository's manifest/input model:
- single-stain reference spectra and one autofluorescence component
- automatic or manifest-provided spectral detector selection
- quantile-based scatter and positive-event filtering for controls
- ordinary least-squares unmixing with condition-number validation
- per-sample unmixed FCS export plus reference and diagnostics outputs

Relevant code:
- `crates/cytiq-autospectral/src/runner.rs`
- `crates/cytiq-autospectral/src/export.rs`
- `crates/cytiq-autospectral/src/manifest.rs`

## CytoScan

If you use `cytiq cytoscan`, or reuse the implementation in
`crates/cytiq-cytoscan`, cite the CytoScan preprint:

Mocking, T.R. et al. CytoScan: automated flagging of technical anomalies for
cytometry quality control. *bioRxiv* (2025).
<https://doi.org/10.1101/2025.09.24.678276>

Project links:
- Preprint: <https://www.biorxiv.org/content/10.1101/2025.09.24.678276v1>
- Upstream R reference implementation: <https://github.com/AUMC-HEMA/CytoScan>
- Manuscript repository: <https://github.com/AUMC-HEMA/CytoScan-manuscript>

The current Rust implementation is aligned with the repository's FCS
input/output model:
- file-level feature generation using summary statistics, quantiles, EMD
  (`transport::wasserstein1d`), aggregate binning (`cut()` semantics), and
  `flowFP` probability-binning fingerprints
- spillover compensation and arcsinh preprocessing from FCS metadata
- `isotree`-style isolation-forest outlier scoring
- reference-based novelty detection with an independently implemented
  mclust-compatible search over eight covariance families and `G = 1..9`
- summary, feature, config, and debug artifact export

Feature values match upstream to ~1e-12 when both use the same events; see
`parity/REPORT.md` for the comparison and the remaining differences.

The novelty model implements mclust-style initialization, covariance families,
BIC selection and singularity rules in `mixture.rs`; please cite mclust when
comparing with upstream:
Scrucca, L. et al. mclust 5: clustering, classification and density estimation
using Gaussian finite mixture models. *The R Journal* 8(1), 289–317 (2016).

Relevant code:
- `crates/cytiq-cytoscan/src/runner.rs`
- `crates/cytiq-cytoscan/src/novelty.rs`
- `crates/cytiq-cytoscan/src/mixture.rs`
- `crates/cytiq-cytoscan/src/export.rs`

## PeacoQC

If you use `cytiq peacoqc`, or reuse the implementation in
`crates/cytiq-peacoqc`, cite the PeacoQC paper:

Emmaneel, A., Quintelier, K., Sichien, D. et al. PeacoQC: Peak-based selection
of high quality cytometry data. *Cytometry Part A* 101, 325-338 (2022).
<https://doi.org/10.1002/cyto.a.24501>

Project links:
- Paper: <https://onlinelibrary.wiley.com/doi/10.1002/cyto.a.24501>
- Upstream R reference implementation: <https://github.com/saeyslab/PeacoQC>

The current Rust implementation is an MVP aligned with the repository's FCS
input/output model:
- optional margin-event removal
- overlapping acquisition-order binning
- per-channel peak detection and peak-feature matrix construction
- MAD filtering, deterministic Isolation Tree-style splitting, and
  consecutive-bin cleanup
- cleaned FCS export plus JSON, mask CSV, and Plotly HTML diagnostics

Relevant code:
- `crates/cytiq-peacoqc/src/runner.rs`
- `crates/cytiq-peacoqc/src/export.rs`

## CompensAID, Secondary Stain Index and flowDensity

If you use `cytiq spillqc`, or reuse the implementation in
`crates/cytiq-spillqc`, cite the CompensAID paper:

Olsman, R., Bonte, S., Hofmans, M. et al. CompensAID: An Automated Detection
Tool for Reference Errors. *Cytometry Part A* 109(2), 124-134 (2026).
<https://doi.org/10.1002/cyto.a.70016>

CompensAID builds on the Secondary Stain Index:

Daniels, K. and Gardner, R. Secondary Stain Index. Memorial Sloan Kettering
Cancer Center (2020).

Its per-channel cut-offs come from flowDensity's `deGate`, ported in
`crates/cytiq-density/src/degate.rs`:

Malek, M., Taghiyar, M.J., Chong, L. et al. flowDensity: reproducing manual
gating of flow cytometry data by automated density-based cell population
identification. *Bioinformatics* 31(4), 606-607 (2015).
<https://doi.org/10.1093/bioinformatics/btu677>

Project links:
- Upstream R reference implementation: <https://github.com/Olsman/CompensAID>
- flowDensity: <https://bioconductor.org/packages/flowDensity>

The Rust implementation ports CompensAID 0.99.7 and runs it after the
vignette's preprocessing (`PeacoQC::RemoveMargins`, `flowCore::compensate`,
`estimateLogicle`, PeacoQC). The batch view, which groups files by the matrix
applied to them and calls flags recurrent, is a cytiq extension without a
publication.

Relevant code:
- `crates/cytiq-spillqc/src/compensaid.rs`
- `crates/cytiq-spillqc/src/preprocess.rs`
- `crates/cytiq-density/src/degate.rs`

## FlowSOM

If you use `cytiq flowsom`, or reuse the implementation in
`crates/cytiq-flowsom`, cite the FlowSOM paper:

Van Gassen, S., Callebaut, B., Van Helden, M.J. et al. FlowSOM: Using
self-organizing maps for visualization and interpretation of cytometry data.
*Cytometry Part A* 87, 636-645 (2015).
<https://doi.org/10.1002/cyto.a.22625>

Project links:
- Paper: <https://onlinelibrary.wiley.com/doi/10.1002/cyto.a.22625>
- Upstream R reference implementation: <https://github.com/saeyslab/FlowSOM>

The current Rust implementation is an MVP aligned with the repository's FCS
input/output model:
- selected marker matrix extraction from one FCS file
- fixed z-score preprocessing
- deterministic self-organizing map training
- deterministic hierarchical metaclustering over SOM nodes
- per-event assignment CSV export plus JSON, node/metacluster CSV, and Plotly
  HTML diagnostics

Relevant code:
- `crates/cytiq-flowsom/src/runner.rs`
- `crates/cytiq-flowsom/src/export.rs`

## Gating-ML 2.0 and FlowKit

If you use `cytiq gate`, or reuse the implementation in `crates/cytiq-gating`,
cite the Gating-ML 2.0 standard:

Spidlen, J., Moore, W., Brinkman, R.R. ISAC's Gating-ML 2.0 data exchange
standard for gating description. *Cytometry Part A* 87, 683-687 (2015).
<https://doi.org/10.1002/cyto.a.22690>

Gate semantics are checked against FlowKit, the reference implementation:

White, S., Quinn, J., Enzor, J. et al. FlowKit: A Python Toolkit for
Integrated Manual and Automated Cytometry Analysis Workflows. *Frontiers in
Immunology* 12, 768541 (2021).
<https://doi.org/10.3389/fimmu.2021.768541>

Project links:
- Specification: <http://flowcyt.sourceforge.net/gating/latest.pdf>
- FlowKit: <https://github.com/whitews/FlowKit>

The Rust implementation evaluates existing gates. It does not estimate gates:
- Gating-ML 2.0 parsing of rectangle, polygon, ellipsoid and Boolean gates,
  gate hierarchies, the `flin`/`flog`/`fasinh`/`logicle`/`hyperlog`
  transformations, and `FCS`/spectrum-matrix compensation
- per-event membership and per-gate counts that match FlowKit 1.3.0 exactly
  on the checked-in fixture

Relevant code:
- `crates/cytiq-gating/src/parse.rs`
- `crates/cytiq-gating/src/geometry.rs`
- `crates/cytiq-gating/src/evaluate.rs`

## Notes

- `cytiq-fcs` implements FCS file parsing, writing, and validation against the
  supported FCS 3.0 / 3.1 / 3.2 layouts. It does not currently introduce a
  separate published analysis algorithm that requires its own citation entry in
  this file.
- As new published methods are added to the repository, append them here with:
  the paper citation, DOI or canonical URL, the repo component that implements
  the method, and a short note describing the implemented scope.

# Citations

This file lists the external publications and upstream references that should
be cited when using algorithmic functionality implemented in `cytiq`.

At the current stage of the repository, the published or preprint-backed
algorithmic implementations are:
- AutoSpill and AutoSpread in `crates/cytiq-autospill`, exposed through
  `cytiq autospill` and `cytiq autospread`
- AutoSpectral-style reference-based spectral unmixing in
  `crates/cytiq-autospectral`, exposed through `cytiq autospectral`
- PeacoQC-style acquisition quality control in `crates/cytiq-peacoqc`,
  exposed through `cytiq peacoqc`
- FlowSOM-style clustering in `crates/cytiq-flowsom`, exposed through
  `cytiq flowsom`

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

## Notes

- `cytiq-fcs` implements FCS file parsing, writing, and validation against the
  supported FCS 3.0 / 3.1 / 3.2 layouts. It does not currently introduce a
  separate published analysis algorithm that requires its own citation entry in
  this file.
- As new published methods are added to the repository, append them here with:
  the paper citation, DOI or canonical URL, the repo component that implements
  the method, and a short note describing the implemented scope.

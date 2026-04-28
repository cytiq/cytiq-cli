# Citations

This file lists the external publications and upstream references that should
be cited when using algorithmic functionality implemented in `cytiq`.

At the current stage of the repository, the published or preprint-backed
algorithmic implementations are:
- AutoSpill and AutoSpread in `crates/cytiq-autospill`, exposed through
  `cytiq autospill` and `cytiq autospread`
- AutoSpectral-style reference-based spectral unmixing in
  `crates/cytiq-autospectral`, exposed through `cytiq autospectral`

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

## Notes

- `cytiq-fcs` implements FCS file parsing, writing, and validation against the
  supported FCS 3.0 / 3.1 / 3.2 layouts. It does not currently introduce a
  separate published analysis algorithm that requires its own citation entry in
  this file.
- As new published methods are added to the repository, append them here with:
  the paper citation, DOI or canonical URL, the repo component that implements
  the method, and a short note describing the implemented scope.

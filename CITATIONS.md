# Citations

This file lists the external publications and upstream references that should
be cited when using algorithmic functionality implemented in `cytiq`.

At the current stage of the repository, the main published algorithmic
implementation is the AutoSpill workflow in `crates/cytiq-autospill`, exposed
through `cytiq autospill`.

## AutoSpill

If you use `cytiq autospill`, or reuse the implementation in
`crates/cytiq-autospill`, cite the AutoSpill paper:

Roca, C.P., Burton, O.T., Gergelits, V. et al. AutoSpill is a principled
framework that simplifies the analysis of multichromatic flow cytometry data.
*Nature Communications* 12, 2890 (2021).
<https://doi.org/10.1038/s41467-021-23126-8>

Project links:
- Paper: <https://www.nature.com/articles/s41467-021-23126-8>
- Upstream R reference implementation: <https://github.com/carlosproca/autospill>

The current Rust implementation follows that publication and the upstream R
reference for the narrow v1 scope implemented in this repository:
- FSC/SSC automatic gating using density maxima search, tessellation-style peak
  selection, MAD-bounded refinement, and convex-hull gating
- robust linear regression for spillover coefficient estimation
- iterative spillover-matrix refinement and compensation-matrix inversion
- square spillover / compensation matrix export

Relevant code:
- `crates/cytiq-autospill/src/gating.rs`
- `crates/cytiq-autospill/src/regression.rs`
- `crates/cytiq-autospill/src/runner.rs`

## Notes

- `cytiq-fcs` currently implements FCS file parsing and validation against the
  supported FCS 3.0 / 3.1 / 3.2 layouts. It does not currently introduce a
  separate published analysis algorithm that requires its own citation entry in
  this file.
- As new published methods are added to the repository, append them here with:
  the paper citation, DOI or canonical URL, the repo component that implements
  the method, and a short note describing the implemented scope.

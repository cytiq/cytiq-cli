# Density reference values

`degate_reference.rs` holds R reference cut-offs for
`cytiq_density::degate_upper_cuts`, included by the unit tests in
`crates/cytiq-density/src/degate.rs`. They are
`flowDensity::deGate(x, all.cuts = TRUE, upper = TRUE, tinypeak.removal = ...)`
on seeded bimodal, trimodal, unimodal, tiny-peak (default and `1e-4` removal),
logicle-like and fewer-than-20-event samples.

Regenerate with `parity/make_fixtures.R` (needs `just parity-setup`). The file
header records the R and flowDensity versions.

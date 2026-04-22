# AutoSpread Synthetic Fixtures

This directory contains synthetic single-color controls for testing `cytiq autospread`.

- `control_fitc.fcs`: primary signal in `FL1-A` with synthetic spread into `FL2-A` and `FL3-A`
- `control_pe.fcs`: primary signal in `FL2-A` with synthetic spread into `FL1-A` and `FL3-A`
- `controls.csv`: manifest for the two controls
- `detectors.csv`: explicit detector order for rectangular `dye x detector` outputs
- `expected_spread.csv`: expected spread matrix exported by `cytiq autospread`

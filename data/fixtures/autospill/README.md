# AutoSpill Synthetic Fixtures

This directory contains synthetic single-color controls for testing `cytiq autospill`.

- `control_a.fcs`: primary signal in `FL1-A`, spillover 0.2 into `FL2-A`
- `control_b.fcs`: primary signal in `FL2-A`, spillover 0.1 into `FL1-A`
- `controls.csv`: manifest for the two controls
- `expected_spillover.csv`: expected spillover matrix
- `expected_compensation.csv`: expected compensation matrix

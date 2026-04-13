# FCS Fixtures

These fixtures are small synthetic FCS files used to exercise the strict
`cytiq-fcs` reader.

- `fcs3_0_integer.fcs`: FCS 3.0 with two 16-bit integer channels.
- `fcs3_1_variable_integer.fcs`: FCS 3.1 with mixed 16-bit and 32-bit integer channels.
- `fcs3_2_mixed.fcs`: FCS 3.2 with mixed integer, float, and double channels.

The files are intentionally tiny and deterministic so they can be checked into
the repository and used in CI.

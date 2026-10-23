# Gating Fixtures

A synthetic sample and GatingML 2.0 strategy with FlowKit reference results,
used by `cytiq-gating` parity tests.

- `sample.fcs`: 4,000 events over `FSC-A`, `SSC-A`, `FL1-A` (CD3), `FL2-A`
  (CD4), `FL3-A` (CD8) and `Time`. It carries `$SPILLOVER` and `$TIMESTEP`
  keywords, and a few events sit exactly on rectangle and polygon boundaries.
- `gates.xml`: polygon (including a self-intersecting one), range, rectangle,
  ellipsoid and Boolean (`and`/`or`/`not`, complement) gates in a nested
  hierarchy. It uses the `flin`, `flog`, `fasinh`, `logicle` and `hyperlog`
  transformations, plus `FCS` and spectrum-matrix compensation.
- `flowkit_reference.json`: for each gate, its path, its event count, and its
  membership as packed bits in hex (`numpy.packbits`, most significant bit
  first). The results come from FlowKit `GatingStrategy.gate_sample`.

Regenerate all three files with:

```bash
parity/.env/py/bin/python parity/gen/gating.py data/fixtures/gating
```

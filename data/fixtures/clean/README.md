# Clean Fixtures

Synthetic samples with a known class for every event, used by the `cytiq clean`
CLI tests.

- `sample.fcs`: 10,000 events over `FSC-A`, `FSC-H`, `FSC-W`, `SSC-A`, `FL1-A`
  (CD3), `FL2-A` (CD19), `FL3-A` (Live/Dead) and `Time`, with `$SPILLOVER`. The
  cells are lymphocytes and monocytes. About 10% of events are low-FSC debris,
  8% of cells are doublets (summed area, slightly higher height), and 15% of
  the remaining singlets are dead (bright Live/Dead, a little smaller and more
  granular).
- `no_viability.fcs`: the same events without the viability channel.
- `control.fcs`: 3,000 live singlets that are unstained for the viability dye.
- `truth.json`: per-class counts, plus the class of every `sample.fcs` event as
  a digit string: `0` live singlet, `1` debris, `2` doublet, `3` dead singlet.

Regenerate with:

```bash
parity/.env/py/bin/python parity/gen/clean.py data/fixtures/clean
```

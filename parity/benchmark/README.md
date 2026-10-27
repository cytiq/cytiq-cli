# spillqc benchmark on real data

How well does `cytiq spillqc` (CompensAID's Secondary Stain Index) find a
wrong spillover coefficient in a real fully stained sample? Parity with R
CompensAID (`just parity`, `just parity-real`) shows the port is exact. This
benchmark measures what the method itself can detect.

```bash
CYTIQ_REAL_SLOW=1 just parity-real   # once: downloads the datasets and runs AutoSpill on the controls
just benchmark-spillqc               # writes parity/results/benchmark_spillqc.json (about 1 minute)
```

`CYTIQ_BIN` selects the binary, as in the parity scripts. The benchmark only
measures; no threshold fails it.

## Data

| Dataset | Use |
| --- | --- |
| `celesta_mdsc_sample` + `celesta_mdsc_controls` (BD FACSCelesta, mouse spleen, 11 colours, 56,458 events; Zenodo 21384789) | Stage 1 and stage 2 |
| `symphony_omip24_controls` (BD FACSymphony, 24 colours) | Reported as not usable |

The FACSymphony set has single-stain controls only. Their `$SPILL` is the
identity, because FACSDiva writes the keyword before compensation is
calculated. So there is no lab matrix to compare and no stained sample to run
spillqc on. The FACSCelesta controls also carry the identity; the lab matrix is
in the stained sample.

## Stage 1: matrix accuracy

The FACSDiva matrix in the stained sample is compared with the matrix
`cytiq autospill` computes from the matching controls. That AutoSpill matrix
equals R AutoSpill to 5e-13 (`parity/REPORT.md` §2.10). spillqc then runs on
the sample with each matrix.

## Stage 2: detection

One off-diagonal coefficient of the sample's FACSDiva matrix is changed at a
time, and spillqc runs on the unchanged events:

- **Relative changes:** ±5, 10, 20, 30, 50%, applied to the 36 coefficients
  of at least 0.01.
- **Absolute changes:** ±0.01, 0.02, 0.05, applied to all 110 coefficients,
  never below 0.

A larger coefficient over-compensates, so its pair should turn `over`; a
smaller one should turn `under`. The pair is primary = the fluorochrome's
channel, secondary = the detector.

The unchanged sample is already flagged, so each run is scored against the
unchanged-matrix run:

| Metric | Definition |
| --- | --- |
| `detected` | The pair carries the expected flag in the run and did not before |
| `recall` | `detected / coefficients changed` |
| `precision` | `detected / (detected + other pairs newly flagged)` |
| `masked` | The pair already carried the expected flag before the change, so it cannot be newly detected |
| `not_assessed` | The pair has no SSI in the run (too few positive or negative events) |
| `median_shift_toward_flag` | The pair's SSI change in the expected direction |
| `new_flags_elsewhere_per_run` | Other pairs newly flagged, per changed coefficient |

## Results (cytiq 0.1.0, 2026-10-27)

**Stage 1.**

- The FACSDiva and AutoSpill matrices agree closely: off-diagonal MAE 0.0021,
  max |Δ| 0.029.
- Only 3 of 110 coefficients differ by 0.01 or more, all into BV510.
- spillqc flags 47 of the 79 assessable pairs with the FACSDiva matrix and 45
  with the AutoSpill matrix; 43 flags are shared.

On this sample the flags come from the data, not from either matrix, as
CompensAID's low precision suggests. Likely sources are myeloid
autofluorescence and co-expression.

**Stage 2** (772 runs):

| Change | Coefficients | Recall | Precision | Masked | Not assessed | Median SSI shift | Other new flags per run |
| --- | --- | --- | --- | --- | --- | --- | --- |
| +5% | 36 | 0.00 | 0.00 | 0 | 14 | 0.02 | 0.25 |
| +10% | 36 | 0.03 | 0.07 | 0 | 13 | 0.07 | 0.36 |
| +20% | 36 | 0.06 | 0.11 | 0 | 13 | 0.14 | 0.47 |
| +30% | 36 | 0.08 | 0.11 | 0 | 12 | 0.29 | 0.69 |
| +50% | 36 | 0.28 | 0.21 | 0 | 11 | 0.68 | 1.06 |
| −5% | 36 | 0.06 | 0.22 | 13 | 13 | 0.03 | 0.19 |
| −10% | 36 | 0.08 | 0.18 | 13 | 13 | 0.06 | 0.39 |
| −20% | 36 | 0.11 | 0.16 | 13 | 14 | 0.09 | 0.58 |
| −30% | 36 | 0.17 | 0.15 | 13 | 14 | 0.09 | 0.92 |
| −50% | 36 | 0.22 | 0.16 | 13 | 14 | 0.14 | 1.14 |
| +0.01 | 110 | 0.11 | 0.19 | 0 | 30 | 0.14 | 0.45 |
| +0.02 | 110 | 0.23 | 0.28 | 0 | 29 | 0.29 | 0.59 |
| +0.05 | 110 | 0.35 | 0.20 | 0 | 28 | 1.20 | 1.46 |
| −0.01 | 36 | 0.11 | 0.29 | 13 | 14 | 0.07 | 0.28 |
| −0.02 | 27 | 0.19 | 0.20 | 10 | 8 | 0.08 | 0.74 |
| −0.05 | 19 | 0.16 | 0.14 | 7 | 7 | 0.09 | 1.00 |

What this means for using spillqc:

- **Small errors are not found.** Errors of 5–20% of a coefficient move the
  SSI by less than 0.15, far below the flag threshold of 1, so recall is at
  most 11%. Even ±50% is found about a quarter of the time.
- **Precision matches the paper.** It is 0.07–0.29, against CompensAID's
  reported 0.16–0.27. Each change also flags other pairs (0.2–1.5 per run)
  through the inverse.
- **A third of the pairs cannot be assessed** on this sample: there are too
  few positive or negative events among the secondary-negative ones.
- **Under-compensation is the harder direction.** In 13 of 36 coefficients the
  pair is already flagged `under`, so lowering the coefficient cannot add a
  flag. Over-compensation shifts the SSI more: 1.2 at +0.05 against 0.09 at
  −0.05.

This is one sample of one panel. CompensAID's published 96% sensitivity comes
from 40 EuroFlow BCP-ALL samples with expert labels. Those samples are
available from the authors on request only, so that benchmark cannot be
reproduced here. Stage 3 (blind refinement) needs a refinement mode that
`cytiq spillqc` does not have yet.

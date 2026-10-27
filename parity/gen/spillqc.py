"""Fully stained samples whose embedded spillover matrix carries two known errors,
for `cytiq spillqc` against CompensAID.

Six markers with realistic co-expression are mixed through the true spillover
matrix. The `$SPILL` keyword holds that matrix with one coefficient halved
(PE into PerCP-Cy5.5: under-compensation, CD4+ CD8- cells keep residual
PerCP-Cy5.5) and one at 1.6x (APC into APC-Cy7: over-compensation, CD45+
HLA-DR- cells are pushed below the HLA-DR negatives). Three tubes share the
matrix, so the batch view sees both errors recur. truth/ holds the true matrix
and the injected pairs.

A share of intermediate events (independent log-uniform levels per marker)
keeps every density valley populated, as in real samples. Across an empty
gap the smoothed density is exactly zero and flowDensity's pick among the
zeros follows round-off, which no port reproduces.

Halving a large coefficient (FITC into PE, 0.22) does not work as a test case:
the residual lifts CD3+ CD4- cells above PE's first density valley, so they
leave CompensAID's secondary-negative population and the pair is not assessed.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pandas as pd

from fcsio import PNR, write_fcs

SEED = 20261027
FLUOR = ["FITC-A", "PE-A", "PerCP-Cy5-5-A", "PE-Cy7-A", "APC-A", "APC-Cy7-A"]
MARKERS = ["CD3", "CD4", "CD8", "CD19", "CD45", "HLA-DR"]
# rows: fluorochrome, columns: detector (FLUOR order)
SPILL = np.array([
    [1.000, 0.220, 0.030, 0.005, 0.000, 0.000],
    [0.015, 1.000, 0.120, 0.060, 0.002, 0.001],
    [0.000, 0.004, 1.000, 0.180, 0.030, 0.050],
    [0.000, 0.020, 0.040, 1.000, 0.004, 0.090],
    [0.000, 0.000, 0.010, 0.002, 1.000, 0.300],
    [0.000, 0.000, 0.003, 0.050, 0.080, 1.000],
])
# (fluorochrome row, detector column, factor applied in the embedded matrix)
ERRORS = [(1, 2, 0.5), (4, 5, 1.6)]
# share, then marker brightness (median linear intensity, 0 = negative)
POPULATIONS = [
    (0.30, [18_000, 9_000, 0, 0, 25_000, 0]),        # CD4 T
    (0.18, [16_000, 0, 7_000, 0, 25_000, 0]),        # CD8 T
    (0.12, [0, 0, 0, 12_000, 22_000, 15_000]),       # B
    (0.10, [0, 2_500, 0, 0, 30_000, 20_000]),        # monocytes
    (0.12, [0, 0, 0, 0, 20_000, 0]),                 # NK / granulocytes
    (0.18, [0, 0, 0, 0, 0, 0]),                      # CD45- events
]
INTERMEDIATE_SHARE = 0.06
INTERMEDIATE_RANGE = (30.0, 30_000.0)


def embedded_spill() -> np.ndarray:
    spill = SPILL.copy()
    for row, column, factor in ERRORS:
        spill[row, column] *= factor
    return spill


def spill_keyword(spill: np.ndarray) -> str:
    values = ",".join(f"{value:g}" for value in spill.ravel())
    return f"{len(FLUOR)},{','.join(FLUOR)},{values}"


def sample(rng: np.random.Generator, n_events: int) -> np.ndarray:
    shares = np.array([share for share, _ in POPULATIONS])
    levels = np.array([level for _, level in POPULATIONS], dtype=float)
    population = rng.choice(len(POPULATIONS), n_events, p=shares / shares.sum())
    brightness = levels[population] * rng.lognormal(0.0, 0.35, (n_events, len(FLUOR)))
    intermediate = rng.random(n_events) < INTERMEDIATE_SHARE
    low, high = np.log(INTERMEDIATE_RANGE)
    brightness[intermediate] = np.exp(rng.uniform(low, high, (intermediate.sum(), len(FLUOR))))
    true = brightness + rng.normal(0.0, 40.0, brightness.shape)  # autofluorescence
    observed = true @ SPILL
    shot = rng.normal(0.0, 1.0, observed.shape) * np.sqrt(2.0 * np.clip(observed, 0, None))
    observed = observed + shot + rng.normal(0.0, 60.0, observed.shape)
    observed = np.clip(observed, -500.0, PNR - 1)
    time = np.sort(rng.uniform(0.0, 600.0, n_events)) * 100.0
    fsc = np.clip(rng.normal(90_000, 12_000, n_events), 0, PNR - 1)
    ssc = np.clip(0.6 * fsc + rng.normal(0, 7_000, n_events), 0, PNR - 1)
    return np.column_stack([time, fsc, ssc, observed])


def gen_spillqc(out: Path, n_events: int = 30_000, n_files: int = 3) -> None:
    rng = np.random.default_rng(SEED)
    channels = ["Time", "FSC-A", "SSC-A", *FLUOR]
    labels = ["", "", "", *MARKERS]
    keyword = spill_keyword(embedded_spill())
    for index in range(n_files):
        write_fcs(out / f"tube_{index + 1}.fcs", sample(rng, n_events), channels, labels=labels,
                  extra_keywords={"$SPILLOVER": keyword})
    truth = out / "truth"
    truth.mkdir(parents=True, exist_ok=True)
    pd.DataFrame(SPILL, index=FLUOR, columns=FLUOR).rename_axis("channel").to_csv(truth / "spillover.csv")
    pd.DataFrame([
        {"primary_channel": FLUOR[row], "secondary_channel": FLUOR[column],
         "status": "under" if factor < 1 else "over"}
        for row, column, factor in ERRORS
    ]).to_csv(truth / "expected_flags.csv", index=False)

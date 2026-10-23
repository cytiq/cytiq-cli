"""Synthetic samples with known debris, doublets and dead cells for `cytiq clean`.

Usage: python clean.py <out_dir>

Writes:
- `sample.fcs`: 10,000 events over FSC-A/H/W, SSC-A, FL1-A (CD3), FL2-A (CD19),
  FL3-A (Live/Dead) and Time, with `$SPILLOVER`.
- `no_viability.fcs`: the same events without the viability channel.
- `control.fcs`: 3,000 live singlets without viability staining (unstained
  control for the viability channel).
- `truth.json`: per-class counts and the class of every `sample.fcs` event as
  a digit string: 0 live singlet, 1 debris, 2 doublet, 3 dead singlet.

The committed copy under `data/fixtures/clean/` is produced by this script.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np

from fcsio import write_fcs

SEED = 20261023
EVENTS = 10_000
CONTROL_EVENTS = 3_000
DEBRIS_FRACTION = 0.10
DOUBLET_FRACTION = 0.08
DEAD_FRACTION = 0.15
CHANNELS = ["FSC-A", "FSC-H", "FSC-W", "SSC-A", "FL1-A", "FL2-A", "FL3-A", "Time"]
LABELS = ["", "", "", "", "CD3", "CD19", "Live/Dead", ""]
SPILL = np.array([
    [1.00, 0.10, 0.02],
    [0.06, 1.00, 0.12],
    [0.01, 0.04, 1.00],
])
# FSC-W = WIDTH_SCALE * FSC-A / FSC-H, like a cytometer's pulse width.
WIDTH_SCALE = 50_000.0


def _cells(rng: np.random.Generator, count: int, dead: np.ndarray) -> dict[str, np.ndarray]:
    """Lymphocytes (70%) and monocytes (30%) with per-event pulse shape."""
    mono = rng.random(count) < 0.30
    fsc = np.where(mono, rng.normal(110_000, 12_000, count), rng.normal(70_000, 8_000, count))
    ssc = np.where(mono, rng.normal(55_000, 9_000, count), rng.normal(22_000, 5_000, count))
    # Dying cells shrink and get granular.
    fsc = np.where(dead, fsc * 0.85, fsc)
    ssc = np.where(dead, ssc * 1.2, ssc)
    ratio = rng.normal(1.15, 0.03, count)
    cd3 = np.where(~mono & (rng.random(count) < 0.7), rng.normal(20_000, 5_000, count),
                   rng.normal(200, 300, count))
    cd19 = np.where(~mono & (cd3 < 5_000), rng.normal(15_000, 4_000, count),
                    rng.normal(200, 300, count))
    viability = np.where(dead, rng.normal(15_000, 4_000, count), rng.normal(300, 250, count))
    return {"fsc": fsc, "ssc": ssc, "ratio": ratio, "cd3": cd3, "cd19": cd19, "ld": viability}


def _events(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    classes = np.zeros(EVENTS, dtype=np.int64)
    roll = rng.random(EVENTS)
    classes[roll < DEBRIS_FRACTION] = 1
    cells = roll >= DEBRIS_FRACTION
    classes[cells & (rng.random(EVENTS) < DOUBLET_FRACTION)] = 2
    classes[(classes == 0) & (rng.random(EVENTS) < DEAD_FRACTION)] = 3

    first = _cells(rng, EVENTS, classes == 3)
    second = _cells(rng, EVENTS, np.zeros(EVENTS, dtype=bool))
    fsc_a, ssc = first["fsc"], first["ssc"]
    fsc_h = fsc_a / first["ratio"]
    signal = np.column_stack([first["cd3"], first["cd19"], first["ld"]])

    doublet = classes == 2
    # Two cells in one pulse: areas add up, height grows only a little.
    fsc_a = np.where(doublet, fsc_a + second["fsc"], fsc_a)
    fsc_h = np.where(doublet, np.maximum(fsc_h, second["fsc"] / second["ratio"]) * 1.1, fsc_h)
    ssc = np.where(doublet, ssc + second["ssc"], ssc)
    signal[doublet] += np.column_stack([second["cd3"], second["cd19"], second["ld"]])[doublet]

    debris = classes == 1
    debris_fsc = rng.lognormal(np.log(9_000), 0.45, EVENTS)
    fsc_a = np.where(debris, debris_fsc, fsc_a)
    fsc_h = np.where(debris, debris_fsc / rng.normal(1.15, 0.15, EVENTS), fsc_h)
    ssc = np.where(debris, rng.lognormal(np.log(6_000), 0.6, EVENTS), ssc)
    signal[debris] = rng.normal(500, 1_500, (EVENTS, 3))[debris]

    observed = signal @ SPILL
    fsc_w = WIDTH_SCALE * fsc_a / np.maximum(fsc_h, 1.0)
    time = np.sort(rng.uniform(0, 600, EVENTS))
    data = np.column_stack([fsc_a, fsc_h, fsc_w, ssc, observed, time])
    return np.clip(data, -1_000, 262_143), classes


def _control(rng: np.random.Generator) -> np.ndarray:
    cells = _cells(rng, CONTROL_EVENTS, np.zeros(CONTROL_EVENTS, dtype=bool))
    fsc_h = cells["fsc"] / cells["ratio"]
    # Unstained for the viability dye: autofluorescence only.
    signal = np.column_stack([cells["cd3"], cells["cd19"], rng.normal(300, 250, CONTROL_EVENTS)])
    observed = signal @ SPILL
    data = np.column_stack([
        cells["fsc"], fsc_h, WIDTH_SCALE * cells["ratio"], cells["ssc"], observed,
        np.sort(rng.uniform(0, 600, CONTROL_EVENTS)),
    ])
    return np.clip(data, -1_000, 262_143)


def _spill_keyword(channels: list[str], matrix: np.ndarray) -> str:
    values = ",".join(f"{value:g}" for value in matrix.ravel())
    return f"{len(channels)},{','.join(channels)},{values}"


def main() -> None:
    out = Path(sys.argv[1])
    rng = np.random.default_rng(SEED)
    data, classes = _events(rng)
    fluor = ["FL1-A", "FL2-A", "FL3-A"]
    write_fcs(out / "sample.fcs", data, CHANNELS, LABELS,
              extra_keywords={"spillover": _spill_keyword(fluor, SPILL)})

    keep = [index for index, name in enumerate(CHANNELS) if name != "FL3-A"]
    write_fcs(out / "no_viability.fcs", data[:, keep], [CHANNELS[i] for i in keep],
              [LABELS[i] for i in keep],
              extra_keywords={"spillover": _spill_keyword(fluor[:2], SPILL[:2, :2])})

    write_fcs(out / "control.fcs", _control(rng), CHANNELS, LABELS,
              extra_keywords={"spillover": _spill_keyword(fluor, SPILL)})

    counts = {name: int((classes == code).sum())
              for code, name in enumerate(["live_singlet", "debris", "doublet", "dead_singlet"])}
    truth = {"events": EVENTS, "counts": counts, "classes": "".join(map(str, classes))}
    (out / "truth.json").write_text(json.dumps(truth, indent=2) + "\n")


if __name__ == "__main__":
    main()

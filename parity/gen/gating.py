"""GatingML 2.0 sample + strategy with FlowKit reference results for `cytiq gate`.

Usage: python gating.py <out_dir>

Writes `sample.fcs`, `gates.xml` and `flowkit_reference.json` (per-gate counts
and packed membership bits from `flowkit.GatingStrategy.gate_sample`). The
committed copy under `data/fixtures/gating/` is produced by this script.

The strategy covers every gate type and transformation that cytiq supports,
`FCS` and spectrum-matrix compensation, nested parents and Boolean gates.
Some events sit exactly on rectangle and polygon boundaries to pin down
edge semantics.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np

from fcsio import PNR, write_fcs

SEED = 20261023
EVENTS = 4000
CHANNELS = ["FSC-A", "SSC-A", "FL1-A", "FL2-A", "FL3-A", "Time"]
LABELS = ["", "", "CD3", "CD4", "CD8", ""]
SPILL = np.array([
    [1.00, 0.12, 0.03],
    [0.05, 1.00, 0.18],
    [0.01, 0.07, 1.00],
])
# GatingML spectrum matrix, deliberately different from $SPILL.
SPECTRUM = np.array([
    [1.00, 0.10, 0.00],
    [0.04, 1.00, 0.15],
    [0.00, 0.06, 1.00],
])
TIMESTEP = 0.01

GATES_XML = """<?xml version="1.0" encoding="UTF-8"?>
<gating:Gating-ML
    xmlns:gating="http://www.isac-net.org/std/Gating-ML/v2.0/gating"
    xmlns:transforms="http://www.isac-net.org/std/Gating-ML/v2.0/transformations"
    xmlns:data-type="http://www.isac-net.org/std/Gating-ML/v2.0/datatypes">

  <transforms:transformation transforms:id="lin">
    <transforms:flin transforms:T="262144" transforms:A="0"/>
  </transforms:transformation>
  <transforms:transformation transforms:id="log">
    <transforms:flog transforms:T="262144" transforms:M="4.5"/>
  </transforms:transformation>
  <transforms:transformation transforms:id="asinh">
    <transforms:fasinh transforms:T="262144" transforms:M="4.5" transforms:A="0.5"/>
  </transforms:transformation>
  <transforms:transformation transforms:id="logicle">
    <transforms:logicle transforms:T="262144" transforms:W="0.5" transforms:M="4.5" transforms:A="0"/>
  </transforms:transformation>
  <transforms:transformation transforms:id="hyperlog">
    <transforms:hyperlog transforms:T="262144" transforms:W="0.5" transforms:M="4.5" transforms:A="0"/>
  </transforms:transformation>

  <transforms:spectrumMatrix transforms:id="panel">
    <transforms:fluorochromes>
      <data-type:fcs-dimension data-type:name="CD3"/>
      <data-type:fcs-dimension data-type:name="CD4"/>
      <data-type:fcs-dimension data-type:name="CD8"/>
    </transforms:fluorochromes>
    <transforms:detectors>
      <data-type:fcs-dimension data-type:name="FL1-A"/>
      <data-type:fcs-dimension data-type:name="FL2-A"/>
      <data-type:fcs-dimension data-type:name="FL3-A"/>
    </transforms:detectors>
{spectrum}
  </transforms:spectrumMatrix>

  <!-- Integer vertices: some events sit exactly on edges and vertices. -->
  <gating:PolygonGate gating:id="Cells">
    <gating:dimension gating:compensation-ref="uncompensated">
      <data-type:fcs-dimension data-type:name="FSC-A"/>
    </gating:dimension>
    <gating:dimension gating:compensation-ref="uncompensated">
      <data-type:fcs-dimension data-type:name="SSC-A"/>
    </gating:dimension>
    <gating:vertex><gating:coordinate data-type:value="20000"/><gating:coordinate data-type:value="10000"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="200000"/><gating:coordinate data-type:value="10000"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="200000"/><gating:coordinate data-type:value="150000"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="110000"/><gating:coordinate data-type:value="60000"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="20000"/><gating:coordinate data-type:value="150000"/></gating:vertex>
  </gating:PolygonGate>

  <gating:RectangleGate gating:id="Singlets" gating:parent_id="Cells">
    <gating:dimension gating:min="40000" gating:max="180000" gating:compensation-ref="uncompensated">
      <data-type:fcs-dimension data-type:name="FSC-A"/>
    </gating:dimension>
    <gating:dimension gating:max="0.45" gating:compensation-ref="uncompensated" gating:transformation-ref="lin">
      <data-type:fcs-dimension data-type:name="SSC-A"/>
    </gating:dimension>
  </gating:RectangleGate>

  <gating:RectangleGate gating:id="Early" gating:parent_id="Cells">
    <gating:dimension gating:max="20" gating:compensation-ref="uncompensated">
      <data-type:fcs-dimension data-type:name="Time"/>
    </gating:dimension>
  </gating:RectangleGate>

  <gating:RectangleGate gating:id="CD3pos" gating:parent_id="Singlets">
    <gating:dimension gating:min="0.55" gating:compensation-ref="FCS" gating:transformation-ref="logicle">
      <data-type:fcs-dimension data-type:name="FL1-A"/>
    </gating:dimension>
  </gating:RectangleGate>

  <gating:RectangleGate gating:id="CD4pos_CD8neg" gating:parent_id="CD3pos">
    <gating:dimension gating:min="0.5" gating:compensation-ref="FCS" gating:transformation-ref="asinh">
      <data-type:fcs-dimension data-type:name="FL2-A"/>
    </gating:dimension>
    <gating:dimension gating:max="0.5" gating:compensation-ref="FCS" gating:transformation-ref="asinh">
      <data-type:fcs-dimension data-type:name="FL3-A"/>
    </gating:dimension>
  </gating:RectangleGate>

  <gating:EllipsoidGate gating:id="CD4_CD8_ellipse" gating:parent_id="CD3pos">
    <gating:dimension gating:compensation-ref="panel" gating:transformation-ref="hyperlog">
      <data-type:fcs-dimension data-type:name="CD4"/>
    </gating:dimension>
    <gating:dimension gating:compensation-ref="panel" gating:transformation-ref="hyperlog">
      <data-type:fcs-dimension data-type:name="CD8"/>
    </gating:dimension>
    <gating:mean><gating:coordinate data-type:value="0.75"/><gating:coordinate data-type:value="0.35"/></gating:mean>
    <gating:covarianceMatrix>
      <gating:row><gating:entry data-type:value="0.010"/><gating:entry data-type:value="0.002"/></gating:row>
      <gating:row><gating:entry data-type:value="0.002"/><gating:entry data-type:value="0.012"/></gating:row>
    </gating:covarianceMatrix>
    <gating:distanceSquare data-type:value="3"/>
  </gating:EllipsoidGate>

  <!-- Self-intersecting (bow-tie) polygon on compensated, logicle-scaled data. -->
  <gating:PolygonGate gating:id="Bowtie" gating:parent_id="Singlets">
    <gating:dimension gating:compensation-ref="panel" gating:transformation-ref="logicle">
      <data-type:fcs-dimension data-type:name="FL2-A"/>
    </gating:dimension>
    <gating:dimension gating:compensation-ref="panel" gating:transformation-ref="logicle">
      <data-type:fcs-dimension data-type:name="FL3-A"/>
    </gating:dimension>
    <gating:vertex><gating:coordinate data-type:value="0.2"/><gating:coordinate data-type:value="0.2"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="0.9"/><gating:coordinate data-type:value="0.9"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="0.9"/><gating:coordinate data-type:value="0.2"/></gating:vertex>
    <gating:vertex><gating:coordinate data-type:value="0.2"/><gating:coordinate data-type:value="0.9"/></gating:vertex>
  </gating:PolygonGate>

  <gating:RectangleGate gating:id="FL3_log" gating:parent_id="Cells">
    <gating:dimension gating:min="0.6" gating:compensation-ref="uncompensated" gating:transformation-ref="log">
      <data-type:fcs-dimension data-type:name="FL3-A"/>
    </gating:dimension>
  </gating:RectangleGate>

  <gating:BooleanGate gating:id="CD4_not_ellipse" gating:parent_id="CD3pos">
    <gating:and>
      <gating:gateReference gating:ref="CD4pos_CD8neg"/>
      <gating:gateReference gating:ref="CD4_CD8_ellipse" gating:use-as-complement="true"/>
    </gating:and>
  </gating:BooleanGate>

  <gating:BooleanGate gating:id="Ellipse_or_log">
    <gating:or>
      <gating:gateReference gating:ref="CD4_CD8_ellipse"/>
      <gating:gateReference gating:ref="FL3_log"/>
    </gating:or>
  </gating:BooleanGate>

  <gating:BooleanGate gating:id="CD3neg" gating:parent_id="Singlets">
    <gating:not>
      <gating:gateReference gating:ref="CD3pos"/>
    </gating:not>
  </gating:BooleanGate>
</gating:Gating-ML>
"""


def _spectrum_xml(matrix: np.ndarray) -> str:
    rows = []
    for row in matrix:
        coefficients = "".join(
            f'<transforms:coefficient transforms:value="{value:g}"/>' for value in row
        )
        rows.append(f"    <transforms:spectrum>{coefficients}</transforms:spectrum>")
    return "\n".join(rows)


def _events(rng: np.random.Generator) -> np.ndarray:
    n = EVENTS
    fsc = rng.normal(100_000, 40_000, n)
    ssc = 0.5 * fsc + rng.normal(0, 25_000, n)
    positive = rng.random((n, 3)) < np.array([0.6, 0.5, 0.3])
    signal = np.where(positive, rng.lognormal(9.5, 0.6, (n, 3)), rng.normal(0, 300, (n, 3)))
    observed = signal @ SPILL  # detectors see true signal · S
    time = np.sort(rng.uniform(0, 4000, n))
    data = np.column_stack([fsc, ssc, observed, time])

    # Exact boundary hits for the untransformed gates.
    edges = np.array([
        [20000, 50000], [200000, 50000], [110000, 10000], [50000, 150000],
        [20000, 10000], [200000, 150000], [110000, 60000], [40000, 20000],
        [180000, 20000], [65000, 105000], [155000, 105000],
    ], dtype=float)
    data[: len(edges), :2] = edges
    data[:, :5] = np.clip(data[:, :5], -5000, PNR - 1)
    return data.astype(np.float32).astype(np.float64)


def write_dataset(out: Path) -> tuple[Path, Path]:
    rng = np.random.default_rng(SEED)
    out.mkdir(parents=True, exist_ok=True)
    spill = ",".join(f"{value:g}" for value in SPILL.ravel())
    fcs = out / "sample.fcs"
    write_fcs(
        fcs,
        _events(rng),
        CHANNELS,
        labels=LABELS,
        extra_keywords={"spillover": f"3,FL1-A,FL2-A,FL3-A,{spill}", "timestep": str(TIMESTEP)},
    )
    xml = out / "gates.xml"
    xml.write_text(GATES_XML.replace("{spectrum}", _spectrum_xml(SPECTRUM)))
    return fcs, xml


def flowkit_reference(fcs: Path, xml: Path) -> dict:
    import flowkit as fk

    sample = fk.Sample(str(fcs))
    strategy = fk.parse_gating_xml(str(xml))
    results = strategy.gate_sample(sample)
    gates = {}
    for gate_id, gate_path in strategy.get_gate_ids():
        membership = np.asarray(results.get_gate_membership(gate_id, gate_path), dtype=bool)
        gates[gate_id] = {
            "gate_path": "/".join(list(gate_path[1:]) + [gate_id]),
            "count": int(results.get_gate_count(gate_id, gate_path)),
            "membership_hex": np.packbits(membership).tobytes().hex(),
        }
    return {"event_count": int(sample.event_count), "gates": gates}


def gen_gating(out: Path) -> None:
    fcs, xml = write_dataset(out)
    reference = flowkit_reference(fcs, xml)
    (out / "flowkit_reference.json").write_text(json.dumps(reference, indent=2) + "\n")


if __name__ == "__main__":
    gen_gating(Path(sys.argv[1]))

"""Compare cytiq with upstream R tools on real acquisitions.

Usage: python compare_real.py <out_dir> <metrics_json>

Exits with 1 when a rule in parity/real/tolerances.json is violated.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import flowio
import numpy as np
import pandas as pd
import pyarrow.parquet as pq
from sklearn.metrics import adjusted_rand_score

PARITY = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PARITY))
from compare import _offdiag, check_tolerances  # noqa: E402

# autospill's `marker.forbidden.char`, each replaced by `-` in its matrix and
# gate names.
AUTOSPILL_FORBIDDEN = " !\"#$%&'()*,/:;?@[\\]^{|}~"


def _r(value: float, digits: int = 6) -> float:
    return float(f"{value:.{digits}g}")


def read_columns(path: Path, channels: list[str]) -> tuple[np.ndarray, dict]:
    flow = flowio.FlowData(str(path), ignore_offset_discrepancy=True)
    data = np.reshape(np.asarray(flow.events, dtype=np.float64), (-1, flow.channel_count))
    names = [flow.text[f"p{i + 1}n"] for i in range(flow.channel_count)]
    ranges = {name: float(flow.text[f"p{i + 1}r"]) for i, name in enumerate(names)}
    return data[:, [names.index(channel) for channel in channels]], ranges


def max_rel_delta(rust_fcs: Path, r_csv: Path, channels: list[str]) -> float:
    mine, _ = read_columns(rust_fcs, channels)
    theirs = pd.read_csv(r_csv).to_numpy()
    return _r(float(np.max(np.abs(mine - theirs) / np.maximum(np.abs(theirs), 1.0))))


def flowkit_membership(fcs: Path, xml: Path) -> dict[str, np.ndarray]:
    import flowkit as fk

    sample = fk.Sample(str(fcs))
    strategy = fk.parse_gating_xml(str(xml))
    results = strategy.gate_sample(sample)
    return {gate_id: np.asarray(results.get_gate_membership(gate_id, gate_path), dtype=bool)
            for gate_id, gate_path in strategy.get_gate_ids()}


def compare_flowsom_68983(directory: Path) -> dict:
    markers = (directory / "markers.txt").read_text().strip().split(",")
    out = {
        "compensated_max_rel_delta": max_rel_delta(directory / "sample.comp.fcs",
                                                   directory / "r_compensated.csv", markers),
        "transformed_max_rel_delta": max_rel_delta(directory / "sample.t.fcs",
                                                   directory / "r_transformed.csv", markers),
    }

    membership = pq.read_table(directory / "membership.parquet").to_pandas()
    membership = membership.set_index("source_event_id").sort_index()
    flowkit = flowkit_membership(directory / "sample.comp.fcs", directory / "gates.xml")
    out["gates"] = {
        gate_id: {"count": int(membership[gate_id].fillna(False).sum()),
                  "flowkit_disagreement": int((membership[gate_id].fillna(False).to_numpy().astype(bool)
                                               != theirs).sum())}
        for gate_id, theirs in flowkit.items()
    }

    # FlowJo's labels come from the workspace; each population is matched to
    # the cytiq gate it overlaps best.
    labels = pd.read_csv(directory / "flowjo_labels.csv", header=None)[0].to_numpy()
    leaves = {gate_id: membership[gate_id].fillna(False).to_numpy().astype(bool)
              for gate_id in flowkit}
    out["flowjo_populations"] = {}
    for label in sorted(set(labels) - {"Unlabeled"}):
        truth = labels == label
        jaccard, gate_id = max(((members & truth).sum() / (members | truth).sum(), gate_id)
                               for gate_id, members in leaves.items())
        out["flowjo_populations"][label] = {"gate": gate_id, "jaccard": _r(jaccard, 4),
                                            "flowjo_events": int(truth.sum())}

    rust = pd.read_csv(directory / "flowsom.csv").sort_values("event_index")["flowsom_metacluster"].to_numpy()
    seeds = [pd.read_csv(directory / f"r_flowsom_seed{seed}.csv")["metacluster"].to_numpy() for seed in (1, 2)]
    labelled = labels != "Unlabeled"
    out["flowsom"] = {
        "rust_vs_upstream_ari": _r(adjusted_rand_score(rust, seeds[0]), 4),
        "upstream_seed_to_seed_ari": _r(adjusted_rand_score(seeds[0], seeds[1]), 4),
        "rust_ari_vs_flowjo": _r(adjusted_rand_score(labels[labelled], rust[labelled]), 4),
        "upstream_ari_vs_flowjo": _r(adjusted_rand_score(labels[labelled], seeds[0][labelled]), 4),
    }
    out["flowsom"]["rust_vs_upstream_minus_seed_band"] = _r(
        out["flowsom"]["rust_vs_upstream_ari"] - out["flowsom"]["upstream_seed_to_seed_ari"], 4)
    return out


def compare_qc_chain(directory: Path) -> dict:
    channels = (directory / "channels.txt").read_text().strip().split(",")
    raw, ranges = read_columns(directory / "sample.fcs", channels)
    upper = np.array([ranges[channel] - 1 for channel in channels])
    # The detector limit recomputed independently: `$PnR - 1` only, no low bound.
    at_limit = (raw >= upper).any(axis=1)
    comp = pq.read_table(directory / "sample.comp.masks.parquet").to_pandas().sort_values("source_event_id")
    margin_removed = ~comp["margin_pass"].astype(bool).to_numpy()
    r_margins = pd.read_csv(directory / "r_margins.csv")["removed"].to_numpy().astype(bool)
    peacoqc = pq.read_table(directory / "sample.peacoqc.masks.parquet").to_pandas().sort_values("source_event_id")
    r_peacoqc = pd.read_csv(directory / "r_peacoqc.csv")["good"].to_numpy().astype(bool)
    return {
        "events": int(len(raw)),
        "channels": len(channels),
        "compensated_max_rel_delta": max_rel_delta(directory / "sample.comp.fcs",
                                                   directory / "r_compensated.csv", channels),
        "transformed_max_rel_delta": max_rel_delta(directory / "sample.t.fcs",
                                                   directory / "r_transformed.csv", channels),
        "margins": {
            "agreement_with_detector_limits": _r((margin_removed == at_limit).mean(), 6),
            "removed": int(margin_removed.sum()),
            "RemoveMargins_removed": int(r_margins.sum()),
            "agreement_with_RemoveMargins": _r((margin_removed == r_margins).mean(), 4),
            "kept_by_RemoveMargins": int((margin_removed & ~r_margins).sum()),
        },
        "peacoqc": {
            "agreement": _r((peacoqc["keep"].astype(bool).to_numpy() == r_peacoqc).mean(), 6),
            "removed_fraction": _r((~r_peacoqc).mean(), 4),
        },
    }


def autospill_name(channel: str) -> str:
    return "".join("-" if char in AUTOSPILL_FORBIDDEN else char for char in channel)


def compare_autospill_chain(directory: Path) -> dict:
    mine = pd.read_csv(directory / "autospill_spillover.csv", index_col=0)
    mine.index = mine.columns = [autospill_name(name) for name in mine.columns]
    theirs = pd.read_csv(directory / "r_autospill_spillover.csv", index_col=0)
    order = list(theirs.index)
    a, b = mine.loc[order, order].to_numpy(), theirs.loc[order, order].to_numpy()
    summary = json.loads((directory / "autospill_debug" / "summary.json").read_text())
    rust_gated = {autospill_name(g["dye"]): g["gated_events"] for g in summary["gate_stats"]}
    r_gated = pd.read_csv(directory / "r_autospill_gates.csv")["sample"].value_counts()
    return {
        "controls": len(order),
        "rust_vs_upstream": {
            "max_abs": _r(float(np.max(np.abs(_offdiag(a - b))))),
            "compensation_frobenius": _r(float(np.linalg.norm(np.linalg.inv(a) - np.linalg.inv(b)))),
        },
        "max_offdiag_spillover": _r(float(np.max(_offdiag(b)))),
        "gates": {dye: {"rust": int(rust_gated[dye]), "upstream": int(r_gated.get(dye, 0)),
                        "delta": int(rust_gated[dye] - r_gated.get(dye, 0))} for dye in order},
    }


def main() -> None:
    out_dir, target = Path(sys.argv[1]), Path(sys.argv[2])
    metrics: dict = {"real": {}}
    if (out_dir / "flowsom_68983").exists():
        metrics["real"]["flowsom_68983"] = compare_flowsom_68983(out_dir / "flowsom_68983")
    for directory in sorted(out_dir.iterdir()):
        if (directory / "channels.txt").exists():
            metrics["real"][directory.name] = compare_qc_chain(directory)
        if (directory / "controls_full.csv").exists():
            metrics["real"][directory.name] = {"autospill": compare_autospill_chain(directory)}
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(metrics, indent=2) + "\n")
    print(json.dumps(metrics, indent=2))
    violations = check_tolerances(metrics, Path(__file__).parent / "tolerances.json")
    for violation in violations:
        print(f"TOLERANCE VIOLATION: {violation}", file=sys.stderr)
    if violations:
        sys.exit(1)
    print("all tolerances hold", file=sys.stderr)


if __name__ == "__main__":
    main()

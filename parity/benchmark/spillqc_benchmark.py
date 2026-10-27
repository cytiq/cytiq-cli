"""Benchmark `cytiq spillqc` on real acquisitions.

Usage: python spillqc_benchmark.py <real_out_dir> <metrics_json>

Reads the outputs of `parity/real/run_real.sh` (`just parity-real`; set
CYTIQ_REAL_SLOW=1 for the FACSymphony controls) and runs `cytiq` from CYTIQ_BIN.

Stage 1, matrix accuracy: the spillover matrix FACSDiva embedded in the
stained sample against the one `cytiq autospill` computes from the matching
single-stain controls (R AutoSpill agrees to 5e-13, parity/REPORT.md section
2.10), and `cytiq spillqc` on the sample with each matrix: the flags and how
many they share. The controls themselves carry an identity `$SPILL` (FACSDiva
writes it before compensation is calculated), so a control set without a
stained sample has no lab matrix to compare; it is reported as such.

Stage 2, detection: one off-diagonal coefficient of the stained sample's
FACSDiva matrix is changed at a time, relatively (+-5 to 50%, coefficients of
at least 0.01) or absolutely (+-0.01 to 0.05, never below 0), and spillqc runs
on the unchanged events. A larger coefficient over-compensates, so the pair
(primary = the fluorochrome's channel, secondary = the detector) should turn
`over`; a smaller one should turn `under`. The real sample is flagged before any
change, so every run is scored against the unchanged-matrix run:

- detected: the pair carries the expected flag and did not before;
- masked: the pair carried the expected flag before (cannot be newly found);
- not_assessed: the pair has no SSI in the run;
- shift: the pair's SSI change toward the expected flag (positive = toward);
- new_flags_elsewhere: other pairs flagged in the run but not before.

Precision counts the detected pairs among every newly flagged pair; recall
counts them among all changed coefficients.
"""

from __future__ import annotations

import csv
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import flowio
import numpy as np

RELATIVE = [0.05, 0.10, 0.20, 0.30, 0.50]
ABSOLUTE = [0.01, 0.02, 0.05]
RELATIVE_FLOOR = 0.01
STAINED_SAMPLE = "celesta_mdsc_sample"
CONTROL_SETS = {"celesta_mdsc_controls": STAINED_SAMPLE, "symphony_omip24_controls": None}
WORKERS = max(1, (os.cpu_count() or 2) // 2)


def _r(value: float, digits: int = 4) -> float:
    return float(f"{value:.{digits}g}")


def embedded_spillover(path: Path) -> tuple[list[str], np.ndarray]:
    text = flowio.FlowData(str(path), ignore_offset_discrepancy=True).text
    value = next(text[key] for key in ("spill", "spillover", "$spill", "$spillover") if key in text)
    parts = value.split(",")
    n = int(parts[0])
    return parts[1:n + 1], np.array(list(map(float, parts[n + 1:]))).reshape(n, n)


def read_matrix(path: Path, order: list[str]) -> np.ndarray:
    with path.open() as handle:
        rows = list(csv.reader(handle))
    header = rows[0][1:]
    values = {row[0]: dict(zip(header, map(float, row[1:]))) for row in rows[1:]}
    return np.array([[values[r][c] for c in order] for r in order])


def write_matrix(path: Path, channels: list[str], matrix: np.ndarray) -> None:
    with path.open("w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["channel", *channels])
        for name, row in zip(channels, matrix):
            writer.writerow([name, *(repr(float(v)) for v in row)])


def matrix_accuracy(fcs: np.ndarray, autospill: np.ndarray, channels: list[str]) -> dict:
    off = ~np.eye(len(channels), dtype=bool)
    delta = autospill - fcs
    order = np.argsort(-np.abs(np.where(off, delta, 0)), axis=None)[:5]
    return {
        "colours": len(channels),
        "offdiag_mae": _r(np.mean(np.abs(delta[off]))),
        "offdiag_max_abs": _r(np.max(np.abs(delta[off]))),
        "offdiag_frobenius": _r(np.linalg.norm(delta[off])),
        "coefficients_off_by_0.01": int(np.sum(np.abs(delta[off]) >= 0.01)),
        "largest": [{"fluorochrome": channels[i // len(channels)], "detector": channels[i % len(channels)],
                     "facsdiva": _r(fcs.flat[i]), "autospill": _r(autospill.flat[i])} for i in order],
    }


def run_spillqc(binary: str, sample: Path, matrix: Path, out: Path) -> dict[tuple[str, str], float | None]:
    """SSI of every assessed pair, keyed (primary, secondary); None when not assessed."""
    subprocess.run([binary, "spillqc", "--input", str(sample), "--output", str(out), "--matrix", str(matrix),
                    "--matrix-kind", "spillover", "--threads", "1", "--overwrite"],
                   check=True, capture_output=True, env={**os.environ, "CYTIQ_LOG": "error"})
    with (out / sample.stem / "ssi_matrix.csv").open() as handle:
        rows = list(csv.reader(handle))
    primaries = rows[0][1:]
    return {(primary, row[0]): (float(cell) if cell else None)
            for row in rows[1:] for primary, cell in zip(primaries, row[1:]) if primary != row[0]}


def status(ssi: float | None) -> str:
    if ssi is None:
        return "not_assessed"
    return "over" if ssi <= -1 else "under" if ssi >= 1 else "pass"


def flags(ssis: dict) -> dict:
    return {pair: status(ssi) for pair, ssi in ssis.items() if status(ssi) in ("over", "under")}


def perturbations(channels: list[str], matrix: np.ndarray):
    n = len(channels)
    for i in range(n):
        for j in range(n):
            if i == j:
                continue
            value = matrix[i, j]
            for level in RELATIVE:
                if value >= RELATIVE_FLOOR:
                    for sign in (1, -1):
                        yield ("relative", sign * level, i, j, value * (1 + sign * level))
            for level in ABSOLUTE:
                for sign in (1, -1):
                    if value + sign * level >= 0:
                        yield ("absolute", sign * level, i, j, value + sign * level)


def detection(binary: str, sample: Path, work: Path) -> dict:
    work.mkdir(parents=True, exist_ok=True)
    channels, base = embedded_spillover(sample)
    base_file = work / "base.csv"
    write_matrix(base_file, channels, base)
    baseline = run_spillqc(binary, sample, base_file, work / "base")
    baseline_flags = flags(baseline)

    def one(index_and_case):
        index, (mode, level, i, j, value) = index_and_case
        matrix = base.copy()
        matrix[i, j] = value
        path = work / f"m{index}.csv"
        write_matrix(path, channels, matrix)
        ssis = run_spillqc(binary, sample, path, work / f"r{index}")
        pair = (channels[i], channels[j])
        expected = "over" if level > 0 else "under"
        before, after = baseline.get(pair), ssis.get(pair)
        new = {p: s for p, s in flags(ssis).items() if baseline_flags.get(p) != s}
        return {
            "mode": mode, "level": level, "pair": pair, "expected": expected,
            "detected": status(after) == expected and baseline_flags.get(pair) != expected,
            "masked": baseline_flags.get(pair) == expected,
            "not_assessed": after is None,
            "shift": None if before is None or after is None else (after - before) * (1 if expected == "under" else -1),
            "new_flags_elsewhere": sum(1 for p in new if p != pair),
        }

    cases = list(enumerate(perturbations(channels, base)))
    with ThreadPoolExecutor(WORKERS) as pool:
        results = list(pool.map(one, cases))

    levels = {}
    for result in results:
        levels.setdefault((result["mode"], result["level"]), []).append(result)
    table = []
    for (mode, level), rows in sorted(levels.items(), key=lambda item: (item[0][0], item[0][1])):
        detected = sum(r["detected"] for r in rows)
        elsewhere = sum(r["new_flags_elsewhere"] for r in rows)
        shifts = [r["shift"] for r in rows if r["shift"] is not None]
        recall = detected / len(rows)
        precision = detected / (detected + elsewhere) if detected + elsewhere else 0.0
        table.append({
            "mode": mode, "level": level, "coefficients": len(rows), "detected": detected,
            "recall": _r(recall), "precision": _r(precision),
            "f1": _r(2 * precision * recall / (precision + recall)) if precision + recall else 0.0,
            "masked": sum(r["masked"] for r in rows),
            "not_assessed": sum(r["not_assessed"] for r in rows),
            "median_shift_toward_flag": _r(float(np.median(shifts))) if shifts else None,
            "new_flags_elsewhere_per_run": _r(elsewhere / len(rows)),
        })
    detected_coefficients = sorted({(r["pair"], r["expected"]) for r in results if r["detected"]})
    return {
        "sample": sample.parent.name,
        "baseline": {"pairs": len(baseline), "assessed": sum(v is not None for v in baseline.values()),
                     "flags": len(baseline_flags)},
        "runs": len(results),
        "levels": table,
        "ever_detected": [{"primary": p, "secondary": s, "direction": d} for (p, s), d in detected_coefficients],
    }


def main() -> None:
    real, target = Path(sys.argv[1]), Path(sys.argv[2])
    binary = os.environ.get("CYTIQ_BIN", str(Path(__file__).resolve().parents[2] / "target/release/cytiq"))
    metrics: dict = {"matrix_accuracy": {}, "detection": {}}
    with tempfile.TemporaryDirectory(prefix="spillqc-benchmark-") as scratch:
        work = Path(scratch)
        for controls, stained in CONTROL_SETS.items():
            directory = real / controls
            if not (directory / "autospill_spillover.csv").exists():
                print(f"skipping {controls}: no `just parity-real` output", file=sys.stderr)
                continue
            sample = real / stained / "sample.fcs" if stained else None
            if sample is None or not sample.exists():
                first = sorted(directory.glob("ctrl_*.fcs"))[0]
                channels, embedded = embedded_spillover(first)
                identity = bool(np.allclose(embedded, np.eye(len(channels))))
                metrics["matrix_accuracy"][controls] = {
                    "colours": len(channels),
                    "skipped": "no stained sample; the controls' $SPILL is "
                               + ("the identity" if identity else "the only matrix"),
                }
                continue
            channels, fcs = embedded_spillover(sample)
            autospill = read_matrix(directory / "autospill_spillover.csv", channels)
            entry = matrix_accuracy(fcs, autospill, channels)
            runs = {}
            for name, matrix in (("facsdiva", fcs), ("autospill", autospill)):
                path = work / f"{controls}_{name}.csv"
                write_matrix(path, channels, matrix)
                runs[name] = flags(run_spillqc(binary, sample, path, work / f"{controls}_{name}"))
            shared = {p for p, s in runs["facsdiva"].items() if runs["autospill"].get(p) == s}
            entry["stained_sample_flags"] = {
                "facsdiva": len(runs["facsdiva"]), "autospill": len(runs["autospill"]),
                "shared": len(shared),
            }
            metrics["matrix_accuracy"][controls] = entry
        sample = real / STAINED_SAMPLE / "sample.fcs"
        if sample.exists():
            metrics["detection"][STAINED_SAMPLE] = detection(binary, sample, work / "detection")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(metrics, indent=2) + "\n")
    print(json.dumps(metrics, indent=2))


if __name__ == "__main__":
    main()

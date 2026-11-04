"""Validate cargo-crap reports and render a summary; scoring stays in cargo-crap."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import html
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys


CRAP_VERSION = "0.9.0"
THRESHOLD = 30
STATUSES = {"regressed", "improved", "new", "unchanged", "moved"}


def read_json(path):
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def number(value, minimum=0, maximum=math.inf):
    return (
        type(value) in (int, float)
        and math.isfinite(value)
        and minimum <= value <= maximum
    )


def read_report(path, delta=False):
    report = read_json(path)
    if not isinstance(report, dict) or report.get("version") != CRAP_VERSION:
        raise ValueError(f"{path}: expected a cargo-crap {CRAP_VERSION} envelope")
    entries = report.get("entries")
    if not isinstance(entries, list) or not entries:
        raise ValueError(f"{path}: workspace report has no functions")
    diagnostics = report.get("diagnostics", {})
    if not isinstance(diagnostics, dict) or not number(diagnostics.get("matched_files"), 1):
        raise ValueError(f"{path}: no source files matched LCOV coverage")
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError(f"{path}: invalid function entry")
        if not all(isinstance(entry.get(key), str) and entry[key] for key in ("file", "function", "crate")):
            raise ValueError(f"{path}: function identity or crate is missing")
        if type(entry.get("line")) is not int or entry["line"] < 1:
            raise ValueError(f"{path}: invalid function line")
        if not number(entry.get("cyclomatic"), 1) or not number(entry.get("crap"), 1):
            raise ValueError(f"{path}: invalid complexity or CRAP score")
        if "coverage" not in entry or (
            entry["coverage"] is not None and not number(entry["coverage"], 0, 100)
        ):
            raise ValueError(f"{path}: invalid coverage percentage")
        if delta and entry.get("status") not in STATUSES:
            raise ValueError(f"{path}: invalid delta status")
    return report


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def record_run(directory):
    lcov = directory / "lcov.info"
    if not lcov.is_file() or lcov.stat().st_size == 0:
        raise ValueError("Coverage export is missing or empty")
    record = {
        "tests_passed": True,
        "collected_at": datetime.now(timezone.utc).isoformat(),
        "commit": command("git", "rev-parse", "HEAD"),
        "dirty": bool(command("git", "status", "--porcelain")),
        "platform": platform.system(),
        "architecture": platform.machine(),
        "rustc": command("rustc", "-vV"),
        "cargo_llvm_cov": command("cargo", "llvm-cov", "--version"),
        "config_sha256": hashlib.sha256(Path(".cargo-crap.toml").read_bytes()).hexdigest(),
        "lcov_sha256": hashlib.sha256(lcov.read_bytes()).hexdigest(),
        "workflow_run": os.environ.get("GITHUB_RUN_ID"),
        "workflow_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }
    (directory / "coverage-run.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")


def cell(value):
    return html.escape(str(value), quote=False).replace("|", "&#124;").replace("\n", " ").replace("`", "&#96;")


def entry_identity(row):
    return tuple(row[key] for key in ("file", "function", "line", "crate", "cyclomatic", "coverage", "crap"))


def render_summary(report, delta=None):
    entries = report["entries"]
    ranked = sorted(entries, key=lambda row: (-row["crap"], row["file"], row["function"], row["line"]))
    counts = Counter(row["status"] for row in delta["entries"]) if delta else Counter()
    missing = sum(row["coverage"] is None for row in entries)
    lines = [
        "## CRAP report",
        "",
        "Stage one: scores and regressions are informational. Coverage is line coverage; "
        "fully covered functions still have CRAP equal to their complexity.",
        "",
        f"Functions: **{len(entries)}** · CRAP > {THRESHOLD}: "
        f"**{sum(row['crap'] > THRESHOLD for row in entries)}** · "
        f"Maximum CRAP: **{ranked[0]['crap']:.2f}** · Missing coverage: **{missing}**",
        "",
    ]
    if delta:
        lines.extend([
            f"Regressed: **{counts['regressed']}** · Improved: **{counts['improved']}** · "
            f"New: **{counts['new']}** · Moved: **{counts['moved']}** · "
            f"Removed: **{len(delta['removed'])}** · Unchanged: **{counts['unchanged']}**",
            "",
        ])
    else:
        lines.extend(["Baseline candidate: no comparison was requested.", ""])
    lines.extend([
        "| Crate | Functions | CRAP > 30 | Maximum CRAP | Regressed | Missing coverage |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ])
    regressions = Counter(row["crate"] for row in delta["entries"] if row["status"] == "regressed") if delta else Counter()
    for crate in sorted({row["crate"] for row in entries}):
        rows = [row for row in entries if row["crate"] == crate]
        regressed = str(regressions[crate]) if delta else "—"
        lines.append(
            f"| {cell(crate)} | {len(rows)} | {sum(row['crap'] > THRESHOLD for row in rows)} | "
            f"{max(row['crap'] for row in rows):.2f} | {regressed} | "
            f"{sum(row['coverage'] is None for row in rows)} |"
        )
    lines.extend([
        "", "### Top 30 functions", "",
        "| Function | Location | CC | Line coverage | CRAP |",
        "| --- | --- | ---: | ---: | ---: |",
    ])
    for row in ranked[:30]:
        coverage = "missing (scored as 0%)" if row["coverage"] is None else f"{row['coverage']:.1f}%"
        lines.append(
            f"| {cell(row['function'])} | {cell(row['file'])}:{row['line']} | "
            f"{row['cyclomatic']:g} | {coverage} | {row['crap']:.2f} |"
        )
    return "\n".join(lines) + "\n"


def write_summary(directory, baseline=None):
    run = read_json(directory / "coverage-run.json")
    if not isinstance(run, dict) or run.get("tests_passed") is not True:
        raise ValueError("No successful coverage test run was recorded")
    digest = hashlib.sha256((directory / "lcov.info").read_bytes()).hexdigest()
    if run.get("lcov_sha256") != digest:
        raise ValueError("LCOV changed since the successful coverage run; rerun just coverage")
    if run.get("config_sha256") != hashlib.sha256(Path(".cargo-crap.toml").read_bytes()).hexdigest():
        raise ValueError("CRAP configuration changed; rerun just coverage")
    report = read_report(directory / "crap.json")
    delta = None
    if baseline:
        read_report(baseline)
        delta = read_report(directory / "crap-delta.json", delta=True)
        if Counter(map(entry_identity, report["entries"])) != Counter(map(entry_identity, delta["entries"])):
            raise ValueError("Absolute and delta reports cover different functions")
        if not isinstance(delta.get("removed"), list):
            raise ValueError("Delta report is missing the removed-functions list")
    (directory / "summary.md").write_text(render_summary(report, delta), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("record", "summary"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "record":
            record_run(args.directory)
        else:
            write_summary(args.directory, args.baseline)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"CRAP report error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

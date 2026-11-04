import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import crap_report


def report_fixture():
    return {
        "version": "0.9.0",
        "diagnostics": {"matched_files": 2},
        "entries": [
            {"file": "crates/core/src/lib.rs", "function": "covered", "crate": "core",
             "line": 1, "cyclomatic": 30, "coverage": 100, "crap": 30},
            {"file": "crates/core/src/lib.rs", "function": "risky", "crate": "core",
             "line": 5, "cyclomatic": 12, "coverage": None, "crap": 156},
        ],
    }


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)

    def write(self, name, value):
        path = self.directory / name
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def prepare_run(self):
        lcov = b"SF:crates/core/src/lib.rs\nDA:1,1\nend_of_record\n"
        (self.directory / "lcov.info").write_bytes(lcov)
        self.write("coverage-run.json", {
            "tests_passed": True,
            "lcov_sha256": hashlib.sha256(lcov).hexdigest(),
            "config_sha256": hashlib.sha256(Path(".cargo-crap.toml").read_bytes()).hexdigest(),
        })
        self.write("crap.json", report_fixture())

    def test_missing_coverage_is_visible_and_threshold_is_strictly_above_30(self):
        report = crap_report.read_report(self.write("report.json", report_fixture()))
        summary = crap_report.render_summary(report)
        self.assertIn("CRAP > 30: **1**", summary)
        self.assertIn("Maximum CRAP: **156.00**", summary)
        self.assertIn("missing (scored as 0%)", summary)
        self.assertLess(summary.index("| risky |"), summary.index("| covered |"))

    def test_incomplete_or_invalid_reports_are_rejected(self):
        variants = [
            {"version": "0.9.0", "entries": []},
            dict(report_fixture(), version="0.4.2"),
            dict(report_fixture(), diagnostics={"matched_files": 0}),
            dict(report_fixture(), diagnostics=None),
        ]
        for field, value in (("coverage", 101), ("crap", float("nan")),
                             ("cyclomatic", True), ("line", 0), ("crate", "")):
            invalid = report_fixture()
            invalid["entries"][0][field] = value
            variants.append(invalid)
        for invalid in variants:
            with self.subTest(report=invalid), self.assertRaises(ValueError):
                crap_report.read_report(self.write("invalid.json", invalid))

    def test_summary_limits_rows_without_truncating_totals(self):
        report = report_fixture()
        for index in range(40):
            row = dict(report["entries"][0], function=f"function_{index}", crap=1000 + index)
            report["entries"].append(row)
        summary = crap_report.render_summary(report)
        self.assertIn("Functions: **42**", summary)
        self.assertIn("| function_39 |", summary)
        self.assertNotIn("| function_0 |", summary)
        self.assertEqual(sum(line.startswith("| function_") for line in summary.splitlines()), 30)

    def test_regressions_and_high_scores_do_not_fail_report_generation(self):
        self.prepare_run()
        delta = report_fixture()
        for row in delta["entries"]:
            row.update(status="regressed", baseline_crap=1, delta=row["crap"] - 1)
        delta["removed"] = []
        self.write("crap-delta.json", delta)
        baseline = self.write("baseline.json", report_fixture())
        crap_report.write_summary(self.directory, baseline)
        self.assertIn("Regressed: **2**", (self.directory / "summary.md").read_text())

    def test_empty_baseline_is_rejected(self):
        self.prepare_run()
        baseline = self.write("baseline.json", {"version": "0.9.0", "entries": []})
        with self.assertRaisesRegex(ValueError, "no functions"):
            crap_report.write_summary(self.directory, baseline)

    def test_lcov_changed_after_tests_is_rejected(self):
        self.prepare_run()
        (self.directory / "lcov.info").write_text("changed", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "LCOV changed"):
            crap_report.write_summary(self.directory)

    def test_failed_test_marker_is_rejected(self):
        self.prepare_run()
        self.write("coverage-run.json", {"tests_passed": False})
        with self.assertRaisesRegex(ValueError, "successful coverage"):
            crap_report.write_summary(self.directory)

    def test_truncated_delta_is_rejected(self):
        self.prepare_run()
        delta = report_fixture()
        delta["entries"] = [dict(delta["entries"][0], status="unchanged")]
        delta["removed"] = []
        self.write("crap-delta.json", delta)
        baseline = self.write("baseline.json", report_fixture())
        with self.assertRaisesRegex(ValueError, "different functions"):
            crap_report.write_summary(self.directory, baseline)


class CoverageFailureTests(unittest.TestCase):
    def test_failed_instrumentation_or_tests_remove_stale_reports(self):
        for failure in ("show-env", "test"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                scripts = root / ".github/scripts"
                scripts.mkdir(parents=True)
                shutil.copyfile(Path(__file__).with_name("crap.sh"), scripts / "crap.sh")
                output = root / "target/quality"
                output.mkdir(parents=True)
                stale = ("crap.json", "crap-delta.json", "summary.md", "lcov.info", "coverage-run.json")
                for name in stale:
                    (output / name).write_text("stale", encoding="utf-8")
                binaries = root / "bin"
                binaries.mkdir()
                cargo = binaries / "cargo"
                cargo.write_text("""#!/usr/bin/env bash
set -eu
case "$*" in
  'llvm-cov --version') echo 'cargo-llvm-cov 0.9.1' ;;
  'llvm-cov show-env --sh')
    if [[ "$CRAP_TEST_FAILURE" == show-env ]]; then exit 7; fi
    echo 'export CRAP_TEST_INSTRUMENTED=1' ;;
  'llvm-cov clean --workspace --locked') : ;;
  'test --workspace --locked')
    [[ "${CRAP_TEST_INSTRUMENTED:-}" == 1 ]]
    exit 9 ;;
  *) echo "Unexpected command: $*" >&2; exit 99 ;;
esac
""", encoding="utf-8")
                cargo.chmod(0o755)
                env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ["PATH"],
                           CRAP_TEST_FAILURE=failure)
                result = subprocess.run(["bash", str(scripts / "crap.sh"), "coverage"],
                                        env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 7 if failure == "show-env" else 9, result.stderr)
                self.assertTrue(all(not (output / name).exists() for name in stale))


if __name__ == "__main__":
    unittest.main()

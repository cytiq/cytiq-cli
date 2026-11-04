"""Fast checks for measurement isolation, acceptance and failure reporting."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run as benchmark


class PerformanceTests(unittest.TestCase):
    def test_rss_units_are_explicit(self):
        self.assertEqual(benchmark.rss_bytes(1234, "darwin"), 1234)
        self.assertEqual(benchmark.rss_bytes(1234, "linux"), 1234 * 1024)
        with self.assertRaises(RuntimeError):
            benchmark.rss_bytes(1234, "unknown")

    def test_acceptance_uses_both_memory_and_upper_confidence_bound(self):
        def pairs(wall, rss):
            return [{"baseline": {"wall_seconds": 1.0, "peak_rss_bytes": 1000},
                     "candidate": {"wall_seconds": wall, "peak_rss_bytes": rss}}] * 20

        self.assertTrue(benchmark.summarize(pairs(1.0, 800), True)["acceptance"])
        self.assertFalse(benchmark.summarize(pairs(1.06, 800), True)["acceptance"])
        self.assertFalse(benchmark.summarize(pairs(1.0, 950), True)["acceptance"])
        uncertain = pairs(0.9, 800)[:11] + pairs(1.2, 800)[:9]
        self.assertFalse(benchmark.summarize(uncertain, True)["acceptance"])
        self.assertIsNone(benchmark.summarize(pairs(1.0, 800), False)["acceptance"])

    def test_bootstrap_is_repeatable(self):
        pairs = [{"baseline": {"wall_seconds": 1, "peak_rss_bytes": 100},
                  "candidate": {"wall_seconds": 0.7 + i / 100, "peak_rss_bytes": 70}}
                 for i in range(20)]
        self.assertEqual(benchmark.summarize(pairs, True), benchmark.summarize(pairs, True))

    def test_measurement_is_per_child_and_preserves_exit_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            big = benchmark.measure([sys.executable, "-c", "a = bytearray(32 * 1024 * 1024)"],
                                    root / "big", os.environ.copy())
            small = benchmark.measure([sys.executable, "-c", "pass"], root / "small", os.environ.copy())
            self.assertLess(small["peak_rss_bytes"], big["peak_rss_bytes"] * 0.8)
            self.assertGreater(small["wall_seconds"], 0)
            with self.assertRaisesRegex(RuntimeError, "exited 7"):
                benchmark.measure([sys.executable, "-c", "raise SystemExit(7)"],
                                  root / "failure", os.environ.copy())

    def test_parity_failure_is_written_as_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "input.fcs"
            source.write_bytes(b"input")
            args = SimpleNamespace(output=root / "report", pairs=20, warmups=2, smoke=False,
                                   synthetic_only=True, baseline_bin=Path(sys.executable),
                                   candidate_bin=Path(sys.executable), helper_bin=Path(sys.executable),
                                   case=None)
            case = {"name": "large_compensate", "stage": "compensate", "inputs": [source], "events": 1}
            sample = {"wall_seconds": 1, "user_seconds": 0, "system_seconds": 0, "peak_rss_bytes": 100}
            with patch.object(benchmark, "host_metadata", return_value={}), \
                    patch.object(benchmark, "prepare", return_value=[case]), \
                    patch.object(benchmark, "measure", return_value=sample), \
                    patch.object(benchmark, "verify_outputs", side_effect=subprocess.CalledProcessError(1, "compare")):
                with self.assertRaises(subprocess.CalledProcessError):
                    benchmark.run(args)
            report = json.loads((args.output / "results.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("error", report)
            self.assertNotIn("summary", report["cases"][0])

    def test_existing_report_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "already exists"):
                benchmark.run(SimpleNamespace(output=Path(directory)))

    def test_reads_directory_provenance_and_rejects_debug_builds(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "files").mkdir()
            path = root / "files/provenance.json"
            build = {"profile": "release", "git_commit": "baseline"}
            path.write_text(json.dumps({"build": build}))
            self.assertEqual(benchmark.provenance_build(root), build)
            build["profile"] = "debug"
            path.write_text(json.dumps({"build": build}))
            with self.assertRaisesRegex(RuntimeError, "release"):
                benchmark.provenance_build(root)


if __name__ == "__main__":
    unittest.main()

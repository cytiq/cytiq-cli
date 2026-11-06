import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest

import check_contracts


def repository(directory, files):
    subprocess.run(["git", "init", "-q", str(directory)], check=True)
    for name, text in files.items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    subprocess.run(["git", "-C", str(directory), "add", "."], check=True)
    subprocess.run(
        ["git", "-C", str(directory), "-c", "user.name=t", "-c", "user.email=t@example.com",
         "-c", "commit.gpgsign=false", "commit", "-q", "-m", "fixture"],
        check=True,
    )
    return subprocess.run(
        ["git", "-C", str(directory), "rev-parse", "HEAD"], check=True, capture_output=True, text=True
    ).stdout.strip()


class CheckContractsTest(unittest.TestCase):
    def test_matching_and_changed_upstream_files(self):
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            revision = repository(checkout, {"schema/a.json": "{}\n"})
            entry = {"source": "schema/a.json", "sha256": hashlib.sha256(b"{}\n").hexdigest()}
            lock = {"revision": revision, "files": [entry]}
            self.assertEqual(check_contracts.tree_problems(lock, checkout), [])
            entry["sha256"] = "0" * 64
            self.assertEqual(len(check_contracts.tree_problems(lock, checkout)), 1)
            entry["source"] = "schema/missing.json"
            self.assertIn("missing.json", check_contracts.tree_problems(lock, checkout)[0])

    def test_ml_revision_is_optional_but_must_exist(self):
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            revision = repository(checkout, {"README.md": "ml\n"})
            self.assertEqual(check_contracts.ml_problems({"revision": None}, checkout), [])
            self.assertEqual(check_contracts.ml_problems({"revision": revision}, checkout), [])
            self.assertEqual(len(check_contracts.ml_problems({"revision": "f" * 40}, checkout)), 1)


if __name__ == "__main__":
    unittest.main()

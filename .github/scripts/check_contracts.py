"""Check vendored upstream contracts against sibling checkouts.

cytiq-tree: every file in crates/cytiq-population/contracts/cytiq-tree.lock.json
must match, byte for byte, `git show <revision>:<source>` in $CYTIQ_TREE_DIR
(default ../cytiq-tree). cytiq-ml: when data/fixtures/classify/cytiq-ml.lock.json
records a revision, that commit must exist in $CYTIQ_ML_DIR (default ../cytiq-ml).
The offline cargo tests already check the vendored files against the locks;
this script checks the locks against upstream, so run it when bumping a pin.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TREE_LOCK = ROOT / "crates/cytiq-population/contracts/cytiq-tree.lock.json"
ML_LOCK = ROOT / "data/fixtures/classify/cytiq-ml.lock.json"


def git(checkout, *args):
    return subprocess.run(
        ["git", "-C", str(checkout), *args], check=True, capture_output=True
    ).stdout


def tree_problems(lock, checkout):
    """Files whose upstream bytes differ from the locked digest."""
    problems = []
    revision = lock["revision"]
    for entry in lock["files"]:
        try:
            upstream = git(checkout, "show", f"{revision}:{entry['source']}")
        except subprocess.CalledProcessError as error:
            problems.append(f"{entry['source']}@{revision}: {error.stderr.decode().strip()}")
            continue
        digest = hashlib.sha256(upstream).hexdigest()
        if digest != entry["sha256"]:
            problems.append(
                f"{entry['source']}@{revision}: upstream sha256 {digest}, locked {entry['sha256']}"
            )
    return problems


def ml_problems(lock, checkout):
    """A recorded revision that the checkout does not contain."""
    revision = lock.get("revision")
    if revision is None:
        return []
    try:
        git(checkout, "cat-file", "-e", f"{revision}^{{commit}}")
    except subprocess.CalledProcessError:
        return [f"cytiq-ml revision {revision} not found in {checkout}"]
    return []


def main():
    tree = Path(os.environ.get("CYTIQ_TREE_DIR", ROOT.parent / "cytiq-tree"))
    ml = Path(os.environ.get("CYTIQ_ML_DIR", ROOT.parent / "cytiq-ml"))
    tree_lock = json.loads(TREE_LOCK.read_text())
    ml_lock = json.loads(ML_LOCK.read_text())
    problems = tree_problems(tree_lock, tree) + ml_problems(ml_lock, ml)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(
        f"cytiq-tree@{tree_lock['revision'][:7]}: {len(tree_lock['files'])} file(s) match; "
        f"cytiq-ml revision: {ml_lock.get('revision') or 'not recorded'}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

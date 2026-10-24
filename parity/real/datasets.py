"""Materialise the real-data registry (parity/real/datasets.json).

Usage: datasets.py <out_dir>

Copies every dataset's files into <out_dir>/<name>/, from the R library in
R_LIBS_USER or from a URL (cached in parity/.data/real-cache by SHA-256), and
checks each file's SHA-256. Prints one `name<TAB>chain` line per dataset that
is ready. A missing or mismatching file fails the run, except for an
`optional` dataset whose download fails, which is skipped with a warning.
"""

import hashlib
import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

PARITY = Path(__file__).resolve().parent.parent
CACHE = PARITY / ".data" / "real-cache"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def source(spec: dict) -> Path:
    if "r_package" in spec:
        library = Path(os.environ["R_LIBS_USER"])
        return library / spec["r_package"] / spec["path"]
    cached = CACHE / spec["sha256"]
    if not cached.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        partial = cached.with_suffix(".part")
        with urllib.request.urlopen(spec["url"], timeout=300) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out)
        partial.rename(cached)
    return cached


def materialise(dataset: dict, out_dir: Path) -> None:
    target = out_dir / dataset["name"]
    target.mkdir(parents=True, exist_ok=True)
    for name, spec in dataset["files"].items():
        path = source(spec)
        if not path.exists():
            raise FileNotFoundError(f"{path} (run parity/setup_env.sh)")
        actual = sha256(path)
        if actual != spec["sha256"]:
            raise ValueError(f"{path}: SHA-256 {actual}, expected {spec['sha256']}")
        shutil.copyfile(path, target / name)


def main() -> None:
    out_dir = Path(sys.argv[1])
    registry = json.loads((Path(__file__).parent / "datasets.json").read_text())
    for dataset in registry["datasets"]:
        try:
            materialise(dataset, out_dir)
        except OSError as error:
            if not dataset.get("optional"):
                raise
            print(f"skipping {dataset['name']}: {error}", file=sys.stderr)
            continue
        print(f"{dataset['name']}\t{dataset['chain']}")


if __name__ == "__main__":
    main()

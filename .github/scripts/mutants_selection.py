"""Validate the actual mutation selection before an expensive test run."""

import json
from pathlib import Path
import re
import sys


def selection_patterns(config):
    """Read the controlled, single-quoted regex list without a TOML dependency."""
    match = re.search(r"(?ms)^examine_re = \[\n(.*?)^\]", config)
    if not match:
        raise ValueError("Missing examine_re list in .cargo/mutants.toml")
    patterns = []
    for line in match.group(1).splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        literal = re.fullmatch(r"\s*'([^']*)',?\s*", line)
        if not literal:
            raise ValueError("Use one single-quoted examine_re pattern per line")
        pattern = literal.group(1)
        re.compile(pattern)
        patterns.append(pattern)
    if not patterns:
        raise ValueError("Empty examine_re list in .cargo/mutants.toml")
    return patterns


def validate_selection(mutants, patterns):
    if not mutants:
        raise ValueError("No mutants selected; check .cargo/mutants.toml")
    for mutant in mutants:
        if not any(re.search(pattern, mutant["name"]) for pattern in patterns):
            raise ValueError(f"Mutation outside the configured functions: {mutant['name']}")


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[2]
    config = (root / ".cargo/mutants.toml").read_text(encoding="utf-8")
    mutants = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_selection(mutants, selection_patterns(config))
    print(f"Selected {len(mutants)} mutants")

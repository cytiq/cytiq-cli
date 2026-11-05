"""Publish the workspace crates to crates.io, resumably.

Every publishable workspace package carries the workspace version and is
published by one `cargo publish --workspace`, which orders packages by their
dependencies and waits for each to reach the index. Versions already in the
index are excluded, so a run that failed halfway can simply be repeated.

Checks that run before anything is uploaded:

- every package has the description and license crates.io requires;
- all packages share one version, equal to `--version` when given (the tag);
- every dependency on a workspace package requires exactly that version;
- every package already exists on crates.io, unless `--allow-new`: trusted
  publishing cannot create a crate, so its first version is published once
  by hand with an API token (see CONTRIBUTING.md).

`--check` runs the first three checks and packages every crate, published or
not, without building or uploading anything: the pull request check.

crates.io limits how fast new crates may be created. `--wait-on-rate-limit`
sleeps until the time its refusal names and resumes; use it for a first
publication, never with a short-lived trusted publishing token.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime

INDEX = "https://index.crates.io"
USER_AGENT = "cytiq-engine-publish (+https://github.com/cytiq/cytiq-engine)"
RATE_LIMIT_FALLBACK_SECONDS = 10 * 60 + 30
RATE_LIMIT_MARGIN_SECONDS = 15


@dataclass(frozen=True)
class Package:
    name: str
    version: str
    # (dependency name, version requirement) for dependencies on workspace packages
    internal: tuple[tuple[str, str], ...] = field(default=())
    # manifest fields crates.io requires that the package leaves empty
    missing: tuple[str, ...] = field(default=())


REQUIRED_FIELDS = ("description", "license")


def workspace_packages(metadata: dict) -> list[Package]:
    """Publishable workspace packages from `cargo metadata --no-deps`."""
    members = set(metadata["workspace_members"])
    names = {
        package["name"] for package in metadata["packages"] if package["id"] in members
    }
    packages = []
    for package in metadata["packages"]:
        if package["id"] not in members or package.get("publish") == []:
            continue
        internal = tuple(
            sorted(
                (dependency["name"], dependency["req"])
                for dependency in package["dependencies"]
                if dependency["name"] in names and dependency.get("path")
            )
        )
        missing = tuple(key for key in REQUIRED_FIELDS if not package.get(key))
        packages.append(Package(package["name"], package["version"], internal, missing))
    return sorted(packages, key=lambda package: package.name)


def release_errors(packages: list[Package], expected: str | None) -> list[str]:
    """Why these packages cannot be released together as one version."""
    versions = sorted({package.version for package in packages})
    if len(versions) != 1:
        return [f"workspace packages disagree on the version: {', '.join(versions)}"]
    version = versions[0]
    errors = [
        f"{package.name} has no {key} in its Cargo.toml"
        for package in packages
        for key in package.missing
    ]
    if expected is not None and version != expected:
        errors.append(f"workspace version {version} does not match the release {expected}")
    publishable = {package.name for package in packages}
    for package in packages:
        for name, requirement in package.internal:
            if name not in publishable:
                errors.append(f"{package.name} depends on {name}, which is not published")
            elif requirement != f"^{version}":
                errors.append(
                    f"{package.name} requires {name} {requirement}, not {version}: "
                    "set it in [workspace.dependencies] of the root Cargo.toml"
                )
    return errors


def index_path(name: str) -> str:
    """The sparse index path of a crate name."""
    name = name.lower()
    if len(name) <= 2:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def parse_index(text: str) -> set[str]:
    """Versions in a sparse index file, yanked ones included: they cannot be reused."""
    return {json.loads(line)["vers"] for line in text.splitlines() if line.strip()}


def published_versions(name: str) -> set[str] | None:
    """Versions of `name` on crates.io, or None when the crate does not exist."""
    request = urllib.request.Request(
        f"{INDEX}/{index_path(name)}", headers={"User-Agent": USER_AGENT}
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return parse_index(response.read().decode())
    except urllib.error.HTTPError as error:
        if error.code in (403, 404):
            return None
        raise


@dataclass(frozen=True)
class Plan:
    new: list[str]
    pending: list[str]
    published: list[str]


def plan(packages: list[Package], index: dict[str, set[str] | None]) -> Plan:
    """Split packages into new crates, versions still to publish and versions already out."""
    new = [package.name for package in packages if index[package.name] is None]
    published = [
        package.name
        for package in packages
        if index[package.name] is not None and package.version in index[package.name]
    ]
    pending = [package.name for package in packages if package.name not in published]
    return Plan(new=new, pending=pending, published=published)


def publish_command(
    published: list[str], dry_run: bool, no_verify: bool, allow_dirty: bool = False
) -> list[str]:
    command = ["cargo", "publish", "--workspace", "--locked"]
    for flag, enabled in [("--dry-run", dry_run), ("--no-verify", no_verify), ("--allow-dirty", allow_dirty)]:
        if enabled:
            command.append(flag)
    for name in published:
        command += ["--exclude", name]
    return command


def rate_limit_wait(output: str, now: datetime) -> float | None:
    """Seconds to wait when crates.io refused a publication as too frequent, else None."""
    if not re.search(r"\b429\b|too many requests|published too many", output, re.IGNORECASE):
        return None
    retry = re.search(r"try again after ([A-Z][a-z]{2}, \d{1,2} [A-Z][a-z]{2} \d{4} [\d:]+ GMT)", output)
    if retry is None:
        return float(RATE_LIMIT_FALLBACK_SECONDS)
    seconds = (parsedate_to_datetime(retry.group(1)) - now).total_seconds()
    return max(seconds, 0.0) + RATE_LIMIT_MARGIN_SECONDS


def run(command: list[str]) -> tuple[int, str]:
    """Run `command`, streaming its stderr while keeping a copy to inspect."""
    print("+ " + " ".join(command), file=sys.stderr, flush=True)
    process = subprocess.Popen(command, stderr=subprocess.PIPE, text=True)
    lines = []
    assert process.stderr is not None
    for line in process.stderr:
        sys.stderr.write(line)
        lines.append(line)
    return process.wait(), "".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", help="release version every package must carry")
    parser.add_argument(
        "--check", action="store_true",
        help="run the checks and package every crate without crates.io (pull requests)",
    )
    parser.add_argument("--dry-run", action="store_true", help="package and verify, upload nothing")
    parser.add_argument("--no-verify", action="store_true", help="skip the build of each package")
    parser.add_argument("--allow-new", action="store_true", help="allow crates absent from crates.io")
    parser.add_argument("--allow-dirty", action="store_true", help="package uncommitted changes (local checks)")
    parser.add_argument(
        "--wait-on-rate-limit", type=int, default=0, metavar="N",
        help="wait out at most N crates.io rate-limit refusals (first publication only)",
    )
    args = parser.parse_args(argv)

    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            check=True, capture_output=True, text=True,
        ).stdout
    )
    packages = workspace_packages(metadata)
    errors = release_errors(packages, args.version)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    if args.check:
        command = ["cargo", "package", "--workspace", "--locked", "--no-verify"]
        return run(command + (["--allow-dirty"] if args.allow_dirty else []))[0]

    waits = 0
    while True:
        current = plan(packages, {package.name: published_versions(package.name) for package in packages})
        if current.new and not args.allow_new:
            print(
                "error: not on crates.io yet: " + ", ".join(current.new) + ". Trusted publishing "
                "cannot create crates; publish their first version with an API token "
                "(CONTRIBUTING.md, Releases) and add the trusted publisher, then rerun.",
                file=sys.stderr,
            )
            return 1
        if current.new:
            print(
                "warning: not on crates.io yet: " + ", ".join(current.new)
                + ". The release workflow cannot create them; publish their first version by hand.",
                file=sys.stderr,
            )
        if not current.pending:
            print(f"all {len(packages)} packages are already on crates.io", file=sys.stderr)
            return 0
        print(
            f"{'verifying' if args.dry_run else 'publishing'} "
            f"{len(current.pending)} of {len(packages)} packages: "
            + ", ".join(current.pending),
            file=sys.stderr,
        )
        status, output = run(
            publish_command(current.published, args.dry_run, args.no_verify, args.allow_dirty)
        )
        if status == 0:
            return 0
        wait = rate_limit_wait(output, datetime.now(timezone.utc))
        if args.dry_run or wait is None or waits >= args.wait_on_rate_limit:
            return status
        waits += 1
        print(f"rate limited by crates.io; resuming in {wait:.0f} s", file=sys.stderr, flush=True)
        time.sleep(wait)


if __name__ == "__main__":
    sys.exit(main())

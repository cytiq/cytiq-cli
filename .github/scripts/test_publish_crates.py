import unittest
from datetime import datetime, timezone

from publish_crates import (
    RATE_LIMIT_FALLBACK_SECONDS,
    RATE_LIMIT_MARGIN_SECONDS,
    Package,
    index_path,
    parse_index,
    plan,
    publish_command,
    rate_limit_wait,
    release_errors,
    workspace_packages,
)


def metadata_package(name, version="0.3.1", dependencies=(), publish=None, description="d"):
    return {
        "id": f"path+file:///w/crates/{name}#{version}",
        "name": name,
        "version": version,
        "description": description,
        "license": "MIT",
        "publish": publish,
        "dependencies": [
            {"name": dependency, "req": req, **({"path": f"/w/crates/{dependency}"} if path else {})}
            for dependency, req, path in dependencies
        ],
    }


class WorkspacePackagesTests(unittest.TestCase):
    def test_keeps_path_dependencies_on_members_and_skips_unpublished_packages(self):
        packages = [
            metadata_package("cytiq-fcs", dependencies=[("serde", "^1", False)], description=None),
            metadata_package("cytiq-engine", dependencies=[("cytiq-fcs", "^0.3.1", True)]),
            metadata_package("cytiq-internal", publish=[]),
            # Same name as a member but from the registry: not a workspace dependency.
            metadata_package("outside", dependencies=[("cytiq-fcs", "^0.2", False)]),
        ]
        metadata = {"packages": packages, "workspace_members": [p["id"] for p in packages[:3]]}
        self.assertEqual(
            workspace_packages(metadata),
            [
                Package("cytiq-engine", "0.3.1", (("cytiq-fcs", "^0.3.1"),)),
                Package("cytiq-fcs", "0.3.1", (), ("description",)),
            ],
        )


class ReleaseErrorsTests(unittest.TestCase):
    fcs = Package("cytiq-fcs", "0.3.2")

    def test_accepts_one_version_matching_the_release(self):
        cli = Package("cytiq-engine", "0.3.2", (("cytiq-fcs", "^0.3.2"),))
        self.assertEqual(release_errors([cli, self.fcs], "0.3.2"), [])
        self.assertEqual(release_errors([cli, self.fcs], None), [])

    def test_rejects_a_release_tag_other_than_the_workspace_version(self):
        self.assertRegex(release_errors([self.fcs], "0.3.3")[0], "does not match the release 0.3.3")

    def test_rejects_packages_with_different_versions(self):
        errors = release_errors([self.fcs, Package("cytiq-engine", "0.3.1")], None)
        self.assertEqual(errors, ["workspace packages disagree on the version: 0.3.1, 0.3.2"])

    def test_rejects_a_stale_or_missing_internal_requirement(self):
        for requirement in ["^0.3.1", "*", "=0.3.2"]:
            cli = Package("cytiq-engine", "0.3.2", (("cytiq-fcs", requirement),))
            with self.subTest(requirement=requirement):
                self.assertRegex(release_errors([cli, self.fcs], None)[0], "workspace.dependencies")

    def test_rejects_packages_without_the_fields_crates_io_requires(self):
        fcs = Package("cytiq-fcs", "0.3.2", (), ("description", "license"))
        self.assertEqual(
            release_errors([fcs], None),
            ["cytiq-fcs has no description in its Cargo.toml", "cytiq-fcs has no license in its Cargo.toml"],
        )

    def test_rejects_a_dependency_on_an_unpublished_member(self):
        cli = Package("cytiq-engine", "0.3.2", (("cytiq-internal", "^0.3.2"),))
        self.assertRegex(release_errors([cli], None)[0], "cytiq-internal, which is not published")


class IndexTests(unittest.TestCase):
    def test_builds_sparse_index_paths(self):
        self.assertEqual(index_path("cytiq-fcs"), "cy/ti/cytiq-fcs")
        self.assertEqual(index_path("Serde"), "se/rd/serde")
        self.assertEqual(index_path("syn"), "3/s/syn")
        self.assertEqual(index_path("cc"), "2/cc")
        self.assertEqual(index_path("a"), "1/a")

    def test_reads_every_version_including_yanked_ones(self):
        text = '{"name":"x","vers":"0.3.1","yanked":false}\n{"name":"x","vers":"0.3.2","yanked":true}\n\n'
        self.assertEqual(parse_index(text), {"0.3.1", "0.3.2"})


class PlanTests(unittest.TestCase):
    packages = [Package("a", "0.3.2"), Package("b", "0.3.2"), Package("c", "0.3.2")]

    def test_resumes_after_a_partial_publication(self):
        result = plan(self.packages, {"a": {"0.3.1", "0.3.2"}, "b": {"0.3.1"}, "c": None})
        self.assertEqual(result.published, ["a"])
        self.assertEqual(result.pending, ["b", "c"])
        self.assertEqual(result.new, ["c"])

    def test_excludes_published_versions_from_the_command(self):
        self.assertEqual(
            publish_command(["a", "b"], dry_run=True, no_verify=False),
            ["cargo", "publish", "--workspace", "--locked", "--dry-run", "--exclude", "a", "--exclude", "b"],
        )
        self.assertEqual(
            publish_command([], dry_run=False, no_verify=True),
            ["cargo", "publish", "--workspace", "--locked", "--no-verify"],
        )
        self.assertEqual(
            publish_command([], dry_run=True, no_verify=True, allow_dirty=True),
            ["cargo", "publish", "--workspace", "--locked", "--dry-run", "--no-verify", "--allow-dirty"],
        )


class RateLimitTests(unittest.TestCase):
    now = datetime(2026, 11, 5, 20, 0, 0, tzinfo=timezone.utc)

    def test_waits_until_the_time_crates_io_names(self):
        output = (
            "error: failed to publish to registry at https://crates.io\n\nCaused by:\n"
            "  the remote server responded with an error (status 429 Too Many Requests): You have "
            "published too many new crates in a short period of time. Please try again after "
            "Thu, 05 Nov 2026 20:07:30 GMT or email help@crates.io to have your limit increased."
        )
        self.assertEqual(rate_limit_wait(output, self.now), 450 + RATE_LIMIT_MARGIN_SECONDS)

    def test_falls_back_to_the_new_crate_interval_without_a_retry_time(self):
        output = "status 429 Too Many Requests"
        self.assertEqual(rate_limit_wait(output, self.now), RATE_LIMIT_FALLBACK_SECONDS)

    def test_never_waits_for_a_retry_time_in_the_past(self):
        output = "status 429 Too Many Requests: try again after Thu, 05 Nov 2026 19:59:00 GMT"
        self.assertEqual(rate_limit_wait(output, self.now), RATE_LIMIT_MARGIN_SECONDS)

    def test_ignores_other_failures(self):
        for output in ["error: crate version `0.3.2` is already uploaded", "status 403 Forbidden"]:
            with self.subTest(output=output):
                self.assertIsNone(rate_limit_wait(output, self.now))


if __name__ == "__main__":
    unittest.main()

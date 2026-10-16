#!/usr/bin/env python3
"""Check the job groups that ci-changes.py picks for typical pull requests."""

import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "ci_changes", pathlib.Path(__file__).with_name("ci-changes.py")
)
ci_changes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_changes)

AGENT = {"restichub-agent", "restichub-common", "restichub-compiler", "restichub-proto"}


def picked(*files):
    result = ci_changes.classify(list(files), AGENT)
    return {group for group, on in result.items() if on}


class Classify(unittest.TestCase):
    def test_ui_only(self):
        self.assertEqual(picked("ui/src/lib/a.ts"), {"ui", "openapi"})

    def test_docs_only(self):
        self.assertEqual(picked("docs/src/content/docs/a.md"), {"docs", "site"})
        self.assertEqual(picked("design/requirements.md"), {"docs"})
        self.assertEqual(picked("README.md"), {"docs"})

    def test_control_plane_skips_windows(self):
        got = picked("crates/restichub-cp/src/lib.rs")
        self.assertIn("rust", got)
        self.assertNotIn("windows", got)

    def test_agent_runs_windows(self):
        self.assertIn("windows", picked("crates/restichub-agent/src/lib.rs"))
        self.assertIn("windows", picked("crates/restichub-common/src/lib.rs"))

    def test_proto(self):
        self.assertTrue({"proto", "rust"} <= picked("proto/a.proto"))

    def test_ci_and_lock_run_everything(self):
        everything = set(ci_changes.GROUPS)
        self.assertEqual(picked(".github/workflows/ci.yml"), everything)
        self.assertEqual(picked("Cargo.lock"), everything)

    def test_unrelated_file_runs_nothing(self):
        self.assertEqual(picked("bench/run.sh"), set())

    def test_agent_package_args(self):
        self.assertEqual(
            ci_changes.package_args(AGENT),
            "-p restichub-agent -p restichub-common -p restichub-compiler -p restichub-proto",
        )


if __name__ == "__main__":
    unittest.main()

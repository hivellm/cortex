"""Installed smoke guards without changing a real installation."""

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import smoke_installed_local as smoke


class InstalledSmokeTests(unittest.TestCase):
    def test_documented_tools_match_actual_dispatch_not_advertisement_constant(self):
        result = smoke.tool_contract(smoke.DOCS.read_text(), smoke.DISPATCH.read_text())
        self.assertEqual(set(result["implemented"]), smoke.TOOLS)
        self.assertEqual(len(result["documented"]), 18)

    def test_missing_dispatch_arm_rejected_even_with_tools_constant_intact(self):
        source = smoke.DISPATCH.read_text()
        prefix, body = source.split("pub fn call(", 1)
        body = body.replace('"cortex_status" =>', '"unsupported_status" =>', 1)
        with self.assertRaisesRegex(ValueError, "Dispatch toolset differs"):
            smoke.tool_contract(smoke.DOCS.read_text(), prefix + "pub fn call(" + body)

    def test_added_documented_tool_and_duplicate_advertisement_rejected(self):
        document = smoke.DOCS.read_text() + "\n| `cortex_unimplemented` | Extra |\n"
        with self.assertRaisesRegex(ValueError, "Documented toolset differs"):
            smoke.tool_contract(document, smoke.DISPATCH.read_text())
        descriptors = [{"name": name} for name in sorted(smoke.TOOLS)]
        smoke.tool_names(descriptors)
        descriptors[-1] = copy.deepcopy(descriptors[0])
        with self.assertRaisesRegex(ValueError, "Advertised toolset differs"):
            smoke.tool_names(descriptors)

    def test_canonical_snapshot_detects_mutation_and_new_job_but_ignores_derived_db(
        self,
    ):
        with tempfile.TemporaryDirectory() as scratch:
            home = Path(scratch)
            local = home / "local"
            (local / "records").mkdir(parents=True)
            (local / "records/source.md").write_bytes(b"original")
            baseline = smoke.canonical_snapshot(home)
            (local / "index.sqlite3").write_bytes(b"derived projection")
            self.assertEqual(smoke.canonical_snapshot(home), baseline)
            (local / "classification/jobs/scope").mkdir(parents=True)
            (local / "classification/jobs/scope/job.json").write_bytes(b"new job")
            self.assertNotEqual(smoke.canonical_snapshot(home), baseline)
            (local / "records/source.md").write_bytes(b"changed")
            self.assertNotEqual(
                smoke.canonical_snapshot(home)["records/source.md"],
                baseline["records/source.md"],
            )

    def test_binary_mismatch_saves_failure_without_constructing_home(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            installed, reference = root / "installed", root / "reference"
            installed.write_bytes(b"first")
            reference.write_bytes(b"second")
            report_path = root / "report.json"
            with mock.patch.object(smoke.evaluation, "build_home") as build:
                result = smoke.evaluate(installed, reference, report_path)
            build.assert_not_called()
            self.assertFalse(result["passed"])
            self.assertEqual(
                result["binary_hashes_before"], result["binary_hashes_after"]
            )
            self.assertEqual(json.loads(report_path.read_bytes()), result)
            self.assertEqual(installed.read_bytes(), b"first")
            self.assertEqual(reference.read_bytes(), b"second")

    def test_readonly_set_excludes_all_five_mutating_tools(self):
        self.assertEqual(len(smoke.READONLY), 13)
        self.assertTrue(
            {
                "cortex_capture_memory",
                "cortex_revise",
                "cortex_classification_enqueue",
                "cortex_audit_retain",
                "cortex_feedback_record",
            }.isdisjoint(smoke.READONLY)
        )

    def test_mcp_numeric_false_rejected_even_when_cli_boolean_is_correct(self):
        cli = {"records": [], "edges": [], "truncated": False}
        mcp = {"records": [], "edges": [], "truncated": 0}
        with self.assertRaisesRegex(ValueError, "graph truncation differs"):
            smoke.validate_graph_pair(
                cli, mcp, "AtlasProduction", {}, [], Path("unused")
            )

    def test_identical_duplicate_nodes_on_both_transports_rejected(self):
        result = {
            "records": [{"id": "graph-report"}, {"id": "graph-report"}],
            "edges": [],
            "truncated": False,
        }
        with self.assertRaisesRegex(ValueError, "duplicate graph node ID"):
            smoke.validate_graph_pair(
                result,
                copy.deepcopy(result),
                "Atlas",
                {"graph-report": {}},
                [],
                Path("unused"),
            )


if __name__ == "__main__":
    unittest.main()

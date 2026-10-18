"""Exact graph identity, orientation and evidence contracts."""

import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock
import json
import tempfile

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location(
    "graph_evaluation", SCRIPTS / "evaluate_local_memory.py"
)
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


class GraphEvidenceTests(unittest.TestCase):
    def setUp(self):
        fixtures = evaluation.load_fixtures(evaluation.DEFAULT_FIXTURES)
        self.expected = {
            request["id"]: {**request, "revision": 1}
            for request in fixtures["graph"][:3]
        }
        self.records = []
        for expected in self.expected.values():
            record = copy.deepcopy(expected)
            record.update(
                schema_version=1,
                created_at="2026-02-03T12:00:00Z",
                updated_at="2026-02-03T12:00:00Z",
                content_sha256="",
            )
            record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
            self.records.append(record)
        self.edges = [
            {
                "source_id": "graph-report",
                "target_id": "graph-log",
                "relation": "derived_from",
            },
            {
                "source_id": "graph-log",
                "target_id": "graph-test",
                "relation": "supports",
            },
        ]
        self.result = {
            "records": self.records,
            "edges": copy.deepcopy(self.edges),
            "truncated": False,
        }

    def validate(self, result=None, expected=None, edges=None, truncated=False):
        return evaluation.validate_graph_result(
            result or self.result,
            "Atlas",
            self.expected if expected is None else expected,
            self.edges if edges is None else edges,
            Path("."),
            truncated,
        )

    def test_exact_evidence_is_order_independent(self):
        self.result["records"].reverse()
        self.result["edges"].reverse()
        evidence = self.validate()
        self.assertEqual(len(evidence["nodes"]), 3)

    def test_same_count_wrong_node_rejected(self):
        self.result["records"][0]["id"] = "graph-unrelated"
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "node identities differ"
        ):
            self.validate()

    def test_same_count_wrong_edge_endpoints_rejected(self):
        self.result["edges"][0]["target_id"] = "graph-test"
        with self.assertRaisesRegex(evaluation.EvaluationError, "typed edges differ"):
            self.validate()

    def test_reverse_traversal_does_not_reverse_edge_orientation(self):
        self.result["edges"][0]["source_id"], self.result["edges"][0]["target_id"] = (
            self.result["edges"][0]["target_id"],
            self.result["edges"][0]["source_id"],
        )
        with self.assertRaisesRegex(evaluation.EvaluationError, "typed edges differ"):
            self.validate()

    def test_wrong_relation_rejected(self):
        self.result["edges"][1]["relation"] = "related_to"
        with self.assertRaisesRegex(evaluation.EvaluationError, "typed edges differ"):
            self.validate()

    def test_duplicate_edge_or_node_not_hidden_by_sets(self):
        self.result["edges"][1] = copy.deepcopy(self.result["edges"][0])
        with self.assertRaisesRegex(evaluation.EvaluationError, "duplicate graph edge"):
            self.validate()
        self.result["edges"] = copy.deepcopy(self.edges)
        self.result["records"][1] = copy.deepcopy(self.result["records"][0])
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "duplicate graph node ID"
        ):
            self.validate()

    def test_wrong_provenance_with_recomputed_hash_rejected(self):
        record = self.result["records"][0]
        record["source"]["source_hash"] = "0" * 64
        record["content_sha256"] = ""
        record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "source source_hash mismatch"
        ):
            self.validate()

    def test_wrong_scope_rejected(self):
        self.result["records"][0]["repository"] = "atlas"
        with self.assertRaisesRegex(evaluation.EvaluationError, "scope differs"):
            self.validate()

    def test_resealed_foreign_fields_rejected(self):
        for location in ("source", "metadata", "record"):
            with self.subTest(location=location):
                result = copy.deepcopy(self.result)
                record = result["records"][0]
                target = record if location == "record" else record[location]
                target["foreign_repository_secret"] = "unrelated AtlasLab content"
                record["content_sha256"] = ""
                record["content_sha256"] = evaluation.digest(
                    evaluation.canonical(record)
                )
                with self.assertRaises(evaluation.EvaluationError):
                    self.validate(result)

    def test_archive_body_must_match_exact_projection_not_substrings(self):
        fixtures = evaluation.load_fixtures(evaluation.DEFAULT_FIXTURES)
        expected = fixtures["manifest"]["records"]["event-export-discussion"]
        fields = (
            "id",
            "repository",
            "revision",
            "kind",
            "authority",
            "title",
            "body",
            "source",
            "links",
            "metadata",
        )
        record = {field: copy.deepcopy(expected[field]) for field in fields}
        with tempfile.TemporaryDirectory(prefix="cortex-graph-history-") as root:
            sources = Path(root) / "sources"
            evaluation.materialize_sources(fixtures, sources)
            record["source"]["locator"] = (
                str((sources / expected["source_file"]).resolve())
                + "#line="
                + str(expected["source_line"])
            )
            record.update(
                schema_version=1,
                created_at="2026-02-03T12:00:00Z",
                updated_at="2026-02-03T12:00:00Z",
                content_sha256="",
            )
            record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
            evaluation.validate_record(record, expected, sources)
            record["body"] += "\nForeign AtlasLab content"
            record["content_sha256"] = ""
            record["content_sha256"] = evaluation.digest(evaluation.canonical(record))
            with self.assertRaisesRegex(evaluation.EvaluationError, "body mismatch"):
                evaluation.validate_record(record, expected, sources)

    def test_depth_zero_root_only_has_no_truncation(self):
        root = self.records[0]
        result = {"records": [root], "edges": [], "truncated": False}
        self.validate(result, {root["id"]: self.expected[root["id"]]}, [])
        result["truncated"] = True
        with self.assertRaisesRegex(evaluation.EvaluationError, "truncation differs"):
            self.validate(result, {root["id"]: self.expected[root["id"]]}, [])

    def test_node_bound_omits_edges_to_unreturned_nodes(self):
        root = self.records[0]
        result = {"records": [root], "edges": [self.edges[0]], "truncated": True}
        with self.assertRaisesRegex(evaluation.EvaluationError, "endpoint absent"):
            self.validate(result, {root["id"]: self.expected[root["id"]]}, [], True)
        result["edges"] = []
        self.validate(result, {root["id"]: self.expected[root["id"]]}, [], True)

    def test_missing_root_is_empty_without_invented_path(self):
        self.validate({"records": [], "edges": [], "truncated": False}, {}, [])

    def test_bound_rejection_must_be_specific_to_requested_constraint(self):
        result = {
            "isError": True,
            "content": [{"type": "text", "text": "unrelated failure"}],
        }
        with self.assertRaisesRegex(
            evaluation.EvaluationError, "unexpected MCP graph bounds"
        ):
            evaluation.validate_graph_bounds_error(result, 1, 101)
        result["content"][0]["text"] = (
            "invalid local record: numeric argument exceeds limit"
        )
        evaluation.validate_graph_bounds_error(result, 1, 101)
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_graph_bounds_error(result, 1, 0)

    def test_graph_failure_report_survives_successful_quality(self):
        report = {
            "quality": {"integrity_ok": True},
            "isolation": {"passed": True},
            "graph": {"passed": False, "checks": [{"error": "wrong edge"}]},
        }
        with tempfile.TemporaryDirectory(prefix="cortex-graph-report-") as root:
            destination = Path(root) / "report.json"
            with (
                mock.patch.object(evaluation, "evaluate", return_value=report),
                mock.patch.object(
                    sys,
                    "argv",
                    ["eval", "--binary", "unused", "--report", str(destination)],
                ),
            ):
                with self.assertRaises(SystemExit) as raised:
                    evaluation.main()
            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(json.loads(destination.read_text()), report)


if __name__ == "__main__":
    unittest.main()

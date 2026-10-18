"""Scope validation and persistent-session contamination regression tests."""

import copy
from pathlib import Path
import unittest

from scripts import evaluate_local_memory as evaluation
from scripts.tests import test_local_eval_scoring_adversarial as evidence_helpers


class ScopeTests(unittest.TestCase):
    def setUp(self):
        self.evidence = evidence_helpers.AdversarialScoringTests()
        self.evidence.setUp()
        self.addCleanup(self.evidence.doCleanups)
        self.fixtures = self.evidence.fixtures
        self.manifest = self.evidence.manifest
        self.sources = self.evidence.sources

    def test_case_colliding_foreign_record_rejected_with_valid_provenance(self):
        hit = self.evidence.hit("lower-storage")
        evaluation.validate_scope_hits([hit], "atlas", self.manifest, self.sources)
        with self.assertRaisesRegex(evaluation.EvaluationError, "foreign or unknown"):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_foreign_link_rejected_even_when_manifest_and_hash_match(self):
        manifest = copy.deepcopy(self.manifest)
        hit = self.evidence.hit("atlas-storage")
        links = [
            {
                "relation": "related_to",
                "target_id": manifest["records"]["lower-storage"]["id"],
            }
        ]
        hit["record"]["links"] = links
        manifest["records"]["atlas-storage"]["links"] = links
        self.evidence.seal(hit["record"])
        evaluation.validate_record(
            hit["record"], manifest["records"]["atlas-storage"], self.sources
        )
        with self.assertRaisesRegex(evaluation.EvaluationError, "foreign link target"):
            evaluation.validate_scope_hits([hit], "Atlas", manifest, self.sources)

    def test_resealed_extra_source_data_rejected(self):
        hit = self.evidence.hit("atlas-storage")
        hit["record"]["source"]["foreign_private_body"] = "Foreign scope evidence"
        self.evidence.seal(hit["record"])
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_resealed_extra_metadata_rejected(self):
        hit = self.evidence.hit("atlas-storage")
        hit["record"]["metadata"]["foreign_private_body"] = "Foreign scope evidence"
        self.evidence.seal(hit["record"])
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_resealed_daily_snapshot_raw_tampering_rejected(self):
        hit = self.evidence.hit("atlas-storage")
        metadata = hit["record"]["metadata"]["daily_import"]
        snapshot = evaluation.parse_json(metadata["snapshot_raw"])
        snapshot["body"] = "Foreign scope evidence"
        metadata["snapshot_raw"] = evaluation.canonical(snapshot).decode()
        self.evidence.seal(hit["record"])
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_resealed_archive_body_with_valid_text_plus_foreign_data_rejected(self):
        hit = self.evidence.hit("event-export-discussion")
        hit["record"]["body"] += "\nForeign scope evidence without citation"
        self.evidence.seal(hit["record"])
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_resealed_extra_top_level_field_rejected(self):
        hit = self.evidence.hit("atlas-storage")
        hit["record"]["foreign_private_body"] = "Foreign scope evidence"
        self.evidence.seal(hit["record"])
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.validate_scope_hits([hit], "Atlas", self.manifest, self.sources)

    def test_status_exact_scope_includes_inactive_records(self):
        expected = {
            "Atlas": (24, 6, 18, 4),
            "atlas": (3, 1, 2, 0),
            "AtlasLab": (3, 1, 2, 0),
            "AtlasProduction": (0, 0, 0, 0),
        }
        inactive = [
            r
            for r in self.manifest["records"].values()
            if r["repository"] == "Atlas" and r["authority"] == "superseded"
        ]
        self.assertEqual(len(inactive), 2)
        for scope, (records, events, memories, links) in expected.items():
            with self.subTest(scope=scope):
                self.assertEqual(
                    evaluation.scoped_status(self.manifest, scope),
                    {
                        "records": records,
                        "revisions": records,
                        "repositories": int(records != 0),
                        "events": events,
                        "memories": memories,
                        "links": links,
                    },
                )

    def run_session(self, contamination=None):
        owner = self
        captures = {}
        calls = []
        hits = {
            scope: [self.evidence.hit(key)]
            for scope, key in (
                ("Atlas", "atlas-storage"),
                ("atlas", "lower-storage"),
                ("AtlasLab", "atlaslab-storage"),
            )
        }

        def status(repository):
            result = evaluation.scoped_status(owner.manifest, repository)
            added = sum(scope == repository for scope, _ in captures)
            for field in ("records", "revisions", "memories"):
                result[field] += added
            return result

        class Cli:
            binary = Path("unused-fixture-binary")
            home = owner.sources.parent / "home"

            def run(self, *args):
                if args[0] == "capture":
                    request = evaluation.parse_json(Path(args[2]).read_bytes())
                    # A bounded in-memory transport, with actual request bytes.
                    record = copy.deepcopy(request)
                    record.update(
                        schema_version=1,
                        revision=1,
                        created_at="2026-01-01T00:00:00Z",
                        updated_at="2026-01-01T00:00:00Z",
                    )
                    owner.evidence.seal(record)
                    captures[(record["repository"], record["id"])] = record
                    return {"record": copy.deepcopy(record)}
                if args[0] == "reindex":
                    return {}
                repository, command = args[1:3]
                if command == "status":
                    return status(repository)
                if command == "search":
                    return copy.deepcopy(hits.get(repository, []))
                raise AssertionError(args)

        class Session:
            def __init__(self, *_args):
                self.round_counts = {}

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def call(self, tool, args):
                repository = args["repository"]
                calls.append((tool, repository))
                if tool == "cortex_search":
                    if args["query"] == "scopeprobe search":
                        records = [
                            r
                            for (scope, _), r in captures.items()
                            if scope == repository
                        ]
                        records.sort(
                            key=lambda r: r["source"]["captured_at"], reverse=True
                        )
                        if contamination == "ranking" and len(captures) == 5:
                            records.reverse()
                        return [
                            {"record": copy.deepcopy(record), "score": 2.0}
                            for record in records
                        ]
                    key = (repository, args["query"])
                    self.round_counts[key] = self.round_counts.get(key, 0) + 1
                    if (
                        contamination == "warm_hits"
                        and repository == "Atlas"
                        and self.round_counts[key] == 2
                    ):
                        return copy.deepcopy(hits["atlas"])
                    return copy.deepcopy(hits.get(repository, []))
                if tool == "cortex_status":
                    result = status(repository)
                    if contamination == "status" and repository == "atlas":
                        result["records"] += 24
                    return result
                if tool == "cortex_get":
                    key = (repository, args["id"])
                    if (
                        contamination == "shared_id"
                        and repository == "Atlas"
                        and len(captures) == 5
                    ):
                        key = ("atlas", args["id"])
                    return copy.deepcopy(captures.get(key))
                if tool == "cortex_graph":
                    record = captures.get((repository, args["id"]))
                    return {
                        "records": [copy.deepcopy(record)] if record else [],
                        "edges": [],
                        "truncated": False,
                    }
                if tool == "cortex_memory_read":
                    context = "Cortex local evidence (advisory; source authority unchanged):\n"
                    for hit in hits.get(repository, []):
                        r = hit["record"]
                        context += (
                            f"[{r['id']}@{r['revision']} {r['content_sha256']}] "
                            f"{r['title']} | repository={r['repository']} "
                            f"authority={r['authority'].capitalize()} source={r['source']['locator']}\n"
                            + r["body"]
                            + ("" if r["body"].endswith("\n") else "\n")
                        )
                    if contamination == "foreign_body" and repository == "Atlas":
                        context += hits["atlas"][0]["record"]["body"]
                    return {
                        "repository": repository,
                        "context": context,
                        "delivery_consumption": "not_observed",
                    }
                raise AssertionError(tool)

        report = evaluation.evaluate_scope(Cli(), self.fixtures, self.sources, Session)
        self.assertEqual(len(captures), 5)
        self.assertGreater(len(calls), 40)
        return report

    def test_clean_persistent_session_is_positive_control(self):
        report = self.run_session()
        self.assertTrue(report["passed"], report["checks"])

    def test_second_round_foreign_search_hits_fail_report(self):
        report = self.run_session("warm_hits")
        self.assertFalse(report["passed"])
        failures = [c for c in report["checks"] if not c["passed"]]
        self.assertTrue(failures)
        self.assertTrue(
            all(c["name"].startswith("mcp-search:1:Atlas:") for c in failures)
        )
        self.assertTrue(all("foreign or unknown" in c["error"] for c in failures))

    def test_cross_scope_statistics_fail_report(self):
        report = self.run_session("status")
        self.assertFalse(report["passed"])
        failures = [c for c in report["checks"] if not c["passed"]]
        self.assertEqual(
            {c["name"] for c in failures}, {"mcp-status:0:atlas", "mcp-status:1:atlas"}
        )
        self.assertTrue(all("statistics disclose" in c["error"] for c in failures))

    def test_same_id_foreign_capture_cannot_replace_primary_evidence(self):
        report = self.run_session("shared_id")
        self.assertFalse(report["passed"])
        failures = [c for c in report["checks"] if not c["passed"]]
        self.assertEqual(len(failures), 1)
        self.assertEqual(failures[0]["name"], "mcp-ranking-and-same-id-perturbation")
        self.assertIn("shared ID contaminated primary evidence", failures[0]["error"])

    def test_foreign_body_without_citation_is_detected(self):
        report = self.run_session("foreign_body")
        self.assertFalse(report["passed"])
        failures = [c for c in report["checks"] if not c["passed"]]
        self.assertEqual(
            {c["name"] for c in failures}, {"mcp-recall:0:Atlas", "mcp-recall:1:Atlas"}
        )

    def test_foreign_capture_cannot_reverse_two_primary_hits(self):
        report = self.run_session("ranking")
        self.assertFalse(report["passed"])
        failures = [c for c in report["checks"] if not c["passed"]]
        self.assertEqual(len(failures), 1)
        self.assertEqual(failures[0]["name"], "mcp-ranking-and-same-id-perturbation")
        self.assertIn(
            "foreign corpus changed ranking inputs/order", failures[0]["error"]
        )


if __name__ == "__main__":
    unittest.main()

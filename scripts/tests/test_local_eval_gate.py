"""Fail-closed policy tests independent of machine timings and generated artifacts."""

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import gate_local_memory as gate


class GateTests(unittest.TestCase):
    def setUp(self):
        self.policy = gate.read_report(gate.DEFAULT_BASELINE)
        fixtures = gate.quality.load_fixtures(gate.quality.DEFAULT_FIXTURES)
        records = fixtures["manifest"]["records"]
        build = dict(self.policy["required_build"], version="fixture-version")
        binary = {"sha256": "a" * 64, "build_info": build}
        home = {
            "sources_preserved": True,
            "rebuild_identical": True,
            "record_ids": {k: v["id"] for k, v in records.items()},
            "source_hashes": {
                "daily.sqlite": "b" * 64,
                "archive/events.ndjson.zst": self.policy["fixture_hashes"][
                    "events.ndjson.zst"
                ],
            },
            "imports": [
                {"complete": True, "rejected": 0, "errors": [], "imported": n}
                for n in (18, 8)
            ],
            "repeat_imports": [
                {"complete": True, "rejected": 0, "errors": [], "imported": 0}
            ]
            * 2,
            "reindex": {"records": 30, "revisions": 30},
        }
        queries = []
        for floor in self.policy["query_floors"]:
            q = copy.deepcopy(floor)
            q.update(integrity_ok=True, diagnostics=[], retrieved=[])
            q["expected_evidence"] = [
                {"id": records[k]["id"], "revision": 1, "source": records[k]["source"]}
                for k in q["expected_keys"]
            ]
            if not q["empty"] and q["split"] == "development":
                for rank, key in enumerate(q["expected_keys"], 1):
                    expected = records[key]
                    source = copy.deepcopy(expected["source"])
                    source.setdefault("locator", "fixture://reviewed-source")
                    q["retrieved"].append(
                        {
                            "rank": rank,
                            "valid": True,
                            "error": None,
                            "id": expected["id"],
                            "revision": 1,
                            "content_sha256": "d" * 64,
                            "source": source,
                        }
                    )
            q["recall_at_5"] = (
                min(len(q["retrieved"]), 5) / len(q["expected_ids"])
                if q["expected_ids"]
                else None
            )
            q["reciprocal_rank_at_10"] = (
                float(bool(q["retrieved"])) if q["expected_ids"] else None
            )
            queries.append(q)
        splits = {}
        for split in ("development", "held_out", "all"):
            selected = [q for q in queries if split == "all" or q["split"] == split]
            answerable = [q for q in selected if not q["empty"]]
            empties = len(selected) - len(answerable)
            splits[split] = {
                "answerable_queries": len(answerable),
                "empty_queries": empties,
                "empty_queries_without_hits": empties,
                "integrity_failed_queries": 0,
                "integrity_ok": True,
                "recall_at_5": sum(q["recall_at_5"] for q in answerable)
                / len(answerable),
                "mrr_at_10": sum(q["reciprocal_rank_at_10"] for q in answerable)
                / len(answerable),
            }
        self.q = {
            "schema_version": 1,
            "binary": copy.deepcopy(binary),
            "fixture_manifest_sha256": self.policy["fixture_manifest_sha256"],
            "fixture_hashes": self.policy["fixture_hashes"],
            "stable_projection_sha256": self.policy["stable_projection_sha256"],
            "reproducible": True,
            "namespace": "local-eval-v1",
            "homes": [home, copy.deepcopy(home)],
            "quality": {"integrity_ok": True, "queries": queries, "splits": splits},
        }
        for field, names in (
            ("isolation", "scope_check_names"),
            ("graph", "graph_check_names"),
        ):
            self.q[field] = {
                "passed": True,
                "checks": [{"name": n, "passed": True} for n in self.policy[names]],
            }
        series = {}
        for name in self.policy["latency"]["series"]:
            series[name] = {
                "planned": 20,
                "measured": 20,
                "successes": 20,
                "failures": 0,
                "unmeasured": 0,
                "p50_ns": 10,
                "p95_ns": 19,
                "samples": [
                    {"ordinal": n, "elapsed_ns": n, "success": True}
                    for n in range(1, 21)
                ],
            }
        self.l = {
            "schema_version": 1,
            "started_at": "2026-10-18T12:00:00+00:00",
            "finished_at": "2026-10-18T12:00:05+00:00",
            "environment": {
                "os": "fixture OS",
                "python": "3.13.9",
                "machine": None,
                "processor": None,
                "logical_cpus": None,
            },
            "passed": True,
            "binary": copy.deepcopy(binary),
            "corpus": {
                "manifest_sha256": self.policy["fixture_manifest_sha256"],
                "files": self.policy["fixture_hashes"],
            },
            "latency": {
                "passed": True,
                "binary_sources_and_original_records_preserved": True,
                "repository": "Atlas",
                "query": "offline desktop notes",
                "concurrency": 1,
                "clock": "time.perf_counter_ns",
                "samples_per_series": 20,
                "series": series,
            },
        }

    def assess(self):
        return gate.assess(self.q, self.l, self.policy)

    def test_nonregression_can_pass_while_natural_language_not_ready(self):
        result = self.assess()
        self.assertTrue(result["passed"], result)
        self.assertFalse(result["natural_language_ready"])
        self.assertEqual(result["readiness"]["observed"]["recall_at_5"], 0)

    def test_new_release_hash_allowed_but_reports_must_match(self):
        self.assertNotEqual(
            self.q["binary"]["sha256"], self.policy["baseline_binary_sha256"]
        )
        self.assertTrue(self.assess()["passed"])
        self.l["binary"]["sha256"] = "e" * 64
        self.assertFalse(self.assess()["passed"])

    def test_duplicate_or_missing_query_rejected(self):
        self.q["quality"]["queries"][-1] = self.q["quality"]["queries"][0]
        self.assertFalse(self.assess()["passed"])

    def test_retrieval_regression_not_hidden_by_claimed_metric(self):
        self.q["quality"]["queries"][0]["retrieved"] = []
        self.assertFalse(self.assess()["passed"])

    def test_boolean_and_nonfinite_scores_cannot_impersonate_valid_metrics(self):
        query = self.q["quality"]["queries"][0]
        for bad in (True, float("nan"), float("inf")):
            with self.subTest(value=bad):
                query["recall_at_5"] = bad
                self.assertFalse(self.assess()["passed"])

    def test_missing_latency_environment_and_invalid_cpu_count_fail(self):
        environment = copy.deepcopy(self.l["environment"])
        for bad in (
            None,
            {},
            {**environment, "logical_cpus": True},
            {**environment, "logical_cpus": 0},
            {**environment, "python": ""},
        ):
            with self.subTest(environment=bad):
                self.l["environment"] = bad
                self.assertFalse(self.assess()["passed"])
        self.l["environment"] = environment
        self.assertTrue(
            self.assess()["passed"],
            "Unavailable hardware explicitly represented by null is valid",
        )

    def test_latency_timestamps_must_be_utc_and_ordered(self):
        for started, finished in (
            (None, "2026-10-18T12:00:05+00:00"),
            ("2026-10-18T12:00:00", "2026-10-18T12:00:05"),
            ("2026-10-18T12:00:00+03:00", "2026-10-18T12:00:05+03:00"),
            ("2026-10-18T12:00:06+00:00", "2026-10-18T12:00:05+00:00"),
        ):
            with self.subTest(started=started, finished=finished):
                self.l.update(started_at=started, finished_at=finished)
                self.assertFalse(self.assess()["passed"])

    def test_empty_query_and_denominator_forgery_rejected(self):
        self.q["quality"]["splits"]["held_out"]["answerable_queries"] = 8
        self.assertFalse(self.assess()["passed"])

    def test_duplicate_contract_check_cannot_replace_missing_check(self):
        self.q["graph"]["checks"][-1] = self.q["graph"]["checks"][0]
        self.assertFalse(self.assess()["passed"])

    def test_latency_sample_and_summary_inconsistency_rejected(self):
        name = next(iter(self.l["latency"]["series"]))
        for mutate in (
            lambda s: s.update(p95_ns=18),
            lambda s: s["samples"][0].update(success=False),
            lambda s: s.update(cleanup_error="failed"),
            lambda s: s.update(unmeasured=1),
            lambda s: s["samples"][0].update(elapsed_ns=float("nan")),
        ):
            original = copy.deepcopy(self.l["latency"]["series"][name])
            mutate(self.l["latency"]["series"][name])
            self.assertFalse(self.assess()["passed"])
            self.l["latency"]["series"][name] = original

    def test_recomputed_latency_above_hard_ceiling_rejected(self):
        name = "mcp_persistent_after_warmup"
        series = self.l["latency"]["series"][name]
        for sample in series["samples"]:
            sample["elapsed_ns"] = 500_000_001
        series.update(p50_ns=500_000_001, p95_ns=500_000_001)
        self.assertFalse(self.assess()["passed"])

    def test_debug_and_services_fail_build_contract(self):
        self.q["binary"]["build_info"]["release"] = False
        self.assertFalse(self.assess()["passed"])

    def test_unknown_report_shape_fails_closed(self):
        self.assertFalse(gate.assess({}, {}, self.policy)["passed"])
        self.assertFalse(gate.assess(self.q, self.l, {})["passed"])

    def test_setup_failure_is_saved_without_modifying_policy(self):
        baseline = gate.DEFAULT_BASELINE.read_bytes()
        with tempfile.TemporaryDirectory() as scratch:
            report_path = Path(scratch) / "gate.json"
            with mock.patch.object(
                gate.quality, "evaluate", side_effect=RuntimeError("fixture failure")
            ):
                result = gate.evaluate("unused", report_path)
            self.assertFalse(result["passed"])
            self.assertEqual(json.loads(report_path.read_bytes()), result)
        self.assertEqual(gate.DEFAULT_BASELINE.read_bytes(), baseline)

    def test_completed_quality_report_survives_later_latency_exception(self):
        with tempfile.TemporaryDirectory() as scratch:
            report_path = Path(scratch) / "gate.json"
            with (
                mock.patch.object(gate.quality, "evaluate", return_value=self.q),
                mock.patch.object(
                    gate.latency,
                    "evaluate",
                    side_effect=RuntimeError("latency unavailable"),
                ),
            ):
                result = gate.evaluate("unused", report_path)
            self.assertFalse(result["passed"])
            self.assertEqual(result["quality_report"], self.q)
            self.assertEqual(
                json.loads(report_path.read_bytes())["quality_report"], self.q
            )


if __name__ == "__main__":
    unittest.main()

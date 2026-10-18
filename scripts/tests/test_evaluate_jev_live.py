"""Live admission and reporting tests; no provider calls."""

import copy
from datetime import date
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import evaluate_jev_live as live


class LiveEvaluationTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="jev-admission-tests-")
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.report = self.root / "report.json"
        self.binary = self.root / "fixture-binary"
        self.binary.write_bytes(b"fixture binary")
        self.questions = json.loads((live.FIXTURES / "questions.json").read_bytes())
        self.states = {}
        self.runs = []
        self.failure = None
        self.secret = "private-test-key-never-publish"
        owner = self

        class FakeCli:
            def __init__(self, binary, home, *args, **kwargs):
                self.home = home
                self.binary = binary
                self.state = owner.states.setdefault(str(home), {})

            def run(self, *args):
                if args[0] == "capture":
                    record = json.loads(Path(args[2]).read_bytes())
                    record.update(schema_version=1, revision=1, content_sha256="a" * 64)
                    self.state["record"] = record
                    return {"created": True, "record": copy.deepcopy(record)}
                operation = args[2:]
                if operation[:2] == ("classify", "enqueue"):
                    record = self.state["record"]
                    state = {
                        "candidate": {
                            k: record[k] for k in ("title", "body", "kind", "authority")
                        },
                        "input_truncated": False,
                    }
                    request = {
                        "model": live.MODEL,
                        "state": state,
                        "questions": owner.questions,
                    }
                    prepared = {
                        "schema_version": 1,
                        "repository": record["repository"],
                        "record_id": record["id"],
                        "revision": 1,
                        "content_sha256": record["content_sha256"],
                        "requested_model": live.MODEL,
                        "state": state,
                        "request_sha256": live.digest(live.canonical(request)),
                    }
                    if owner.failure == "request_hash":
                        prepared["request_sha256"] = "f" * 64
                    self.state["job"] = {
                        "key": "job-key",
                        "status": "pending",
                        "attempts": 0,
                        "prepared": prepared,
                        "unknown_usage_attempts": 0,
                        "known_input_tokens": 0,
                        "known_output_tokens": 0,
                    }
                    return copy.deepcopy(self.state["job"])
                if operation[:2] == ("classify", "run"):
                    owner.runs.append((self.home, args))
                    if owner.failure == "throw":
                        raise ValueError("provider echoed " + owner.secret)
                    job = self.state["job"]
                    job.update(
                        status="completed",
                        attempts=1,
                        known_input_tokens=13,
                        known_output_tokens=5,
                    )
                    if owner.failure == "pending":
                        job.update(
                            status="pending",
                            unknown_usage_attempts=1,
                            known_input_tokens=0,
                            known_output_tokens=0,
                        )
                    if owner.failure == "source_mutation":
                        self.state["record"]["authority"] = "deprecated"
                    if owner.failure == "token_overflow":
                        job["known_input_tokens"] = 65537
                    if owner.failure == "binary_mutation":
                        owner.binary.write_bytes(b"changed binary")
                    return {"attempted": 1}
                if operation[:2] == ("classify", "get"):
                    return copy.deepcopy(self.state["job"])
                if operation[0] == "get":
                    if operation[1].startswith("classification:"):
                        advice = {
                            "model": live.MODEL,
                            "category": "unknown",
                            "probabilities": {
                                c: float(c == "unknown") for c in live.CATEGORIES
                            },
                            "confidence": 0.5,
                            "durable_probability": 0.5,
                            "usage": {"input_tokens": 13, "output_tokens": 5},
                        }
                        if owner.failure == "token_overflow":
                            advice["usage"]["input_tokens"] = 65537
                        return {
                            "kind": "event",
                            "authority": "accepted"
                            if owner.failure == "advice_promotion"
                            else "observation",
                            "links": [
                                {
                                    "target_id": self.state["record"]["id"],
                                    "relation": "derived_from",
                                }
                            ],
                            "metadata": {
                                "prepared": self.state["job"]["prepared"],
                                "advice": advice,
                            },
                        }
                    return copy.deepcopy(self.state["record"])
                raise AssertionError(args)

        self.fake_cli = FakeCli

    def evaluate(self, **kwargs):
        with (
            mock.patch.object(live, "CortexCli", self.fake_cli),
            mock.patch.object(live, "LiveCli", self.fake_cli),
        ):
            return live.evaluate(
                self.binary, self.report, today=date(2026, 10, 18), **kwargs
            )

    def ready(self):
        preview = self.evaluate()
        self.assertTrue(preview["success"])
        return {
            "live": True,
            "reviewed_payload_sha256": preview["payload_sha256"],
            "budget_usd": "0.02",
        }

    def test_preview_never_runs_classification_or_sends_ground_truth(self):
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate()
        self.assertTrue(report["success"])
        self.assertEqual(self.runs, [])
        self.assertEqual(len(report["requests"]), 6)
        for request in report["requests"]:
            self.assertEqual(set(request), {"state", "questions", "model"})
            self.assertEqual(
                set(request["state"]["candidate"]),
                {"title", "body", "kind", "authority"},
            )
        self.assertNotIn("expected_category", self.report.read_text())
        self.assertNotIn("synthetic_response", self.report.read_text())
        self.assertNotIn(self.secret, self.report.read_text())

    def test_budget_hash_and_missing_key_block_before_any_classify_run(self):
        options = self.ready()
        for change in (
            {"budget_usd": "0.001"},
            {"reviewed_payload_sha256": "0" * 64},
            {},
        ):
            with self.subTest(change=change):
                env = {"TYPESAFE_API_KEY": self.secret} if change else {}
                with mock.patch.dict(os.environ, env, clear=True):
                    result = self.evaluate(**(options | change))
                self.assertFalse(result["success"])
                self.assertEqual(self.runs, [])
                self.assertEqual(result["attempted"], 0)

    def test_expired_policy_fails_before_materialization(self):
        with mock.patch.object(live, "prepare_case") as prepare:
            report = live.evaluate(
                "unused", self.report, live=True, today=date(2026, 11, 18)
            )
        prepare.assert_not_called()
        self.assertFalse(report["success"])
        self.assertEqual(report["attempted"], 0)

    def test_expired_policy_still_allows_offline_preview(self):
        with mock.patch.object(live, "CortexCli", self.fake_cli):
            report = live.evaluate(self.binary, self.report, today=date(2027, 1, 1))
        self.assertTrue(report["success"])
        self.assertFalse(report["policy"]["valid_on_run_date"])
        self.assertEqual(self.runs, [])

    def test_production_request_hash_drift_blocks_preview(self):
        self.failure = "request_hash"
        report = self.evaluate()
        self.assertFalse(report["success"])
        self.assertEqual(self.runs, [])

    def test_failures_persist_report_without_echoing_credentials(self):
        options = self.ready()
        self.failure = "throw"
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate(**options)
        self.assertFalse(report["success"])
        self.assertEqual(report["metrics"]["completed_scored"], 0)
        self.assertEqual(report["metrics"]["usage_unknown_cases"], 1)
        self.assertEqual(report["metrics"]["not_attempted"], 5)
        self.assertIsNone(report["metrics"]["accuracy_completed"])
        self.assertEqual(report["cost_reserved_or_estimated_usd"], str(live.RESERVE))
        self.assertEqual(json.loads(self.report.read_bytes()), report)
        self.assertNotIn(self.secret, self.report.read_text())

    def test_pending_jobs_are_not_success_and_keep_full_reservation(self):
        options = self.ready()
        self.failure = "pending"
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate(**options)
        self.assertFalse(report["success"])
        self.assertEqual(report["metrics"]["pending"], 6)
        self.assertEqual(report["metrics"]["completed_scored"], 0)
        self.assertEqual(
            report["cost_reserved_or_estimated_usd"], str(live.RESERVE * 6)
        )

    def test_changed_source_authority_invalidates_completed_case(self):
        options = self.ready()
        self.failure = "source_mutation"
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate(**options)
        self.assertFalse(report["success"])
        self.assertEqual(report["metrics"]["completed_scored"], 0)
        self.assertTrue(all(not c["source_preserved"] for c in report["cases"]))

    def test_completed_unknown_predictions_are_counted_separately(self):
        options = self.ready()
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate(**options)
        self.assertTrue(report["success"])
        self.assertEqual(report["metrics"]["unknown_predictions"], 6)
        self.assertEqual(report["metrics"]["accuracy_completed"], 1 / 6)
        self.assertEqual(len(self.runs), 6)
        self.assertEqual(len({home for home, _ in self.runs}), 6)
        for _, args in self.runs:
            self.assertEqual(args[-4:], ("--max-jobs", "1", "--seconds", "12"))

    def test_only_live_environment_includes_key(self):
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            self.assertNotIn(
                "TYPESAFE_API_KEY", live.CortexCli("bin", self.root).environment()
            )
            env = live.LiveCli("bin", self.root, self.secret).environment()
            self.assertEqual(env["TYPESAFE_API_KEY"], self.secret)

    def test_budget_assumption_and_advice_authority_violations_halt_further_sends(self):
        for failure in ("token_overflow", "advice_promotion"):
            with self.subTest(failure=failure):
                options = self.ready()
                self.runs.clear()
                self.failure = failure
                with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
                    report = self.evaluate(**options)
                self.assertFalse(report["success"])
                self.assertEqual(len(self.runs), 1)
                self.assertEqual(report["metrics"]["not_attempted"], 5)
                if failure == "token_overflow":
                    self.assertEqual(
                        report["cases"][0]["policy_violation"],
                        "pricing_assumption_exceeded",
                    )
                    self.assertGreater(
                        live.Decimal(report["cost_reserved_or_estimated_usd"]),
                        live.RESERVE,
                    )
                self.failure = None

    def test_binary_replacement_after_first_send_prevents_second_send(self):
        options = self.ready()
        self.failure = "binary_mutation"
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": self.secret}):
            report = self.evaluate(**options)
        self.assertFalse(report["success"])
        self.assertEqual(len(self.runs), 1)
        self.assertEqual(report["attempted"], 1)


if __name__ == "__main__":
    unittest.main()

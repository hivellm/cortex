"""Feedback evaluation must fail closed and keep held-out labels out of seeding."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

from scripts import evaluate_local_feedback as evaluation


class FeedbackEvaluationTests(unittest.TestCase):
    def test_p95_nearest_rank_uses_95th_of_100_samples(self):
        result = evaluation.distribution(list(range(1, 101)))
        self.assertEqual(result["p95_ms"], 95)
        self.assertEqual(result["percentile_method"], "nearest_rank_ceil_p_times_n")

    def test_held_out_rejected_before_any_transport_or_label_read(self):
        session = mock.Mock()
        with self.assertRaisesRegex(evaluation.base.EvaluationError, "held-out"):
            evaluation.seed_query(session, {"split": "held_out"}, {})
        session.call.assert_not_called()

    def test_fixture_lock_rejects_changed_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in evaluation.LOCKS:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(
                    (evaluation.ROOT / "crates/cortex-local" / name).read_bytes()
                )
            evaluation.verify_locks(root)
            corpus = root / "tests/fixtures/eval-v1/corpus.json"
            corpus.write_bytes(corpus.read_bytes() + b" ")
            with self.assertRaisesRegex(evaluation.base.EvaluationError, "frozen"):
                evaluation.verify_locks(root)

    def test_failed_real_process_never_becomes_empty_success(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "report.json"
            args = [
                "evaluate_local_feedback",
                "--binary",
                sys.executable,
                "--output",
                str(output),
            ]
            with mock.patch.object(sys, "argv", args):
                self.assertEqual(evaluation.main(), 1)
            result = json.loads(output.read_text())
            self.assertFalse(result["passed"])
            self.assertFalse(result["natural_language_ready"])
            self.assertNotIn("queries", result)

    def test_transport_failure_during_seed_propagates(self):
        session = mock.Mock()
        session.call.side_effect = RuntimeError("broken real transport")
        query = {
            "split": "development",
            "id": "d1",
            "expected_keys": [],
            "repository": "Atlas",
            "text": "auth",
        }
        with self.assertRaisesRegex(RuntimeError, "broken real transport"):
            evaluation.seed_query(session, query, {"records": {}})

    def test_projection_keeps_scores_and_canonical_evidence(self):
        before = [
            {
                "score": 1.0,
                "record": {"id": "x", "revision": 1},
                "feedback": {"adjustment": 0},
            }
        ]
        after = evaluation.projection(before)
        self.assertEqual(after, [{"score": 1.0, "record": {"id": "x", "revision": 1}}])
        self.assertIn("feedback", before[0])


if __name__ == "__main__":
    unittest.main()

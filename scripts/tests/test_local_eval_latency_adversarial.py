"""Independent latency accounting checks; no real timing claims or providers."""

import importlib.util
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location(
    "latency_adversarial", SCRIPTS / "evaluate_local_latency.py"
)
latency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(latency)


class AccountingTests(unittest.TestCase):
    def test_nearest_rank_percentiles_keep_original_samples(self):
        samples = [
            {"ordinal": n, "success": True, "elapsed_ns": n} for n in range(20, 0, -1)
        ]
        result = latency.summarize(samples, 20)
        self.assertEqual(result["p50_ns"], 10)
        self.assertEqual(result["p95_ns"], 19)
        self.assertEqual(result["samples"], samples)

    def test_failed_slow_samples_reported_without_distorting_success_quantiles(self):
        samples = [
            {"ordinal": 1, "success": True, "elapsed_ns": 10},
            {"ordinal": 2, "success": False, "elapsed_ns": 900000},
        ]
        result = latency.summarize(samples, 3)
        self.assertEqual(
            (result["successes"], result["failures"], result["unmeasured"]), (1, 1, 1)
        )
        self.assertEqual(result["p95_ns"], 10)
        self.assertEqual(result["samples"][1]["elapsed_ns"], 900000)

    def test_all_failures_have_no_success_percentile(self):
        result = latency.summarize(
            [{"ordinal": 1, "success": False, "elapsed_ns": 999}], 1
        )
        self.assertIsNone(result["p50_ns"])
        self.assertIsNone(result["p95_ns"])

    def test_collect_continues_after_error_and_keeps_failed_elapsed_time(self):
        ticks = iter([0, 10, 20, 70, 80, 100])

        def operation(index):
            if index == 1:
                raise RuntimeError("fictional operation failed")

        result = latency.collect(operation, 3, clock=lambda: next(ticks))
        self.assertEqual([s["elapsed_ns"] for s in result["samples"]], [10, 50, 20])
        self.assertEqual([s["success"] for s in result["samples"]], [True, False, True])
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["unmeasured"], 0)

    def test_cleanup_failure_invalidates_sample_without_erasing_it(self):
        ticks = iter([10, 30])

        def cleanup():
            raise RuntimeError("cleanup failed")

        result = latency.collect(
            lambda _: None, 1, cleanup=cleanup, clock=lambda: next(ticks)
        )
        self.assertEqual(result["measured"], 1)
        self.assertEqual(result["samples"][0]["elapsed_ns"], 20)
        self.assertEqual(result["successes"], 0)
        self.assertEqual(result["failures"], 1)


class LatencyFailurePreservationTests(unittest.TestCase):
    def exercise(self, *, cleanup_failure=False, fail_open=False, state_drift=False):
        with tempfile.TemporaryDirectory(prefix="latency-independent-") as scratch:
            root = Path(scratch)
            home = root / "home"
            original = {"id": "fictional", "repository": "Atlas", "body": "original"}
            baseline = [{"record": original}]
            built = {
                "cli": SimpleNamespace(home=home),
                "records": {"fixture": original},
                "sources": root / "sources",
                "report": {"source_hashes": {}},
            }
            fixtures = {"manifest": {"records": {}}}
            instances = []

            class FakeMcp:
                def __init__(self, *_args, **_kwargs):
                    instances.append(self)
                    self.warm = len(instances) == 3

                def __enter__(self):
                    return self

                def __exit__(self, *_args):
                    self.close()

                def close(self):
                    if cleanup_failure and self.warm:
                        raise RuntimeError("persistent cleanup failed")

                def call(self, *_args, **_kwargs):
                    return baseline

            def run(*args):
                if "get" in args:
                    return dict(original, body="changed") if state_drift else original
                return baseline

            cli = SimpleNamespace(home=home, run=run)

            def hook(_binary, _home, _payload, event):
                return (
                    {}
                    if fail_open
                    else {
                        "hookSpecificOutput": {
                            "hookEventName": event,
                            "additionalContext": "expected context",
                        }
                    }
                )

            with (
                mock.patch.object(latency, "CortexCli", return_value=cli),
                mock.patch.object(latency, "CortexMcp", FakeMcp),
                mock.patch.object(latency, "binary_hash", return_value="unchanged"),
                mock.patch.object(latency, "validate_scope_hits"),
                mock.patch.object(latency, "assert_preserved"),
                mock.patch.object(
                    latency,
                    "hook_expected",
                    return_value={
                        "SessionStart": "expected context",
                        "UserPromptSubmit": "expected context",
                    },
                ),
                mock.patch.object(latency, "run_hook", side_effect=hook),
            ):
                return latency.measure_latency(
                    root / "binary", fixtures, built, samples=2
                )

    def test_persistent_cleanup_failure_preserves_completed_samples(self):
        result = self.exercise(cleanup_failure=True)
        series = result["series"]["mcp_persistent_after_warmup"]
        self.assertEqual(series["measured"], 2)
        self.assertEqual(len(series["samples"]), 2)
        self.assertFalse(result["passed"])

    def test_fail_open_hook_never_accepted_as_fast_success(self):
        result = self.exercise(fail_open=True)
        self.assertFalse(result["passed"])
        self.assertEqual(
            result["series"]["cli_new_process_warmed_store"]["successes"], 2
        )
        for event in ("SessionStart", "UserPromptSubmit"):
            series = result["series"]["hook_" + event + "_new_process_warmed_store"]
            self.assertEqual(series["successes"], 0)
            self.assertEqual(series["planned"], 2)

    def test_canonical_state_drift_invalidates_otherwise_successful_samples(self):
        result = self.exercise(state_drift=True)
        self.assertFalse(result["passed"])
        self.assertFalse(result["binary_sources_and_original_records_preserved"])
        self.assertTrue(
            all(series["successes"] == 2 for series in result["series"].values())
        )

    def test_clean_control_passes_with_five_complete_series(self):
        result = self.exercise()
        self.assertTrue(result["passed"])
        self.assertEqual(len(result["series"]), 5)
        self.assertTrue(result["binary_sources_and_original_records_preserved"])


if __name__ == "__main__":
    unittest.main()

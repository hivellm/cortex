"""Latency reporting and bounded direct-hook transport tests."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from scripts import evaluate_local_latency as latency


class LatencyTests(unittest.TestCase):
    def test_nearest_rank_excludes_failed_duration_without_hiding_failure(self):
        samples = [{"elapsed_ns": n, "success": True} for n in range(1, 21)]
        samples.append({"elapsed_ns": 99999, "success": False})
        report = latency.summarize(samples, 22)
        self.assertEqual(report["p50_ns"], 10)
        self.assertEqual(report["p95_ns"], 19)
        self.assertEqual(report["failures"], 1)
        self.assertEqual(report["unmeasured"], 1)
        self.assertEqual(len(report["samples"]), 21)

    def test_all_failed_has_no_success_latency(self):
        report = latency.summarize([{"elapsed_ns": 10, "success": False}], 1)
        self.assertIsNone(report["p50_ns"])
        self.assertIsNone(report["p95_ns"])

    def test_cleanup_occurs_after_timing_and_failed_sample_retained(self):
        trace = []
        times = iter([100, 120, 500, 550])

        def clock():
            value = next(times)
            trace.append(value)
            return value

        def operation(ordinal):
            trace.append("operation")
            if ordinal == 1:
                raise ValueError("sensitive diagnostic")

        result = latency.collect(
            operation, 2, cleanup=lambda: trace.append("cleanup"), clock=clock
        )
        self.assertEqual(
            trace, [100, "operation", 120, "cleanup", 500, "operation", 550, "cleanup"]
        )
        self.assertEqual([s["elapsed_ns"] for s in result["samples"]], [20, 50])
        self.assertEqual(result["failures"], 1)
        self.assertNotIn("sensitive diagnostic", json.dumps(result))

    def controlled_hook(self, code, **kwargs):
        original = subprocess.Popen

        def adapted(_command, **options):
            return original([sys.executable, "-c", code], **options)

        with tempfile.TemporaryDirectory() as scratch:
            with mock.patch.object(latency.subprocess, "Popen", adapted):
                return latency.run_hook(
                    Path(sys.executable),
                    Path(scratch) / "home",
                    {"cwd": scratch},
                    "SessionStart",
                    **kwargs,
                )

    def test_real_hook_transport_passes_stdin_and_json_without_shell(self):
        result = self.controlled_hook(
            "import json,sys; p=json.load(sys.stdin); print(json.dumps({'cwd':p['cwd']}))"
        )
        self.assertTrue(Path(result["cwd"]).is_absolute())

    def test_real_hook_transport_deadline_and_output_limit(self):
        for code, options in (
            ("import time; time.sleep(5)", {"timeout": 0.05}),
            (
                "import sys; sys.stdout.write('x'*10000); sys.stdout.flush()",
                {"output_limit": 100},
            ),
        ):
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.controlled_hook(code, **options)

    def test_invalid_sample_budget_persists_failure_report_before_build(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "report.json"
            with mock.patch.object(latency, "build_home") as build:
                report = latency.evaluate("unused", path, samples=101)
            build.assert_not_called()
            self.assertFalse(report["passed"])
            self.assertEqual(json.loads(path.read_bytes()), report)
            self.assertIn("started_at", report)
            self.assertIn("finished_at", report)


if __name__ == "__main__":
    unittest.main()

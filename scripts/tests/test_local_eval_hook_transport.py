"""Real child-process checks for hook transport failure boundaries."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from scripts import evaluate_local_latency as latency


class HookTransportTests(unittest.TestCase):
    def controlled_hook(self, code, **kwargs):
        original = subprocess.Popen
        children = []

        def adapted(_command, **options):
            process = original([sys.executable, "-c", code], **options)
            children.append(process)
            return process

        with tempfile.TemporaryDirectory() as scratch:
            try:
                with mock.patch.object(latency.subprocess, "Popen", adapted):
                    return latency.run_hook(
                        Path(sys.executable),
                        Path(scratch) / "home",
                        {"cwd": scratch},
                        "SessionStart",
                        **kwargs,
                    )
            finally:
                for process in children:
                    self.assertIsNotNone(process.poll(), "Hook child was not reaped")

    def test_stderr_overflow_rejects_valid_stdout(self):
        with self.assertRaisesRegex(ValueError, "Hook transport failed"):
            self.controlled_hook(
                "import sys; sys.stdin.read(); print('{}', flush=True); "
                "sys.stderr.write('x' * 10000); sys.stderr.flush()",
                output_limit=100,
            )

    def test_nonzero_exit_rejects_valid_json(self):
        with self.assertRaisesRegex(ValueError, "Hook process failed"):
            self.controlled_hook(
                "import sys; sys.stdin.read(); print('{}'); sys.exit(7)"
            )

    def test_malformed_or_ambiguous_json_rejected(self):
        for output in ("not JSON", "{}\n{}", '{"context":1,"context":2}'):
            with self.subTest(output=output), self.assertRaises(ValueError):
                self.controlled_hook(
                    "import sys; sys.stdin.read(); print(" + repr(output) + ")"
                )

    def test_large_input_fails_before_process_creation(self):
        with tempfile.TemporaryDirectory() as scratch:
            with mock.patch.object(latency.subprocess, "Popen") as spawn:
                with self.assertRaisesRegex(ValueError, "input exceeds bound"):
                    latency.run_hook(
                        Path(sys.executable),
                        Path(scratch) / "home",
                        {"prompt": "x" * 4097},
                        "SessionStart",
                    )
                spawn.assert_not_called()

    def test_offline_hook_child_does_not_receive_provider_credential(self):
        with mock.patch.dict(os.environ, {"TYPESAFE_API_KEY": "fixture-only-key"}):
            result = self.controlled_hook(
                "import os,json,sys; sys.stdin.read(); "
                "print(json.dumps({'credential_present':'TYPESAFE_API_KEY' in os.environ}))"
            )
        self.assertEqual(result, {"credential_present": False})


if __name__ == "__main__":
    unittest.main()

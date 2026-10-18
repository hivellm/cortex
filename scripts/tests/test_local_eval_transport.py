"""Persistent MCP protocol boundaries using controlled and real subprocesses."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

MODULE = Path(__file__).resolve().parents[1] / "local_eval_transport.py"
spec = importlib.util.spec_from_file_location("local_eval_transport", MODULE)
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)

SERVER = """
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = {'protocolVersion': '2024-11-05'}
    else:
        MODE
        result = {'content':[{'type':'text','text':json.dumps({'repository': 'Atlas'})}]}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"""


class ControlledTransportTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="cortex-mcp-boundary-")
        self.addCleanup(self.scratch.cleanup)
        self.home = Path(self.scratch.name) / "home"

    def client(self, action="pass", *, on_eof="pass", **options):
        real_popen = subprocess.Popen
        code = SERVER.replace("MODE", action) + "\n" + on_eof + "\n"

        def adapted(command, **kwargs):
            self.assertEqual(command[-1], "mcp")
            self.assertEqual(command[1:3], ["--home", str(self.home)])
            self.assertNotIn("TYPESAFE_API_KEY", kwargs["env"])
            return real_popen([sys.executable, "-c", code], **kwargs)

        patch = mock.patch.object(transport.subprocess, "Popen", side_effect=adapted)
        patch.start()
        self.addCleanup(patch.stop)
        return transport.CortexMcp(sys.executable, self.home, **options)

    def test_multiple_calls_keep_one_process_and_increment_ids(self):
        with self.client() as client:
            pid = client.process.pid
            self.assertEqual(client.call("fixture", {}), {"repository": "Atlas"})
            self.assertEqual(client.call("fixture", {}), {"repository": "Atlas"})
            self.assertEqual(client.process.pid, pid)
            self.assertEqual(client._sequence, 3)
        self.assertIsNotNone(client.process.returncode)

    def test_response_id_mismatch(self):
        with self.client("request['id'] += 1") as client:
            with self.assertRaisesRegex(transport.TransportError, "ID mismatch"):
                client.request("fixture")

    def test_close_allows_eof_cleanup_even_after_stdout_closes(self):
        cleanup = (
            "import os\n"
            "from pathlib import Path\n"
            "os.close(1)\n"
            "time.sleep(0.05)\n"
            "Path('shutdown.complete').write_text('EOF', encoding='ascii')\n"
            "os._exit(0)"
        )
        with self.client(on_eof=cleanup) as client:
            self.assertEqual(client.call("fixture", {}), {"repository": "Atlas"})
        marker = self.home.parent / "shutdown.complete"
        self.assertTrue(marker.exists(), "normal shutdown cleanup was interrupted")
        self.assertEqual(marker.read_text(), "EOF")
        self.assertEqual(client.process.returncode, 0)
        self.assertIsNone(client._failure)

    def test_close_reaps_child_that_refuses_eof_shutdown(self):
        with self.client(on_eof="time.sleep(10)", timeout=0.3) as client:
            self.assertEqual(client.call("fixture", {}), {"repository": "Atlas"})
        self.assertIsNotNone(client.process.returncode)
        self.assertNotEqual(client.process.returncode, 0)
        self.assertTrue(all(not thread.is_alive() for thread in client._threads))

    def test_response_deadline_kills_process(self):
        with self.client("time.sleep(5)", timeout=0.3) as client:
            with self.assertRaisesRegex(transport.TransportError, "deadline exceeded"):
                client.request("fixture")
        self.assertIsNotNone(client.process.returncode)

    def test_stdout_overflow(self):
        with self.client(
            "print('x'*20000, flush=True); continue", output_limit=1024
        ) as client:
            with self.assertRaisesRegex(transport.TransportError, "limit exceeded"):
                client.request("fixture")

    def test_stderr_overflow(self):
        with self.client(
            "sys.stderr.write('x'*20000); sys.stderr.flush(); time.sleep(1)",
            output_limit=1024,
        ) as client:
            with self.assertRaisesRegex(transport.TransportError, "stderr limit"):
                client.request("fixture")

    def test_stdin_overflow(self):
        with self.client(output_limit=1024) as client:
            with self.assertRaisesRegex(
                transport.TransportError, "stdin message limit"
            ):
                client.request("fixture", {"large": "x" * 2000})

    def test_jsonrpc_error_does_not_leak_server_message(self):
        action = "print(json.dumps({'jsonrpc':'2.0','id':request['id'],'error':{'message':'secret-fixture'}}), flush=True); continue"
        with self.client(action) as client:
            with self.assertRaises(transport.TransportError) as raised:
                client.request("fixture")
            self.assertNotIn("secret-fixture", str(raised.exception))

    def test_tool_error_raw_available_but_call_raises(self):
        action = "print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'isError':True,'content':[{'type':'text','text':'fixture error'}]}}), flush=True); continue"
        with self.client(action) as client:
            self.assertTrue(client.call_raw("fixture", {})["isError"])
            with self.assertRaisesRegex(
                transport.TransportError, "tool returned an error"
            ):
                client.call("fixture", {})

    def test_process_exit_is_explicit_failure(self):
        with self.client("sys.exit(7)") as client:
            with self.assertRaisesRegex(
                transport.TransportError, "closed|write failed"
            ):
                client.request("fixture")

    def test_invalid_tool_json(self):
        action = "print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'content':[{'type':'text','text':'not-json'}]}}), flush=True); continue"
        with self.client(action) as client:
            with self.assertRaisesRegex(transport.TransportError, "not valid JSON"):
                client.call("fixture", {})


@unittest.skipUnless(
    os.environ.get("CORTEX_EVAL_BINARY"),
    "set CORTEX_EVAL_BINARY for real MCP integration",
)
class RealMcpTests(unittest.TestCase):
    def test_real_capture_get_and_exact_scope(self):
        with tempfile.TemporaryDirectory(prefix="cortex-mcp-real-") as scratch:
            with transport.CortexMcp(
                os.environ["CORTEX_EVAL_BINARY"], Path(scratch) / "home"
            ) as client:
                self.assertEqual(len(client.list_tools()["tools"]), 11)
                captured = client.call(
                    "cortex_capture_memory",
                    {
                        "repository": "Atlas",
                        "title": "Fictional scope check",
                        "body": "Only Atlas owns this fictional record.",
                        "kind": "decision",
                        "authority": "accepted",
                        "source": {
                            "kind": "fixture",
                            "locator": "fiction://transport-test",
                        },
                    },
                )
                record = captured["record"]
                found = client.call(
                    "cortex_get", {"repository": "Atlas", "id": record["id"]}
                )
                self.assertEqual(found, record)
                wrong = client.call_raw(
                    "cortex_get", {"repository": "atlas", "id": record["id"]}
                )
                if wrong.get("isError"):
                    self.assertTrue(wrong["isError"])
                else:
                    self.assertIsNone(json.loads(wrong["content"][0]["text"]))


if __name__ == "__main__":
    unittest.main()

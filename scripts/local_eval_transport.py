"""Bounded, persistent MCP stdio transport for isolated local evaluations."""

from __future__ import annotations

import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time


class TransportError(RuntimeError):
    """Protocol or process failure; diagnostics never include server payloads."""


class CortexMcp:
    def __init__(self, binary, home, timeout=10, output_limit=4 * 1024 * 1024):
        if timeout <= 0 or output_limit <= 0:
            raise ValueError("timeout and output_limit must be positive")
        self.binary = Path(binary).resolve()
        self.home = Path(home).resolve()
        self.timeout = timeout
        self.output_limit = output_limit
        self.process = None
        self._threads = []
        self._messages = queue.Queue(maxsize=16)
        self._failure = None
        self._sequence = 0
        self._lock = threading.Lock()
        self._closing = threading.Event()

    def __enter__(self):
        if self.process is not None:
            raise TransportError("transport cannot be opened twice")
        env = os.environ.copy()
        env["CORTEX_HOME"] = str(self.home)
        env.pop("TYPESAFE_API_KEY", None)
        self.process = subprocess.Popen(
            [str(self.binary), "--home", str(self.home), "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=self.home.parent,
            env=env,
        )
        self._threads = [
            threading.Thread(target=self._read_stdout, daemon=True),
            threading.Thread(target=self._read_stderr, daemon=True),
        ]
        for thread in self._threads:
            thread.start()
        try:
            initialized = self.request(
                "initialize",
                {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "cortex-public-eval", "version": "1"},
                },
            )
            if (
                not isinstance(initialized, dict)
                or initialized.get("protocolVersion") != "2024-11-05"
            ):
                raise TransportError("unsupported initialize response")
            self._send(
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                time.monotonic() + self.timeout,
            )
            return self
        except BaseException:
            self.close()
            raise

    def _fail(self, message):
        if self._failure is None:
            self._failure = message
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
        try:
            self._messages.put_nowait(None)
        except queue.Full:
            pass

    def _read_stdout(self):
        pending = bytearray()
        try:
            while chunk := os.read(self.process.stdout.fileno(), 4096):
                pending.extend(chunk)
                while b"\n" in pending:
                    line, _, rest = pending.partition(b"\n")
                    pending = bytearray(rest)
                    if len(line) > self.output_limit:
                        self._fail("MCP stdout message limit exceeded")
                        return
                    try:
                        self._messages.put_nowait(bytes(line))
                    except queue.Full:
                        self._fail("MCP response queue limit exceeded")
                        return
                if len(pending) > self.output_limit:
                    self._fail("MCP stdout message limit exceeded")
                    return
            if not self._closing.is_set():
                self._fail("MCP stdout closed before a response")
        except OSError:
            self._fail("MCP stdout read failed")

    def _read_stderr(self):
        size = 0
        try:
            while chunk := os.read(self.process.stderr.fileno(), 4096):
                size += len(chunk)
                if size > self.output_limit:
                    self._fail("MCP stderr limit exceeded")
                    return
        except OSError:
            self._fail("MCP stderr read failed")

    def _send(self, message, deadline):
        if self.process is None or self._failure:
            raise TransportError(self._failure or "MCP transport is not open")
        data = (json.dumps(message, separators=(",", ":")) + "\n").encode()
        if len(data) > self.output_limit:
            raise TransportError("MCP stdin message limit exceeded")
        failed = threading.Event()

        def write():
            try:
                self.process.stdin.write(data)
                self.process.stdin.flush()
            except (BrokenPipeError, OSError, ValueError):
                failed.set()

        writer = threading.Thread(target=write, daemon=True)
        writer.start()
        writer.join(max(0, deadline - time.monotonic()))
        if writer.is_alive():
            self._fail("MCP stdin deadline exceeded")
            writer.join(timeout=1)
            raise TransportError("MCP stdin deadline exceeded")
        if failed.is_set():
            raise TransportError("MCP stdin write failed")

    def request(self, method, params=None):
        with self._lock:
            self._sequence += 1
            identity = self._sequence
            deadline = time.monotonic() + self.timeout
            message = {"jsonrpc": "2.0", "id": identity, "method": method}
            if params is not None:
                message["params"] = params
            self._send(message, deadline)
            try:
                raw = self._messages.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty as exc:
                self._fail("MCP response deadline exceeded")
                raise TransportError("MCP response deadline exceeded") from exc
            if self._failure or raw is None:
                raise TransportError(self._failure or "MCP response unavailable")
            try:
                response = json.loads(raw)
            except (ValueError, UnicodeError) as exc:
                raise TransportError("MCP response is not valid JSON") from exc
            if not isinstance(response, dict) or response.get("jsonrpc") != "2.0":
                raise TransportError("invalid MCP JSON-RPC envelope")
            if type(response.get("id")) is not int or response["id"] != identity:
                raise TransportError("MCP response ID mismatch")
            if "error" in response:
                raise TransportError("MCP JSON-RPC error")
            if "result" not in response:
                raise TransportError("MCP response lacks result")
            return response["result"]

    def list_tools(self):
        return self.request("tools/list")

    def call_raw(self, name, args):
        result = self.request("tools/call", {"name": name, "arguments": args})
        if not isinstance(result, dict):
            raise TransportError("MCP tool response is not an object")
        return result

    def call(self, name, args):
        result = self.call_raw(name, args)
        if result.get("isError"):
            raise TransportError("MCP tool returned an error")
        content = result.get("content")
        if (
            not isinstance(content, list)
            or len(content) != 1
            or not isinstance(content[0], dict)
            or content[0].get("type") != "text"
            or not isinstance(content[0].get("text"), str)
        ):
            raise TransportError("MCP tool lacks one JSON text result")
        try:
            return json.loads(content[0]["text"])
        except (ValueError, UnicodeError) as exc:
            raise TransportError("MCP tool text is not valid JSON") from exc

    def close(self):
        if self.process is None:
            return
        self._closing.set()
        if self.process.poll() is None:
            # EOF lets the store close SQLite and its files before temporary-home cleanup.
            try:
                self.process.stdin.close()
            except (BrokenPipeError, OSError):
                pass
            try:
                self.process.wait(timeout=min(self.timeout, 1.0))
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process.wait(timeout=self.timeout)
        for thread in self._threads:
            thread.join(timeout=1)
        if any(thread.is_alive() for thread in self._threads):
            raise TransportError("MCP output readers did not terminate")
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            stream.close()

    def __exit__(self, *_args):
        self.close()

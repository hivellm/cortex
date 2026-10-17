#!/usr/bin/env python3
"""Measure isolated daily API/adapter processes on Windows; no model calls.

Requires --run. Keeps synthetic artifacts, never reads operator configuration or
credential stores. Stops only children created here. Uses Python stdlib and Win32.
"""

import argparse
import concurrent.futures
import ctypes
from ctypes import wintypes as wt
import datetime
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid


class FileTime(ctypes.Structure):
    _fields_ = [("low", wt.DWORD), ("high", wt.DWORD)]

    def seconds(self):
        return ((self.high << 32) | self.low) / 10_000_000


class IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_ulonglong) for name in (
        "read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("page_fault_count", wt.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "peak_working_set_bytes", "working_set_bytes", "quota_peak_paged_pool",
            "quota_paged_pool", "quota_peak_nonpaged_pool", "quota_nonpaged_pool",
            "pagefile_usage", "peak_pagefile_usage", "private_bytes")]


class ProcessEntry(ctypes.Structure):
    _fields_ = [("dwSize", wt.DWORD), ("cntUsage", wt.DWORD), ("pid", wt.DWORD),
                ("default_heap", ctypes.c_size_t), ("module_id", wt.DWORD),
                ("threads", wt.DWORD), ("parent_pid", wt.DWORD), ("base_priority", wt.LONG),
                ("flags", wt.DWORD), ("exe", wt.WCHAR * 260)]


class WindowsMetrics:
    def __init__(self):
        if os.name != "nt":
            raise ValueError("This probe requires Windows native process counters")
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.psapi = ctypes.WinDLL("psapi", use_last_error=True)
        self.kernel.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
        self.kernel.OpenProcess.restype = wt.HANDLE
        self.kernel.CloseHandle.argtypes = [wt.HANDLE]
        self.kernel.CloseHandle.restype = wt.BOOL
        self.kernel.GetProcessTimes.argtypes = [wt.HANDLE] + [ctypes.POINTER(FileTime)] * 4
        self.kernel.GetProcessTimes.restype = wt.BOOL
        self.kernel.GetSystemTimes.argtypes = [ctypes.POINTER(FileTime)] * 3
        self.kernel.GetSystemTimes.restype = wt.BOOL
        self.kernel.GetProcessIoCounters.argtypes = [wt.HANDLE, ctypes.POINTER(IoCounters)]
        self.kernel.GetProcessIoCounters.restype = wt.BOOL
        self.psapi.GetProcessMemoryInfo.argtypes = [wt.HANDLE, ctypes.POINTER(MemoryCounters), wt.DWORD]
        self.psapi.GetProcessMemoryInfo.restype = wt.BOOL
        self.kernel.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
        self.kernel.CreateToolhelp32Snapshot.restype = wt.HANDLE
        for name in ("Process32FirstW", "Process32NextW"):
            function = getattr(self.kernel, name)
            function.argtypes = [wt.HANDLE, ctypes.POINTER(ProcessEntry)]
            function.restype = wt.BOOL

    @staticmethod
    def check(value):
        if not value:
            raise ctypes.WinError(ctypes.get_last_error())

    def processes(self):
        snapshot = self.kernel.CreateToolhelp32Snapshot(2, 0)
        if snapshot == ctypes.c_void_p(-1).value:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            entry = ProcessEntry()
            entry.dwSize = ctypes.sizeof(entry)
            result = {}
            exists = self.kernel.Process32FirstW(snapshot, ctypes.byref(entry))
            while exists:
                result[entry.pid] = {"pid": entry.pid, "name": entry.exe, "threads": entry.threads}
                exists = self.kernel.Process32NextW(snapshot, ctypes.byref(entry))
            error = ctypes.get_last_error()
            if error != 18:  # ERROR_NO_MORE_FILES
                raise ctypes.WinError(error)
            return result
        finally:
            self.kernel.CloseHandle(snapshot)

    def process(self, pid, inventory):
        handle = self.kernel.OpenProcess(0x1000 | 0x0010, False, pid)
        self.check(handle)
        try:
            created, exited, kernel, user = (FileTime() for _ in range(4))
            self.check(self.kernel.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited),
                                                  ctypes.byref(kernel), ctypes.byref(user)))
            io = IoCounters()
            self.check(self.kernel.GetProcessIoCounters(handle, ctypes.byref(io)))
            memory = MemoryCounters()
            memory.cb = ctypes.sizeof(memory)
            self.check(self.psapi.GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb))
            return {**inventory[pid], "creation_filetime_seconds": created.seconds(),
                    "cpu_seconds": kernel.seconds() + user.seconds(),
                    "kernel_seconds": kernel.seconds(), "user_seconds": user.seconds(),
                    "rss_bytes": memory.working_set_bytes,
                    "lifetime_peak_rss_bytes": memory.peak_working_set_bytes,
                    "private_bytes": memory.private_bytes,
                    "io": {name: getattr(io, name) for name, _ in io._fields_}}
        finally:
            self.kernel.CloseHandle(handle)

    def host(self):
        idle, kernel, user = (FileTime() for _ in range(3))
        self.check(self.kernel.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)))
        return {"idle_seconds": idle.seconds(), "kernel_seconds": kernel.seconds(),
                "user_seconds": user.seconds()}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def clean_env():
    # No inherited credentials, model settings, HOME overrides, or operator Cortex settings.
    return {key: os.environ[key] for key in ("PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT")
            if key in os.environ}


def port():
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def http_request(port_number, path, body=None, token=None, timeout=5):
    connection = http.client.HTTPConnection("127.0.0.1", port_number, timeout=timeout)
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    try:
        connection.request("POST" if body is not None else "GET", path,
                           json.dumps(body) if body is not None else None, headers)
        response = connection.getresponse()
        raw = response.read()
        try:
            result = json.loads(raw)
        except (ValueError, UnicodeError):
            result = {"non_json_response": True}
        return response.status, result
    finally:
        connection.close()


class Fixture:
    def __init__(self, root, api, adapter):
        self.root, self.api_binary, self.adapter_binary = root, api, adapter
        self.api, self.adapter = None, None
        self.logs = []
        self.starts = []
        self.api_port, self.admin_port = port(), port()
        self.repo = "resource-probe-" + uuid.uuid4().hex
        self.binding = r"\\.\pipe\cortex-resource-" + uuid.uuid4().hex
        self.sentinel = socket.socket()
        self.sentinel.bind(("127.0.0.1", 0))
        self.sentinel.listen(128)
        self.sentinel.setblocking(False)
        self.sentinel_url = "http://127.0.0.1:" + str(self.sentinel.getsockname()[1])
        self.env = clean_env()
        self.env.update(CORTEX_CONFIG_FILE=str(root / "runtime.toml"), CORTEX_RUNTIME_PROFILE="daily",
                        CORTEX_DASHBOARD_AUTH="1", RUST_LOG="warn")
        for name in ("VECTORIZER_URL", "NEXUS_URL", "CORTEX_ADAPTER_ADMIN_URL",
                     "CORTEX_CLASSIFIER_WORKER_URL", "CORTEX_EMBEDDER_WORKER_URL",
                     "CORTEX_FULLTEXT_WORKER_URL", "CORTEX_GRAPH_WORKER_URL"):
            self.env[name] = self.sentinel_url
        quote = lambda path: json.dumps(str(path).replace("\\", "/"))
        (root / "archive").mkdir()
        (root / ".rulebook").mkdir()
        (root / "runtime.toml").write_text(f'''[runtime]
profile = "full"
[access_control]
enabled = true
default_level = 0
deny_on_missing_principal = true
[ingestion]
home = {quote(root)}
metadata_db = {quote(root / "metadata.sqlite")}
archive_root = {quote(root / "archive")}
archive_incremental_enabled = false
ingestion_url = "{self.sentinel_url}"
synap_url = "{self.sentinel_url}"
archive_watcher_urls = "{self.sentinel_url}"
[dashboard]
api_keys_db = {quote(root / "api-keys.sqlite")}
relevance_config_path = {quote(root / "relevance.toml")}
api_url = "http://127.0.0.1:{self.api_port}"
synap_url = "{self.sentinel_url}"
query_rewriter = "sonnet"
watch = true
memory_tail = true
archive_refresh_secs = 1
meili_refresh_secs = 1
[embedder]
vectorizer_url = "{self.sentinel_url}"
synap_url = "{self.sentinel_url}"
[meili]
meili_url = "{self.sentinel_url}"
synap_url = "{self.sentinel_url}"
[nexus]
nexus_url = "{self.sentinel_url}"
synap_url = "{self.sentinel_url}"
[reranker]
enabled = true
endpoint = "{self.sentinel_url}"
[canary]
enabled = true
interval_secs = 10
pre_thinking_health_secs = 10
[rulebook]
root = {quote(root / ".rulebook")}
[adapter]
adapter_admin_port = {self.admin_port}
''', encoding="utf-8")
        (root / "adapter.toml").write_text(
            f'[adapter]\nendpoint="{self.sentinel_url}"\napi_endpoint="http://127.0.0.1:{self.api_port}"\n',
            encoding="utf-8")
        issued = subprocess.run([str(api), "admin", "issue-api-key", "--label", "resource-fixture"],
                                env=self.env, cwd=root, capture_output=True, text=True, timeout=20,
                                creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        if issued.returncode:
            raise RuntimeError("Synthetic key issuance failed; credential output withheld")
        self.token = next(line.split(":", 1)[1].strip() for line in issued.stdout.splitlines()
                          if line.startswith("key:"))
        self.env["CORTEX_API_TOKEN"] = self.token

    @staticmethod
    def stop(child):
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)

    def spawn(self, label, command):
        log = (self.root / f"{label}-{len(self.logs)}.log").open("wb")
        self.logs.append(log)
        return subprocess.Popen(command, cwd=self.root, env=self.env, stdin=subprocess.DEVNULL,
                                stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)

    def wait_ready(self, process, port_number, token=None):
        begin = time.perf_counter()
        while time.perf_counter() - begin < 10:
            if process.poll() is not None:
                raise RuntimeError("Fixture child exited; inspect synthetic log files")
            try:
                status, _ = http_request(port_number, "/healthz", token=token, timeout=0.2)
                if status == 200:
                    return (time.perf_counter() - begin) * 1000
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(0.025)
        raise RuntimeError("Fixture startup exceeded 10 seconds")

    def start_api(self):
        self.stop(self.api)
        begin = time.perf_counter()
        self.api = self.spawn("api", [str(self.api_binary), "--bind", f"127.0.0.1:{self.api_port}"])
        self.wait_ready(self.api, self.api_port)
        self.starts.append({"pid": self.api.pid, "startup_ms": (time.perf_counter() - begin) * 1000})

    def start(self):
        self.start_api()
        self.adapter = self.spawn("adapter", [str(self.adapter_binary), "--config",
                                  str(self.root / "adapter.toml"), "--bind", self.binding])
        self.wait_ready(self.adapter, self.admin_port)
        status, value = http_request(self.api_port, "/v1/status", token=self.token)
        if status != 200 or value.get("runtime_profile") != "daily":
            raise RuntimeError("API did not resolve the daily profile")
        denied, _ = http_request(self.api_port, "/v1/memory/query", {"repo": self.repo, "query": "test"})
        if denied != 401:
            raise RuntimeError("Synthetic API authentication is not enforced")
        return value

    def close(self):
        self.stop(self.adapter)
        self.stop(self.api)
        for log in self.logs:
            log.close()
        self.sentinel.close()


def docker_snapshot(root):
    config = root / "docker-cli-empty"
    config.mkdir(exist_ok=True)
    try:
        result = subprocess.run(["docker", "--config", str(config), "--host",
                                 "npipe:////./pipe/docker_engine", "stats", "--no-stream", "--format", "{{json .}}"],
                                capture_output=True, text=True, timeout=15, env=clean_env(),
                                creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        if result.returncode:
            return {"available": False, "reason": "docker stats command failed", "exit_code": result.returncode}
        return {"available": True, "containers": [json.loads(line) for line in result.stdout.splitlines()]}
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        return {"available": False, "reason": type(error).__name__}


def idle_measurement(metrics, fixture, duration):
    inventory = metrics.processes()
    targets = {pid: item for pid, item in inventory.items()
               if item["name"].lower().startswith(("cortex-", "com.docker", "vmmem", "docker desktop"))}
    owned = {fixture.api.pid: "api", fixture.adapter.pid: "adapter"}
    frames = []
    started = time.perf_counter()
    host_before = metrics.host()
    while True:
        sample = {"elapsed_seconds": time.perf_counter() - started, "processes": {}}
        inventory = metrics.processes()
        for pid in targets:
            try:
                sample["processes"][str(pid)] = metrics.process(pid, inventory)
            except (OSError, KeyError) as error:
                sample["processes"][str(pid)] = {"error": type(error).__name__}
        frames.append(sample)
        remaining = duration - (time.perf_counter() - started)
        if remaining <= 0:
            break
        time.sleep(min(1, remaining))
    elapsed = time.perf_counter() - started
    host_after = metrics.host()
    total = sum(host_after[key] - host_before[key] for key in ("kernel_seconds", "user_seconds"))
    idle = host_after["idle_seconds"] - host_before["idle_seconds"]
    summaries = []
    interval = frames[-1]["elapsed_seconds"] - frames[0]["elapsed_seconds"]
    for pid, target in targets.items():
        values = [frame["processes"][str(pid)] for frame in frames]
        summary = {**target, "owned_role": owned.get(pid), "interval_seconds": interval}
        if any("error" in value for value in values):
            summary["unavailable"] = "One or more native samples unavailable; inspect raw frames"
        elif values[0]["creation_filetime_seconds"] != values[-1]["creation_filetime_seconds"]:
            summary["unavailable"] = "PID was reused during interval"
        else:
            before, after = values[0], values[-1]
            cpu = after["cpu_seconds"] - before["cpu_seconds"]
            summary.update(cpu_seconds_delta=cpu, cpu_percent_one_core=100 * cpu / interval,
                           rss_start_bytes=before["rss_bytes"], rss_end_bytes=after["rss_bytes"],
                           sampled_peak_rss_bytes=max(item["rss_bytes"] for item in values),
                           lifetime_peak_rss_bytes=after["lifetime_peak_rss_bytes"],
                           private_end_bytes=after["private_bytes"], threads_start=before["threads"],
                           threads_end=after["threads"], peak_threads=max(item["threads"] for item in values),
                           io_delta={key: after["io"][key] - before["io"][key] for key in before["io"]})
        summaries.append(summary)
    return {"requested_seconds": duration, "elapsed_seconds": elapsed, "sample_interval_seconds": 1,
            "owned_process_count": len(owned), "host_cpu_percent_all_cores": 100 * (total - idle) / total,
            "host_before": host_before, "host_after": host_after, "processes": summaries, "frames": frames}


def summarize(samples):
    latencies = sorted(item["milliseconds"] for item in samples)
    percentile = lambda value: latencies[max(0, math.ceil(value * len(latencies)) - 1)]
    return {"requests": len(samples), "failures": sum(not item["passed"] for item in samples),
            "p50_ms": percentile(0.50), "p95_ms": percentile(0.95), "max_ms": max(latencies),
            "samples": samples}


def timed_operation(fixture, operation, barrier=None):
    identifier = "capture-" + uuid.uuid4().hex
    if operation == "capture":
        body = {"repo": fixture.repo, "kind": "decision", "memory_id": identifier,
                "body": "Synthetic isolated latency capture " + identifier}
        path, expected = "/v1/ingest", 202
    else:
        body = {"repo": fixture.repo, "query": "resourceuniquetoken", "budget_bytes": 1536}
        path, expected = "/v1/memory/query", 200
    if barrier:
        barrier.wait(timeout=5)
    started = time.perf_counter()
    try:
        status, value = http_request(fixture.api_port, path, body, fixture.token)
        elapsed = (time.perf_counter() - started) * 1000
        if operation == "capture":
            correct = value.get("memory_id") == identifier and value.get("durability") == "local_committed"
        else:
            correct = (any(item.get("id") == "resource-seed" for item in value.get("memories", []))
                       and "resourceuniquetoken" in value.get("context", "")
                       and len(value.get("context", "").encode()) <= 1536)
        return {"milliseconds": elapsed, "status": status, "passed": status == expected and correct,
                "cache_hit": value.get("cache_hit"), "memory_id": identifier if operation == "capture" else "resource-seed"}
    except (OSError, http.client.HTTPException, ValueError) as error:
        return {"milliseconds": (time.perf_counter() - started) * 1000,
                "passed": False, "error": type(error).__name__}


def latency_measurement(fixture, warm_samples, cold_rounds):
    seed = {"repo": fixture.repo, "kind": "decision", "memory_id": "resource-seed",
            "body": "Synthetic resourceuniquetoken exact durable decision", "status": "accepted"}
    status, _ = http_request(fixture.api_port, "/v1/ingest", seed, fixture.token)
    if status != 202:
        raise RuntimeError("Cannot seed authenticated latency fixture")
    results = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        for state in ("warm", "cold"):
            for operation in ("capture", "read"):
                for parallel in (1, 4):
                    samples = []
                    if state == "warm":
                        warmup = timed_operation(fixture, operation)
                        if not warmup["passed"]:
                            raise RuntimeError("Latency warmup failed")
                    rounds = warm_samples // parallel if state == "warm" else cold_rounds
                    for _ in range(rounds):
                        if state == "cold":
                            fixture.start_api()
                        if parallel == 1:
                            samples.append(timed_operation(fixture, operation))
                        else:
                            barrier = threading.Barrier(parallel)
                            pending = [executor.submit(timed_operation, fixture, operation, barrier)
                                       for _ in range(parallel)]
                            samples.extend(future.result(timeout=10) for future in pending)
                        # Stay below sustained 30 rps; pacing is excluded from request latency.
                        time.sleep(0.05 * parallel)
                    results[f"{operation}_{state}_{parallel}_concurrent"] = summarize(samples)
    return results


def run(args):
    metrics = WindowsMetrics()
    root = Path(tempfile.mkdtemp(prefix="cortex-resource-acceptance-"))
    print("Artifacts: " + str(root), flush=True)
    report = {"started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "artifacts": str(root), "logical_cpu_count": os.cpu_count(), "platform": sys.platform,
              "method": {"idle_http_requests": 0, "latency_connection": "fresh loopback TCP per request",
                         "cold": "first authenticated operation/wave after API process restart; OS file cache not flushed",
                         "warm": "operation warmed once before sampling; process/credential cache retained",
                         "percentiles": "nearest rank; includes failed request latency",
                         "io": "GetProcessIoCounters includes file, network and device I/O; not disk-only",
                         "baseline": "simultaneous existing processes and Docker snapshots; no before-activation reduction claim",
                         "models": "no inherited provider credentials; no model calls"},
              "binaries": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                           for name, path in (("api", args.api_binary), ("adapter", args.adapter_binary))}}
    fixture = None
    try:
        fixture = Fixture(root, args.api_binary, args.adapter_binary)
        report["runtime_status"] = fixture.start()
        report["scope"] = fixture.repo
        report["docker_before"] = docker_snapshot(root)
        print("Idle measurement started: " + str(args.idle_seconds) + " seconds", flush=True)
        report["idle"] = idle_measurement(metrics, fixture, args.idle_seconds)
        print("Idle complete; authenticated cold/warm HTTP latency starting", flush=True)
        report["docker_after"] = docker_snapshot(root)
        report["latency"] = latency_measurement(fixture, args.warm_samples, args.cold_rounds)
        report["api_starts"] = fixture.starts
        try:
            connection, _ = fixture.sentinel.accept()
            connection.close()
            report["zero_optional_connections"] = False
        except BlockingIOError:
            report["zero_optional_connections"] = True
        # Read only the new fixture database, never an operator store.
        with sqlite3.connect((root / "metadata.sqlite").as_uri() + "?mode=ro", uri=True) as database:
            report["cron_jobs"] = database.execute("SELECT COUNT(*) FROM cron_jobs").fetchone()[0]
            report["archive_tables"] = database.execute(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name GLOB 'archive_index_*'").fetchone()[0]
        measured = [item for item in report["idle"]["processes"] if item.get("owned_role")]
        report["passed"] = (all(item["failures"] == 0 for item in report["latency"].values())
                            and len(measured) == 2 and not any("unavailable" in item for item in measured)
                            and report["zero_optional_connections"] and report["cron_jobs"] == 0
                            and report["archive_tables"] == 0)
    except Exception as error:
        report["passed"] = False
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        if fixture is not None:
            fixture.close()
        report["ended_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        write_json(root / "report.json", report)
        print("Report: " + str(root / "report.json"), flush=True)
    print(json.dumps({"passed": report["passed"], "latency": {
        name: {key: value for key, value in result.items() if key != "samples"}
        for name, result in report["latency"].items()}}), flush=True)
    return 0 if report["passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--api-binary", type=Path, default=Path("target/debug/cortex-api.exe"))
    parser.add_argument("--adapter-binary", type=Path, default=Path("target/debug/cortex-adapter-claude.exe"))
    parser.add_argument("--idle-seconds", type=float, default=60)
    parser.add_argument("--warm-samples", type=int, default=40)
    parser.add_argument("--cold-rounds", type=int, default=5)
    args = parser.parse_args()
    if not args.run:
        parser.print_help()
        return 0
    if not 1 <= args.idle_seconds <= 300 or args.warm_samples < 4 or args.warm_samples % 4 or args.cold_rounds < 1:
        parser.error("idle-seconds must be 1..300, warm-samples positive multiple of four, cold-rounds positive")
    args.api_binary, args.adapter_binary = args.api_binary.resolve(strict=True), args.adapter_binary.resolve(strict=True)
    return run(args)


if __name__ == "__main__":
    sys.exit(main())

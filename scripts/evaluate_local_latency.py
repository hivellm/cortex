"""Sequential local latency measurements; no OS cache eviction or provider calls."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import threading
import time

if __package__:
    from .evaluate_local_memory import (
        CortexCli,
        DEFAULT_FIXTURES,
        assert_preserved,
        binary_hash,
        build_home,
        canonical,
        load_fixtures,
        parse_json,
        require,
        validate_scope_hits,
    )
    from .local_eval_transport import CortexMcp
else:
    from evaluate_local_memory import (
        CortexCli,
        DEFAULT_FIXTURES,
        assert_preserved,
        binary_hash,
        build_home,
        canonical,
        load_fixtures,
        parse_json,
        require,
        validate_scope_hits,
    )
    from local_eval_transport import CortexMcp


QUERY = "offline desktop notes"
REPOSITORY = "Atlas"
TIMEOUT = 5


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def environment_info():
    processor = platform.processor().strip() or None
    machine = platform.machine().strip() or None
    identifier = None
    if platform.system() == "Windows":
        machine = machine or os.environ.get("PROCESSOR_ARCHITECTURE") or None
        try:
            import winreg

            with winreg.OpenKey(
                winreg.HKEY_LOCAL_MACHINE,
                r"HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            ) as key:
                processor = (
                    processor
                    or str(winreg.QueryValueEx(key, "ProcessorNameString")[0]).strip()
                    or None
                )
                identifier = (
                    str(winreg.QueryValueEx(key, "Identifier")[0]).strip() or None
                )
        except (ImportError, OSError):
            pass
    return {
        "os": platform.platform(),
        "machine": machine,
        "processor": processor,
        "processor_identifier": identifier,
        "logical_cpus": os.cpu_count(),
        "python": platform.python_version(),
        "hardware_note": "CPU model/architecture only; null means unavailable. No hostname or device serial collected.",
    }


def summarize(samples, planned):
    durations = sorted(s["elapsed_ns"] for s in samples if s["success"])

    def percentile(fraction):
        return (
            durations[math.ceil(fraction * len(durations)) - 1] if durations else None
        )

    return {
        "planned": planned,
        "measured": len(samples),
        "successes": len(durations),
        "failures": sum(not s["success"] for s in samples),
        "unmeasured": planned - len(samples),
        "samples": samples,
        "p50_ns": percentile(0.5),
        "p95_ns": percentile(0.95),
        "percentile_method": "nearest rank over successful samples only",
    }


def collect(operation, count, *, cleanup=None, clock=time.perf_counter_ns):
    samples = []
    for ordinal in range(count):
        start = clock()
        success = False
        try:
            operation(ordinal)
            success = True
        except Exception:
            pass
        elapsed = clock() - start
        sample = {"ordinal": ordinal + 1, "elapsed_ns": elapsed, "success": success}
        if not success:
            sample["error"] = "Operation, parsing or semantic validation failed"
        if cleanup:
            try:
                cleanup()
            except Exception:
                sample["success"] = False
                sample["error"] = "Transport cleanup failed outside timing"
        samples.append(sample)
    return summarize(samples, count)


def run_hook(binary, home, payload, event, timeout=TIMEOUT, output_limit=64 * 1024):
    """Bound stdout/stderr while feeding a small fixture through stdin."""
    data = canonical(payload)
    require(len(data) <= 4096, "Hook fixture input exceeds bound")
    env = CortexCli(binary, home).environment()
    command = [
        str(binary),
        "--home",
        str(home),
        "hook",
        "--client",
        "codex",
        "--event",
        event,
    ]
    with subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        cwd=home.parent,
    ) as process:
        buffers = [bytearray(), bytearray()]
        failed = threading.Event()

        def drain(stream, buffer):
            try:
                while chunk := stream.read(4096):
                    if len(buffer) + len(chunk) > output_limit:
                        failed.set()
                        process.kill()
                        return
                    buffer.extend(chunk)
            except OSError:
                failed.set()

        def write():
            try:
                process.stdin.write(data)
                process.stdin.close()
            except (OSError, ValueError):
                failed.set()

        threads = [
            threading.Thread(target=drain, args=(stream, buffer), daemon=True)
            for stream, buffer in zip((process.stdout, process.stderr), buffers)
        ]
        threads.append(threading.Thread(target=write, daemon=True))
        for thread in threads:
            thread.start()
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            failed.set()
            process.kill()
            process.wait()
        finally:
            for thread in threads:
                thread.join(timeout=1)
        require(
            not failed.is_set() and not any(t.is_alive() for t in threads),
            "Hook transport failed",
        )
        require(process.returncode == 0, "Hook process failed")
    return parse_json(bytes(buffers[0]))


def render_context(records, budget=6144):
    context = "Cortex local evidence (advisory; source authority unchanged):\n"
    marker = "<!-- cortex: memory clipped; cortex_memory_read returns more -->\n"
    for record in records:
        title = record["title"].replace("\r", " ").replace("\n", " ")
        locator = record["source"]["locator"].replace("\r", " ").replace("\n", " ")
        citation = (
            f"[{record['id']}@{record['revision']} {record['content_sha256']}] "
            f"{title} | repository={record['repository']} "
            f"authority={record['authority'].title()} source={locator}\n"
        )
        body = record["body"].split("\n")
        lines = [part + "\n" for part in body[:-1]]
        if body[-1]:
            lines.append(body[-1])
        for line in [citation, *lines]:
            suffix = "" if line.endswith("\n") else "\n"
            if len((context + line + suffix + marker).encode()) > budget:
                return context + marker
            context += line + suffix
    return context


def hook_expected(built, baseline):
    def eligible(record):
        applicability = record.get("metadata", {}).get("applicability", {})
        return (
            record["repository"] == REPOSITORY
            and record["kind"] != "event"
            and record["authority"] not in ("superseded", "deprecated")
            and applicability.get("stale") in (None, False)
            and applicability.get("branch") is None
            and applicability.get("path") is None
        )

    recent = [r for r in built["records"].values() if eligible(r)]
    recent.sort(key=lambda r: r["id"])
    recent.sort(
        key=lambda r: datetime.fromisoformat(
            r["source"].get("captured_at") or r["created_at"]
        ).timestamp(),
        reverse=True,
    )
    recent = recent[:10]
    matched = [h["record"] for h in baseline if eligible(h["record"])][:10]
    prompt = matched + [r for r in recent if r["id"] not in {m["id"] for m in matched}]
    return {
        "SessionStart": render_context(recent),
        "UserPromptSubmit": render_context(prompt[:10]),
    }


def measure_latency(binary, fixtures, built, samples=20):
    require(type(samples) is int and 1 <= samples <= 100, "Samples must be 1..100")
    binary = Path(binary).resolve()
    original_hash = binary_hash(binary)
    cli = CortexCli(binary, built["cli"].home, timeout=TIMEOUT)
    manifest, sources = fixtures["manifest"], built["sources"]
    args = ("--repository", REPOSITORY, "search", QUERY, "--limit", "10")
    baseline = cli.run(*args)
    validate_scope_hits(baseline, REPOSITORY, manifest, sources)
    require(bool(baseline), "Latency fixture search must return evidence")
    expected = hook_expected(built, baseline)
    repo = cli.home.parent / "latency-repository"
    repo.mkdir()
    (repo / "cortex.toml").write_text('[cortex]\nid = "Atlas"\n', encoding="utf-8")
    series = {}

    def validate(hits):
        validate_scope_hits(hits, REPOSITORY, manifest, sources)
        require(hits == baseline, "Latency result differs from validated baseline")

    series["cli_new_process_warmed_store"] = collect(
        lambda _: validate(cli.run(*args)), samples
    )
    active = []

    def cold(_):
        transport = CortexMcp(binary, cli.home, timeout=TIMEOUT)
        active.append(transport)
        transport.__enter__()
        validate(
            transport.call(
                "cortex_search", {"repository": REPOSITORY, "query": QUERY, "limit": 10}
            )
        )

    def close():
        while active:
            active.pop().close()

    series["mcp_new_process_first_request"] = collect(cold, samples, cleanup=close)
    transport = CortexMcp(binary, cli.home, timeout=TIMEOUT)
    warm_name = "mcp_persistent_after_warmup"
    try:
        transport.__enter__()
        validate(
            transport.call(
                "cortex_search", {"repository": REPOSITORY, "query": QUERY, "limit": 10}
            )
        )
        series[warm_name] = collect(
            lambda _: validate(
                transport.call(
                    "cortex_search",
                    {"repository": REPOSITORY, "query": QUERY, "limit": 10},
                )
            ),
            samples,
        )
    except Exception:
        series[warm_name] = summarize([], samples)
        series[warm_name]["setup_error"] = "Persistent MCP warmup failed"
    finally:
        try:
            transport.close()
        except Exception:
            series[warm_name]["cleanup_error"] = (
                "Persistent MCP cleanup failed outside timing"
            )
    for event in ("SessionStart", "UserPromptSubmit"):
        payload = {
            "cwd": str(repo.resolve()),
            "prompt": QUERY,
            "session_id": "fictional-latency-session",
            "event_id": "latency-" + event,
        }

        def hook(ordinal, event=event, payload=payload):
            # Each UserPromptSubmit exercises a new event publication.
            current = dict(payload, event_id=payload["event_id"] + "-" + str(ordinal))
            result = run_hook(binary, cli.home, current, event)
            require(
                result
                == {
                    "hookSpecificOutput": {
                        "hookEventName": event,
                        "additionalContext": expected[event],
                    }
                },
                "Hook context missing or incorrect",
            )

        name = "hook_" + event + "_new_process_warmed_store"
        try:
            hook(-1)
            series[name] = collect(hook, samples)
        except Exception:
            series[name] = summarize([], samples)
            series[name]["setup_error"] = "Hook warmup context or execution failed"
    preserved = True
    try:
        require(
            binary_hash(binary) == original_hash, "Binary changed during latency run"
        )
        assert_preserved(sources, built["report"]["source_hashes"])
        for record in built["records"].values():
            require(
                cli.run("--repository", record["repository"], "get", record["id"])
                == record,
                "Canonical fixture record changed during latency run",
            )
    except Exception:
        preserved = False
    return {
        "series": series,
        "passed": preserved
        and all(
            s["successes"] == samples and not s.get("cleanup_error")
            for s in series.values()
        ),
        "binary_sources_and_original_records_preserved": preserved,
        "concurrency": 1,
        "samples_per_series": samples,
        "clock": "time.perf_counter_ns",
        "repository": REPOSITORY,
        "query": QUERY,
        "timing_scope": "Evaluator-observed wall time, including subprocess polling, transport/thread overhead, JSON decoding and semantic/provenance validation; not internal engine timing. MCP cold includes spawn, initialize and first tool request; teardown excluded. CLI and direct hooks spawn a process every sample on an already prepared/warmed store. Import, build, baseline and warmup excluded.",
        "cold_definition": "Fresh process only; OS/filesystem caches are not evicted or claimed cold.",
        "hook_scope": "Direct cortex hook only; no launcher or client scheduling. UserPromptSubmit includes local event publication; both hooks may publish local audit receipts while preserving existing memory sources. Prepared context does not prove model consumption.",
        "deadlines": {
            "cli_or_hook_seconds": TIMEOUT,
            "mcp_seconds_per_initialize_or_tool_phase": TIMEOUT,
        },
    }


def evaluate(binary, report_path, fixtures_path=DEFAULT_FIXTURES, samples=20):
    report = {
        "schema_version": 1,
        "started_at": utc_now(),
        "passed": False,
        "environment": environment_info(),
    }
    try:
        require(type(samples) is int and 1 <= samples <= 100, "Samples must be 1..100")
        binary = Path(binary).resolve()
        fixtures = load_fixtures(fixtures_path)
        report["binary"] = {"sha256": binary_hash(binary)}
        report["corpus"] = {
            "manifest_sha256": fixtures["manifest_sha256"],
            "files": fixtures["manifest"]["files"],
        }
        with tempfile.TemporaryDirectory(prefix="cortex-latency-") as scratch:
            root = Path(scratch)
            report["binary"]["build_info"] = CortexCli(binary, root / "info").run(
                "build-info"
            )
            built = build_home(binary, fixtures, root / "fixture")
            require(
                binary_hash(binary) == report["binary"]["sha256"],
                "Binary changed during fixture build",
            )
            report["latency"] = measure_latency(binary, fixtures, built, samples)
            report["passed"] = report["latency"]["passed"]
            require(
                binary_hash(binary) == report["binary"]["sha256"],
                "Binary changed during evaluation",
            )
    except Exception:
        report["passed"] = False
        report["error"] = "Latency setup, execution or validation failed"
    finally:
        report["finished_at"] = utc_now()
        Path(report_path).write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--samples", type=int, default=20)
    args = parser.parse_args()
    report = evaluate(args.binary, args.report, samples=args.samples)
    print(
        "Latency report written; "
        + ("checks passed." if report["passed"] else "checks failed.")
    )
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()

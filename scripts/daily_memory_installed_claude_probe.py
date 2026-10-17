#!/usr/bin/env python3
"""Opt-in native Claude recall through installed user hooks; never replaces hooks.

Expected memory IDs/details remain host-side. No tool or MCP server is enabled.
Default invocation displays help. Use only after coordinator capture-ready GO.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from daily_memory_cross_model_probe import stream_evidence
from daily_memory_live_harness import redact, reported_usage, stop, write_json


PROMPT = (
    "Report the two current accepted repository decisions supplied by the installed Cortex lifecycle hooks. "
    "Use only context already supplied to this session. Do not use tools or inspect files. "
    "Return one JSON object only, no Markdown: "
    '{"memories":[{"id":"exact cited memory ID","version":1,"details":"exact operational requirements"}]}. '
    "Include both IDs and the concrete requirements from each saved decision. If context lacks either, "
    'return {"missing":true}. Do not invent decisions or identifiers.'
)


def settings_inventory(path, expected_hook):
    content = path.read_bytes()
    value = json.loads(content.decode("utf-8-sig"))
    hooks = []
    for event, entries in value.get("hooks", {}).items():
        for entry in entries:
            for hook in entry.get("hooks", []):
                command = hook.get("command", "")
                if "cortex-hook" in command.lower():
                    hooks.append({"event": event, "command_sha256": hashlib.sha256(command.encode()).hexdigest(),
                                  "expected_binary": str(expected_hook).replace("\\", "/").lower()
                                  in command.replace("\\", "/").lower(),
                                  "synchronous": "--fire-forget" not in command})
    return {"sha256": hashlib.sha256(content).hexdigest(), "hooks": hooks,
            "all_hooks_enabled": not value.get("disableAllHooks", False)}


def session_hook_events(log, offset, sessions):
    if not log.exists() or log.stat().st_size < offset:
        return {"available": False, "reason": "log_missing_or_rotated", "events": []}
    with log.open("rb") as source:
        source.seek(offset)
        data = source.read(64 * 1024)
    events = []
    pattern = re.compile(r"^(\S+) (\w+) env_sid=(\S*) payload_sid=(\S*) pid=(\d+)$")
    for line in data.decode("utf-8", errors="replace").splitlines():
        match = pattern.match(line)
        if match and (match[3] in sessions or match[4] in sessions):
            events.append({"time": match[1], "event": match[2], "env_session": match[3],
                           "payload_session": match[4], "dispatcher_pid": int(match[5])})
    return {"available": True, "events": events, "tail_limit_bytes": 65536}


def validate(results, expected):
    try:
        value = json.loads(results[-1]) if results else None
    except json.JSONDecodeError:
        value = None
    memories = value.get("memories", []) if isinstance(value, dict) else []
    wanted = {item["id"]: item for item in expected["memories"]}
    checks = {"two_exact_ids": len(memories) == len(wanted) == 2 and
              {item.get("id") for item in memories if isinstance(item, dict)} == set(wanted)}
    for memory_id, requirement in wanted.items():
        matches = [item for item in memories if isinstance(item, dict) and item.get("id") == memory_id]
        details = matches[0].get("details", "") if matches else ""
        checks[memory_id + "_version"] = bool(matches) and matches[0].get("version") == requirement.get("version", 1)
        checks[memory_id + "_details"] = isinstance(details, str) and all(
            part.casefold() in details.casefold() for part in requirement["required_detail_substrings"])
    return {"passed": all(checks.values()), "checks": checks, "model_action": value}


def run(args):
    directory = Path(tempfile.mkdtemp(prefix="cortex-installed-claude-"))
    print("Artifacts: " + str(directory), flush=True)
    expected = json.loads(args.expectations.read_text(encoding="utf-8-sig"))
    prompt = args.prompt_file.read_text(encoding="utf-8") if args.prompt_file else PROMPT
    if len(expected.get("memories", [])) != 2:
        raise ValueError("Exactly two host-side memory expectations required")
    if any(item["id"] in prompt for item in expected["memories"]):
        raise ValueError("Expected IDs must stay absent from model prompt")
    before = settings_inventory(args.user_settings, args.hook_binary)
    lifecycle = {item["event"] for item in before["hooks"] if item["expected_binary"] and item["synchronous"]}
    if not before["all_hooks_enabled"] or not {"SessionStart", "UserPromptSubmit"} <= lifecycle:
        raise ValueError("Real installed synchronous Cortex user hooks are not ready")
    write_json(directory / "user-hooks-before.json", before)
    mcp = directory / "empty-mcp.json"
    write_json(mcp, {"mcpServers": {}})
    command = [args.claude_binary or shutil.which("claude"), "-p", prompt,
               "--setting-sources", "user", "--strict-mcp-config", "--mcp-config", str(mcp),
               "--tools", "", "--system-prompt", "Answer only from the actual lifecycle context delivered to this session. No tools, no guessing.",
               "--model", "claude-fable-5-1", "--permission-mode", "dontAsk", "--no-session-persistence",
               "--verbose", "--output-format", "stream-json", "--effort", "low", "--max-budget-usd", "1"]
    if not command[0]:
        raise ValueError("Installed Claude executable unavailable")
    write_json(directory / "invocation.json", {"argv": command, "cwd": str(args.cwd.resolve()),
               "timeout_seconds": 120, "auth_source": "installed native client; unchanged user environment",
               "hook_source": str(args.user_settings), "hook_settings_replaced": False})
    log_offset = args.hook_log.stat().st_size if args.hook_log.exists() else 0
    env = os.environ.copy()
    env.pop("CORTEX_LIVE_ROUTER_KEY", None)
    start = time.monotonic()
    timed_out = False
    stdout_path, stderr_path = directory / "stdout.log", directory / "stderr.log"
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(command, cwd=args.cwd, env=env, stdout=stdout, stderr=stderr)
        try:
            process.wait(timeout=120)
        except subprocess.TimeoutExpired:
            timed_out = True
            stop(process)
    elapsed = time.monotonic() - start
    raw = stdout_path.read_bytes()
    for path in (stdout_path, stderr_path):
        path.write_text(redact(path.read_text(encoding="utf-8", errors="replace")), encoding="utf-8")
    trace = stream_evidence(raw)
    results, tools, native_hooks = [], [], []
    for line in raw.decode("utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "result":
            results.append(event.get("result", ""))
        if event.get("type") == "system" and event.get("subtype") == "init":
            tools = event.get("tools", [])
        if event.get("type") == "system" and event.get("subtype") in ("hook_started", "hook_response"):
            hook_event = {key: event[key] for key in ("subtype", "hook_id", "hook_name", "hook_event", "exit_code", "outcome")
                          if key in event}
            if event.get("subtype") == "hook_response":
                try:
                    hook_value = json.loads(event.get("stdout", ""))
                except json.JSONDecodeError:
                    hook_value = {}
                context = hook_value.get("hookSpecificOutput", {}).get("additionalContext", "")
                hook_event.update(context_bytes=len(context.encode("utf-8")),
                    context_sha256=hashlib.sha256(context.encode("utf-8")).hexdigest(),
                    expected_ids_present=all(item["id"] in context for item in expected["memories"]),
                    unavailable_notice=bool(hook_value.get("systemMessage")))
            native_hooks.append(hook_event)
    after = settings_inventory(args.user_settings, args.hook_binary)
    write_json(directory / "user-hooks-after.json", after)
    hook_events = session_hook_events(args.hook_log, log_offset, set(trace["observed_session_ids"]))
    report = validate(results, expected)
    report["checks"].update(no_model_tool_actions=not trace["observed_tool_actions"],
        no_advertised_tools=not tools, user_hooks_settings_unchanged=before == after,
        native_sessionstart_context=any(item.get("hook_event") == "SessionStart" and item.get("expected_ids_present")
                                       for item in native_hooks),
        observed_real_session_hooks=hook_events["available"] and {"SessionStart", "UserPromptSubmit"}
        <= {item["event"] for item in hook_events["events"]},
        client_completed=process.returncode == 0 and not timed_out)
    report.update(passed=all(report["checks"].values()), scenario_label=args.scenario_label,
                  artifacts=str(directory), exit_code=process.returncode,
                  timed_out=timed_out, duration_seconds=round(elapsed, 3), client_pid=process.pid,
                  stream_evidence=trace, real_hook_dispatches=hook_events,
                  native_hook_events=native_hooks,
                  reported_usage=reported_usage(raw), evidence_scope="actual installed user hooks; no wrapper, fixture, MCP or model tools")
    write_json(directory / "report.json", report)
    print(json.dumps(redact(report), ensure_ascii=True), flush=True)
    return 0 if report["passed"] else 1


def self_test():
    expected = {"memories": [{"id": "memory-a", "required_detail_substrings": ["SQLite"]},
                              {"id": "memory-b", "required_detail_substrings": ["manual only"]}]}
    result = {"memories": [{"id": "memory-a", "version": 1, "details": "Use SQLite"},
                            {"id": "memory-b", "version": 1, "details": "Manual only"}]}
    assert validate([json.dumps(result)], expected)["passed"]
    result["memories"][1]["details"] = "automatic"
    assert not validate([json.dumps(result)], expected)["passed"]
    assert not validate(["MISSING"], expected)["passed"]
    print("Offline validation passed; no client, hook or model invoked.")
    return 0


def main():
    home = Path.home()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--expectations", type=Path)
    parser.add_argument("--prompt-file", type=Path, help="Reviewed host-side public prompt; default asks for real decisions")
    parser.add_argument("--scenario-label", choices=["installed-real-decisions", "installed-public-synthetic"],
                        default="installed-real-decisions")
    parser.add_argument("--cwd", type=Path, default=Path.cwd())
    parser.add_argument("--claude-binary")
    parser.add_argument("--user-settings", type=Path, default=home / ".claude/settings.json")
    parser.add_argument("--hook-log", type=Path, default=home / ".cortex/hook-invocations.log")
    parser.add_argument("--hook-binary", type=Path,
                        default=Path.cwd() / ".cortex-state/daily-runtime/bin/cortex-hook.exe")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.run:
        parser.print_help()
        return 0
    if not args.expectations:
        parser.error("--run requires host-only --expectations")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Two native-client sessions against disposable real memory API; opt-in only.

Claude captures and explicitly supersedes an accepted policy. After API restart,
Codex constructs a request using values absent from its prompt. No request is
executed. Native CLI authentication stays in each installed client. Stdlib only.
"""

import argparse
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import uuid
from urllib.request import Request, urlopen

import daily_memory_live_harness as live


QUERY = "billing export request policy"
READ = {"repo": live.REPO, "query": QUERY, "limit": 5, "budget_bytes": 1536}
MODELS = {"claude": "claude-fable-5-1", "codex": "cx/gpt-6.1-sol"}


def scenario():
    token = uuid.uuid4().hex
    old_request = {"method": "POST", "path": "/synthetic/export/old-" + token[:10],
                   "headers": {"X-Export-Policy": "old-" + token},
                   "retry": {"max_attempts": 7, "backoff_ms": 900}}
    new_request = {"method": "PUT", "path": "/synthetic/export/current-" + token[10:20],
                   "headers": {"X-Export-Policy": "current-" + token[::-1]},
                   "retry": {"max_attempts": 2, "backoff_ms": 175}}
    def decision(name, request):
        return {"repo": live.REPO, "kind": "decision", "status": "accepted",
                "memory_id": name + "-" + token,
                "body": QUERY + ". Required request JSON: " + json.dumps(request)}
    old = decision("original", old_request)
    new = decision("corrected", new_request)
    new["supersedes"] = old["memory_id"]
    distractors = [
        {**decision("wrong-repo", old_request), "repo": "phase34-other-repository"},
        {**decision("proposed", old_request), "status": "proposed"},
        {**decision("unrelated", old_request), "body": "Health monitor uses port 8123."},
    ]
    return {"original": old, "successor": new, "distractors": distractors,
            "expected_request": new_request, "old_request": old_request}


def allowed(params, policy):
    args = params.get("arguments", {})
    if params.get("name") == "cortex_memory_read":
        return args == READ
    if params.get("name") != "cortex_capture_memory":
        return False
    for expected in policy.get("captures", []):
        if "supersedes" in expected:
            revision = args.get("expected_revision")
            if type(revision) is not int or revision < 0:
                continue
            if args == {**expected, "expected_revision": revision}:
                return True
        elif args == expected:
            return True
    return False


def proxy(args):
    policy = json.loads(Path(args.policy).read_text(encoding="utf-8"))
    live.allowed_call = lambda params, _nonce, _memory_id: allowed(params, policy)
    args.nonce = args.memory_id = None
    return live.proxy(args)


def api(ready, endpoint, value):
    live.validate_ready(ready)
    request = Request(ready["api_url"] + endpoint,
                      data=json.dumps(value).encode("utf-8"),
                      headers={"Content-Type": "application/json"})
    with urlopen(request, timeout=8) as response:
        return {"status": response.status, "body": json.load(response)}


def start_fixture(binary, directory, resume=False):
    log = (directory / ("fixture-restarted.stderr.log" if resume else "fixture.stderr.log")).open("wb")
    command = [str(binary), "--directory", str(directory / "fixture")]
    if resume:
        command.append("--resume")
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=log)
    lines = queue.Queue()
    threading.Thread(target=lambda: lines.put(process.stdout.readline()), daemon=True).start()
    try:
        ready = json.loads(lines.get(timeout=15))
        live.validate_ready(ready)
        live.write_json(directory / ("fixture-restarted.ready.json" if resume else "fixture.ready.json"), ready)
        return process, log, ready
    except Exception:
        live.stop(process)
        log.close()
        raise


def observed(path):
    requests, results = {}, []
    if not path.exists():
        return results
    for line in path.read_text(encoding="utf-8").splitlines():
        event = json.loads(line)
        frame = event["frame"]
        if not isinstance(frame, dict):
            continue
        key = json.dumps(frame.get("id"))
        if event["direction"] == "client_to_proxy" and frame.get("method") == "tools/call":
            requests[key] = frame["params"]
        if event["direction"] == "proxy_to_client" and key in requests:
            results.append({"rpc_id": frame.get("id"), "params": requests.pop(key),
                            "payload": live.payload(frame)})
    return results


def read_checks(value, expected_id, forbidden):
    value = value or {}
    ids = [item.get("id") for item in value.get("memories", [])]
    context = value.get("context", "")
    return {"exact_active_memory_only": ids == [expected_id],
            "version_one": len(value.get("memories", [])) == 1
            and value["memories"][0].get("version") == 1,
            "citation_present": expected_id in context,
            "no_distractor_citations": not any(item in context or item in ids for item in forbidden),
            "bounded_context": len(context.encode("utf-8")) <= 1536,
            "prepared_delivery": bool(value.get("delivery_id"))
            and value.get("delivery_id") == value.get("query_id")}


def validate_writer(records, case):
    captures = [r for r in records if r["params"]["name"] == "cortex_capture_memory"]
    reads = [r for r in records if r["params"]["name"] == "cortex_memory_read"]
    sequence = [r["params"]["name"] for r in records]
    checks = {"capture_read_correct_read_order": sequence == ["cortex_capture_memory", "cortex_memory_read",
                                                             "cortex_capture_memory", "cortex_memory_read"]}
    for index, name in enumerate(("original", "successor")):
        value = captures[index]["payload"] if len(captures) > index else None
        checks[name + "_committed"] = bool(value) and value.get("memory_id") == case[name]["memory_id"] \
            and value.get("version") == 1 and value.get("durability") == "local_committed"
    checks["matching_revision"] = len(captures) == 2 and bool(reads) and bool(reads[0]["payload"]) \
        and captures[1]["params"]["arguments"].get("expected_revision") == reads[0]["payload"].get("revision")
    forbidden = [d["memory_id"] for d in case["distractors"]]
    for index, name in enumerate(("original", "successor")):
        value = reads[index]["payload"] if len(reads) > index else None
        excluded = forbidden + ([case["original"]["memory_id"]] if index else [])
        checks.update({name + "_" + k: v for k, v in read_checks(value, case[name]["memory_id"], excluded).items()})
    return {"passed": all(checks.values()), "checks": checks, "rpc_results": records}


def validate_reader(records, final, case, prompt):
    reads = [r for r in records if r["params"]["name"] == "cortex_memory_read" and r["payload"]]
    value = reads[-1]["payload"] if reads else None
    checks = read_checks(value, case["successor"]["memory_id"],
                         [case["original"]["memory_id"]] + [d["memory_id"] for d in case["distractors"]])
    try:
        action = json.loads(final.strip())
    except (json.JSONDecodeError, AttributeError):
        action = None
    expected = {"request": case["expected_request"], "memory_id": case["successor"]["memory_id"],
                "version": 1, "delivery_id": (value or {}).get("delivery_id")}
    checks.update(exact_model_action=action == expected, one_read_only=len(records) == len(reads) == 1,
                  hidden_values_absent_from_prompt=all(
                      secret not in prompt for secret in (case["expected_request"]["path"],
                      case["expected_request"]["headers"]["X-Export-Policy"], case["successor"]["memory_id"])),
                  corrected_values_in_tool_context=bool(value) and all(
                      secret in value.get("context", "") for secret in (case["expected_request"]["path"],
                      case["expected_request"]["headers"]["X-Export-Policy"])))
    return {"passed": all(checks.values()), "checks": checks, "model_action": action,
            "rpc_results": records, "receipt_stage": "prepared; model action verified independently"}


def stream_evidence(stdout):
    sessions, models, actions, forbidden = set(), set(), [], []
    def visit(value):
        if isinstance(value, list):
            for item in value:
                visit(item)
        elif isinstance(value, dict):
            for name in ("session_id", "thread_id"):
                if isinstance(value.get(name), str):
                    sessions.add(value[name])
            for name in ("model", "model_name"):
                if isinstance(value.get(name), str):
                    models.add(value[name])
            kind = value.get("type")
            if kind in {"tool_use", "mcp_tool_call", "command_execution", "file_change", "web_search",
                        "function_call", "custom_tool_call"}:
                action = {key: value[key] for key in ("type", "name", "server", "tool") if key in value}
                actions.append(action)
                valid = (kind == "tool_use" and value.get("name") in
                         {"mcp__cortex__" + tool for tool in live.TOOLS}) or (
                         kind == "mcp_tool_call" and value.get("server") == "cortex"
                         and value.get("tool") in live.TOOLS)
                if not valid:
                    forbidden.append(action)
            for item in value.values():
                if isinstance(item, (list, dict)):
                    visit(item)
    for line in stdout.decode("utf-8", errors="replace").splitlines():
        try:
            visit(json.loads(line))
        except json.JSONDecodeError:
            pass
    return {"observed_session_ids": sorted(sessions), "observed_models": sorted(models),
            "observed_tool_actions": actions, "forbidden_tool_actions": forbidden,
            "scope": "native structured stream only; provider fields absent from stream stay unavailable"}


def native_client(client, directory, ready, server, prompt, args):
    executable = getattr(args, client + "_binary") or shutil.which(client)
    if not executable:
        raise ValueError("Installed client unavailable: " + client)
    env = os.environ.copy()
    env.pop("CORTEX_LIVE_ROUTER_KEY", None)
    live.write_json(directory / "prompt.json", {"prompt": prompt})
    live.write_json(directory / "mcp.json", {"mcpServers": {"cortex": server}})
    if client == "claude":
        live.write_json(directory / "settings.json", {})
        command = [executable, "-p", prompt, "--setting-sources", "", "--settings", str(directory / "settings.json"),
                   "--strict-mcp-config", "--mcp-config", str(directory / "mcp.json"), "--tools", "",
                   "--allowedTools", *["mcp__cortex__" + t for t in sorted(live.TOOLS)],
                   "--system-prompt", "Perform this synthetic memory probe. Use only Cortex MCP tools. No shell or file work.",
                   "--model", MODELS[client], "--permission-mode", "dontAsk", "--no-session-persistence",
                   "--verbose", "--output-format", "stream-json", "--effort", "low",
                   "--max-budget-usd", str(args.claude_budget_usd)]
    else:
        command = [executable, "--ask-for-approval", "never", "--no-daemon", "exec", "--ignore-rules", "--ephemeral",
                   "--skip-git-repo-check", "--sandbox", "read-only", "--cd", ready["cwd"], "--model", MODELS[client],
                   "--json", "--output-last-message", str(directory / "last-message.txt")]
        overrides = {"mcp_servers.cortex.command": json.dumps(server["command"]),
                     "mcp_servers.cortex.args": json.dumps(server["args"]), "mcp_servers.cortex.enabled": "true",
                     "mcp_servers.cortex.tools.cortex_memory_read.approval_mode": json.dumps("approve")}
        for key, value in overrides.items():
            command += ["-c", key + "=" + value]
        command.append(prompt)
    live.write_json(directory / "invocation.json", {"argv": command, "requested_model": MODELS[client],
                    "authentication": "installed native client; no credential inspection", "attempts": 1})
    version = subprocess.run([executable, "--version"], capture_output=True, timeout=15, check=False)
    live.write_json(directory / "version.json", {"exit_code": version.returncode,
                    "stdout": version.stdout.decode("utf-8", errors="replace")})
    # File-backed capture also avoids inherited-pipe EOF stalls from child MCP processes.
    out_path, err_path = directory / "stdout.log", directory / "stderr.log"
    timed_out = False
    with out_path.open("wb") as out, err_path.open("wb") as err:
        process = subprocess.Popen(command, cwd=ready["cwd"], env=env, stdout=out, stderr=err)
        try:
            process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            live.stop(process)
    stdout = out_path.read_bytes()
    for path in (out_path, err_path):
        path.write_text(live.redact(path.read_text(encoding="utf-8", errors="replace")), encoding="utf-8")
    final_path = directory / "last-message.txt"
    final = final_path.read_text(encoding="utf-8") if final_path.exists() else ""
    if final_path.exists():
        final_path.write_text(live.redact(final), encoding="utf-8")
    return {"exit_code": process.returncode, "client_pid": process.pid, "timed_out": timed_out,
            "reported_usage": live.reported_usage(stdout), "stream_evidence": stream_evidence(stdout)}, final


def run(args):
    directory = Path(tempfile.mkdtemp(prefix="cortex-cross-model-"))
    print("Artifacts: " + str(directory), flush=True)
    case = scenario()
    live.write_json(directory / "scenario.json", case)
    fixture = log = None
    report = {"passed": False, "artifacts": str(directory), "models": MODELS,
              "execution": "JSON request constructed only; never executed", "model_calls": 0,
              "model_call_unit": "native CLI session; MCP tool loops may make multiple provider requests"}
    try:
        fixture, log, ready = start_fixture(Path(args.fixture_binary).resolve(), directory)
        report["seed_receipts"] = [api(ready, "/v1/ingest", d) for d in case["distractors"]]
        if not all(r["status"] == 202 for r in report["seed_receipts"]):
            raise ValueError("Distractor seed failed")
        writer_prompt = ("Use only Cortex MCP tools, exactly four calls in this order. "
                         "1. cortex_capture_memory with " + json.dumps(case["original"]) + ". "
                         "2. cortex_memory_read with " + json.dumps(READ) + ". "
                         "3. User correction: replace original policy by calling cortex_capture_memory with "
                         + json.dumps(case["successor"]) + " plus expected_revision equal to revision returned by call 2. "
                         "4. cortex_memory_read with " + json.dumps(READ) + ". No other tools. Report completion briefly.")
        reader_prompt = ("Prepare billing export request from current accepted repository policy. Use only "
                         "cortex_memory_read exactly once with " + json.dumps(READ) + ". No other tools, shell or files. "
                         "Do not execute request. Return one JSON object only, no Markdown: "
                         '{"request": <exact required request JSON from memory>, "memory_id": <cited active memory ID>, '
                         '"version": <cited version>, "delivery_id": <actual tool delivery_id>}. '
                         "Apply current policy, including method, path, headers and retry. Do not guess missing values.")
        for client in ("claude", "codex"):
            if client == "codex":
                old_pid = fixture.pid
                live.stop(fixture)
                log.close()
                fixture, log, resumed = start_fixture(Path(args.fixture_binary).resolve(), directory, resume=True)
                report["restart"] = {"old_pid": old_pid, "new_pid": fixture.pid,
                                     "same_database": ready["db_path"] == resumed["db_path"]}
                if not report["restart"]["same_database"]:
                    raise ValueError("Restart changed database")
                ready = resumed
            session = directory / client
            session.mkdir()
            (session / "cortex-empty.toml").write_text("# Synthetic fixture only.\n", encoding="utf-8")
            live.write_json(session / "policy.json", {"captures": [case["original"], case["successor"]]
                            if client == "claude" else []})
            server = {"command": sys.executable, "args": [str(Path(__file__).resolve()), "--proxy",
                      "--mcp-binary", str(Path(args.mcp_binary).resolve()), "--api-url", ready["api_url"],
                      "--journal", str(session / "mcp.jsonl"), "--policy", str(session / "policy.json")]}
            prompt = writer_prompt if client == "claude" else reader_prompt
            report["model_calls"] += 1
            runtime, final = native_client(client, session, ready, server, prompt, args)
            records = observed(session / "mcp.jsonl")
            result = validate_writer(records, case) if client == "claude" else validate_reader(records, final, case, prompt)
            result["runtime"] = runtime
            result["checks"]["no_other_native_tool_actions"] = not runtime["stream_evidence"]["forbidden_tool_actions"]
            result["checks"]["native_tool_actions_observed"] = bool(runtime["stream_evidence"]["observed_tool_actions"])
            result["passed"] = all(result["checks"].values())
            result["passed"] = result["passed"] and runtime["exit_code"] == 0 and not runtime["timed_out"]
            report[client] = result
            live.write_json(directory / "report.json", report)
            if not result["passed"]:
                raise ValueError(client + " acceptance failed; no automatic retry")
        writer_runtime, reader_runtime = report["claude"]["runtime"], report["codex"]["runtime"]
        writer_ids = set(writer_runtime["stream_evidence"]["observed_session_ids"])
        reader_ids = set(reader_runtime["stream_evidence"]["observed_session_ids"])
        report["session_separation"] = {
            "distinct_client_processes": writer_runtime["client_pid"] != reader_runtime["client_pid"],
            "observed_session_ids_disjoint": not bool(writer_ids & reader_ids),
            "session_id_observation": "both clients" if writer_ids and reader_ids else "partial; separate CLI processes recorded"}
        report["passed"] = all(v for k, v in report["session_separation"].items() if k != "session_id_observation")
    except Exception as error:
        report["error"] = str(error)
    finally:
        if fixture is not None:
            live.stop(fixture)
        if log is not None:
            log.close()
        live.write_json(directory / "report.json", report)
    print(json.dumps(live.redact(report), ensure_ascii=True), flush=True)
    return 0 if report["passed"] else 1


def self_test():
    case = scenario()
    policy = {"captures": [case["original"], case["successor"]]}
    assert allowed({"name": "cortex_capture_memory", "arguments": case["original"]}, policy)
    assert allowed({"name": "cortex_capture_memory", "arguments": {**case["successor"], "expected_revision": 3}}, policy)
    assert not allowed({"name": "cortex_capture_memory", "arguments": case["successor"]}, policy)
    assert not allowed({"name": "cortex_capture_memory", "arguments": {**case["successor"], "expected_revision": True}}, policy)
    assert not allowed({"name": "cortex_capture_memory", "arguments": case["original"]}, {"captures": []})
    assert not allowed({"name": "cortex_memory_read", "arguments": {**READ, "include_global": True}}, policy)
    assert not allowed({"name": "cortex_memory_read", "arguments": {**READ, "repo": "wrong"}}, policy)
    writer_records = []
    for index, name in enumerate(("original", "successor")):
        arguments = case[name] if index == 0 else {**case[name], "expected_revision": 3}
        memory_id = case[name]["memory_id"]
        writer_records += [
            {"params": {"name": "cortex_capture_memory", "arguments": arguments},
             "payload": {"memory_id": memory_id, "version": 1, "durability": "local_committed"}},
            {"params": {"name": "cortex_memory_read", "arguments": READ},
             "payload": {"context": memory_id + " " + case[name]["body"], "revision": 3 + index,
                         "memories": [{"id": memory_id, "version": 1, "scope": live.REPO, "event_id": "synthetic"}],
                         "delivery_id": "d" + str(index), "query_id": "d" + str(index)}}]
    assert validate_writer(writer_records, case)["passed"]
    writer_records[2]["params"]["arguments"]["expected_revision"] = 2
    assert not validate_writer(writer_records, case)["passed"]
    value = {"context": case["successor"]["memory_id"] + " " + case["successor"]["body"],
             "memories": [{"id": case["successor"]["memory_id"], "version": 1}], "delivery_id": "d1", "query_id": "d1"}
    records = [{"params": {"name": "cortex_memory_read"}, "payload": value}]
    action = {"request": case["expected_request"], "memory_id": case["successor"]["memory_id"], "version": 1, "delivery_id": "d1"}
    assert validate_reader(records, json.dumps(action), case, QUERY)["passed"]
    assert not validate_reader(records, json.dumps({**action, "request": case["old_request"]}), case, QUERY)["passed"]
    assert not validate_reader(records, json.dumps({**action, "delivery_id": "fabricated"}), case, QUERY)["passed"]
    assert not validate_reader(records, json.dumps(action), case, value["context"])["passed"]
    value["memories"].append({"id": case["original"]["memory_id"], "version": 1})
    assert not validate_reader(records, json.dumps(action), case, QUERY)["passed"]
    trace = stream_evidence(b'{"type":"thread.started","thread_id":"session-b"}\n'
                            b'{"item":{"type":"mcp_tool_call","server":"cortex","tool":"cortex_memory_read"}}\n')
    assert trace["observed_session_ids"] == ["session-b"] and not trace["forbidden_tool_actions"]
    assert stream_evidence(b'{"item":{"type":"command_execution"}}')["forbidden_tool_actions"]
    print("Offline checks passed; no fixture, credentials or model calls.")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="Authorize exactly two bounded native model sessions")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--fixture-binary")
    parser.add_argument("--mcp-binary")
    parser.add_argument("--claude-binary")
    parser.add_argument("--codex-binary")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--claude-budget-usd", type=float, default=1.0)
    parser.add_argument("--proxy", action="store_true", help=argparse.SUPPRESS)
    for name in ("api-url", "journal", "policy"):
        parser.add_argument("--" + name, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.proxy:
        return proxy(args)
    if args.self_test:
        return self_test()
    if not args.run:
        parser.print_help()
        return 0
    if not args.fixture_binary or not args.mcp_binary:
        parser.error("--run requires --fixture-binary and --mcp-binary")
    if args.timeout < 1 or args.claude_budget_usd <= 0:
        parser.error("Timeout and budget must be positive")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())

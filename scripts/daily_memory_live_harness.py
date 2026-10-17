#!/usr/bin/env python3
"""Optional installed-client capture/read probe. No live calls without --run.

Uses only a new synthetic fixture. Retains redacted artifacts; never edits user
configuration. The stdio proxy exposes exactly two tools and records actual RPC
requests/results, independently of the model's final prose. Python stdlib only.
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
import time
import uuid
from urllib.parse import urlparse


TOOLS = {"cortex_capture_memory", "cortex_memory_read"}
REPO = "phase34-harness"
SECRET_NAMES = ("CORTEX_LIVE_ROUTER_KEY", "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "CORTEX_API_TOKEN",
                "KIMI_MODEL_API_KEY")


def redact(value):
    """Redact known credentials even when embedded in CLI error strings."""
    serialized = json.dumps(value, ensure_ascii=False)
    for name in SECRET_NAMES:
        secret = os.environ.get(name)
        if secret:
            serialized = serialized.replace(json.dumps(secret)[1:-1], "[REDACTED]")
    return json.loads(serialized)


def write_json(path, value):
    path.write_text(json.dumps(redact(value), indent=2, ensure_ascii=False) + "\n",
                    encoding="utf-8")


def stop(process):
    if process.poll() is not None:
        return
    # Only this harness-owned process tree; no image-name/global termination.
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                       check=False)
    else:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()


def fixture_ready(binary, directory, timeout=15):
    error_log = (directory / "fixture.stderr.log").open("wb")
    process = subprocess.Popen([str(binary), "--directory", str(directory / "fixture")],
                               stdout=subprocess.PIPE, stderr=error_log)
    lines = queue.Queue()
    threading.Thread(target=lambda: lines.put(process.stdout.readline()), daemon=True).start()
    try:
        ready = json.loads(lines.get(timeout=timeout))
        validate_ready(ready)
        write_json(directory / "fixture.ready.json", ready)
        return process, error_log, ready
    except Exception:
        stop(process)
        error_log.close()
        raise


def validate_ready(ready):
    url = urlparse(ready["api_url"])
    if (url.scheme != "http" or url.hostname != "127.0.0.1" or not url.port
            or ready.get("repo") != REPO
            or ready.get("authentication") != "disabled_synthetic_fixture_only"
            or ready.get("model_advice") != "disabled"):
        raise ValueError("Only explicit loopback synthetic fixture ready metadata is accepted")


def allowed_call(params, nonce, memory_id):
    args = params.get("arguments", {})
    if params.get("name") not in TOOLS or args.get("repo") != REPO:
        return False
    if args.get("include_global", False):
        return False
    if params["name"] == "cortex_capture_memory":
        return (args.get("memory_id") == memory_id and args.get("kind") == "decision"
                and args.get("body") == f"Synthetic harness decision {nonce}."
                and args.get("status") == "accepted"
                and set(args) <= {"repo", "memory_id", "kind", "body", "status"})
    return (args.get("query") == nonce and args.get("limit", 5) <= 5
            and args.get("budget_bytes", 1536) <= 1536
            and set(args) <= {"repo", "query", "limit", "budget_bytes", "include_global"})


def proxy(args):
    """JSONL MCP relay; client-visible tool results stay byte-for-byte intact."""
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("CORTEX_") and k not in SECRET_NAMES}
    env["CORTEX_CONFIG_FILE"] = str(Path(args.journal).parent / "cortex-empty.toml")
    process = subprocess.Popen([args.mcp_binary, "--api-url", args.api_url, "serve"],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, env=env)
    lock = threading.Lock()
    pending = {}
    journal = open(args.journal, "a", encoding="utf-8", buffering=1)

    def record(direction, frame):
        with lock:
            journal.write(json.dumps(redact({"time": time.time(), "direction": direction,
                                            "frame": frame}), ensure_ascii=False) + "\n")

    def send(frame):
        with lock:
            sys.stdout.buffer.write(json.dumps(frame, ensure_ascii=False).encode() + b"\n")
            sys.stdout.buffer.flush()

    def requests():
        for line in sys.stdin.buffer:
            frame = json.loads(line)
            record("client_to_proxy", frame)
            method = frame.get("method", "")
            if method == "tools/call" and not allowed_call(
                    frame.get("params", {}), args.nonce, args.memory_id):
                denied = {"jsonrpc": "2.0", "id": frame.get("id"),
                          "error": {"code": -32602, "message": "Synthetic harness scope only"}}
                record("proxy_to_client", denied)
                send(denied)
                continue
            # No resources, prompts or arbitrary method extensions in this probe.
            if method not in {"initialize", "notifications/initialized", "tools/list",
                              "tools/call", "ping", "notifications/cancelled"}:
                if "id" in frame:
                    denied = {"jsonrpc": "2.0", "id": frame["id"],
                              "error": {"code": -32601, "message": "Probe method unavailable"}}
                    record("proxy_to_client", denied)
                    send(denied)
                continue
            if "id" in frame:
                pending[json.dumps(frame["id"])] = method
            process.stdin.write(line)
            process.stdin.flush()
        process.stdin.close()

    def errors():
        for line in process.stderr:
            record("server_stderr", line.decode("utf-8", errors="replace").rstrip())

    threading.Thread(target=requests, daemon=True).start()
    threading.Thread(target=errors, daemon=True).start()
    try:
        for line in process.stdout:
            frame = json.loads(line)
            record("server_to_proxy", frame)
            if pending.get(json.dumps(frame.get("id"))) == "tools/list" and "result" in frame:
                frame["result"]["tools"] = [tool for tool in frame["result"].get("tools", [])
                                             if tool.get("name") in TOOLS]
                record("proxy_to_client", frame)
                send(frame)
            else:
                # Exact server bytes, including text-encoded JSON tool content.
                record("proxy_to_client", frame)
                with lock:
                    sys.stdout.buffer.write(line)
                    sys.stdout.buffer.flush()
        return process.wait()
    finally:
        stop(process)
        journal.close()


def payload(frame):
    result = frame.get("result", {})
    if "error" in frame or result.get("isError"):
        return None
    for item in result.get("content", []):
        if item.get("type") == "text":
            try:
                return json.loads(item["text"])
            except (KeyError, json.JSONDecodeError):
                pass
    return None


def validate_journal(path, nonce, memory_id):
    requests, observed = {}, {}
    capture_before_read = False
    if not path.exists():
        return {"passed": False, "reason": "No MCP journal: client did not connect"}
    for line in path.read_text(encoding="utf-8").splitlines():
        event = json.loads(line)
        frame = event["frame"]
        if not isinstance(frame, dict):
            continue
        key = json.dumps(frame.get("id"))
        if event["direction"] == "client_to_proxy" and frame.get("method") == "tools/call":
            requests[key] = frame.get("params", {})
            if requests[key].get("name") == "cortex_memory_read":
                capture_before_read = "cortex_capture_memory" in observed
        if event["direction"] == "proxy_to_client" and key in requests:
            params = requests[key]
            value = payload(frame)
            if value is not None and allowed_call(params, nonce, memory_id):
                observed[params["name"]] = value
    capture = observed.get("cortex_capture_memory")
    read = observed.get("cortex_memory_read", {})
    memories = read.get("memories", [])
    exact = any(item.get("id") == memory_id and item.get("version") == 1 for item in memories)
    context = read.get("context", "")
    checks = {"capture_returned": capture is not None,
              "capture_exact_committed_id_version": bool(capture)
              and capture.get("memory_id") == memory_id and capture.get("version") == 1
              and capture.get("durability") == "local_committed",
              "capture_returned_before_read_request": capture_before_read,
              "exact_id_version_returned": exact,
              "nonce_and_citation_in_context": nonce in context and memory_id in context,
              "context_within_1536_bytes": len(context.encode("utf-8")) <= 1536,
              "delivery_equals_query_id": bool(read.get("delivery_id"))
              and read.get("delivery_id") == read.get("query_id")}
    return {"passed": all(checks.values()), "checks": checks,
            "returned_tool_payloads": observed,
            "evidence_scope": "Probe scope: real client tool transport; receipt is prepared, not proof of general model compliance"}


def reported_usage(stdout):
    """Keep actual CLI stream metrics, without estimating an absent dollar cost."""
    metrics = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        if not isinstance(event, dict):
            continue
        fields = {key: event[key] for key in ("usage", "modelUsage", "total_cost_usd", "cost_usd")
                  if key in event}
        if fields:
            metrics.append({"event_type": event.get("type"), **fields})
    return metrics or "unavailable: CLI stream reported no recognized usage fields"


def client_command(args, directory, ready, server):
    env = os.environ.copy()
    secret = env.get("CORTEX_LIVE_ROUTER_KEY")
    if not secret and not args.native_auth:
        raise ValueError("Set private CORTEX_LIVE_ROUTER_KEY environment variable before --run")
    if args.native_auth:
        # Let the installed client resolve its own configured login/provider.
        # Do not inspect, copy, extract or inject credential stores.
        env.pop("CORTEX_LIVE_ROUTER_KEY", None)
    prompt = (f"Use only Cortex MCP tools. First call cortex_capture_memory with exactly "
              f"{json.dumps({'repo': REPO, 'kind': 'decision', 'body': f'Synthetic harness decision {args.nonce}.', 'memory_id': args.memory_id, 'status': 'accepted'})}. "
              f"After success call cortex_memory_read with exactly "
              f"{json.dumps({'repo': REPO, 'query': args.nonce, 'limit': 5, 'budget_bytes': 1536})}. "
              "Do not call any other tools. Finish with returned memory ID, version and delivery_id; under 80 words.")
    write_json(directory / "prompt.json", {"prompt": prompt})
    executable = args.client_binary or shutil.which(args.client)
    if not executable:
        raise ValueError(f"Client executable unavailable: {args.client}; use --client-binary")
    mcp = directory / "mcp.json"
    write_json(mcp, {"mcpServers": {"cortex": server}})
    system = "Run this bounded synthetic MCP acceptance probe exactly. No filesystem or shell work."
    if args.client == "claude":
        if not args.native_auth:
            env.update(ANTHROPIC_AUTH_TOKEN=secret, ANTHROPIC_BASE_URL=args.router_url.rstrip("/"))
            env.pop("ANTHROPIC_API_KEY", None)
        command = [executable, "-p", prompt, "--bare", "--strict-mcp-config", "--mcp-config", str(mcp),
                   "--tools", "", "--allowedTools", *[f"mcp__cortex__{t}" for t in sorted(TOOLS)],
                   "--system-prompt", system, "--model", args.model, "--permission-mode", "dontAsk",
                   "--no-session-persistence", "--verbose", "--output-format", "stream-json", "--effort", "low"]
        if args.max_budget_usd is not None:
            command += ["--max-budget-usd", str(args.max_budget_usd)]
    elif args.client == "codex":
        provider = {"name": "Synthetic acceptance router", "base_url": args.router_url.rstrip("/") + "/v1",
                    "env_key": "CORTEX_LIVE_ROUTER_KEY", "wire_api": "responses",
                    "requires_openai_auth": False, "supports_websockets": False}
        # -c accepts TOML values; a TOML inline table has '=' rather than JSON ':'.
        inline = "{ " + ", ".join(f"{k} = {json.dumps(v)}" if not isinstance(v, bool)
                                  else f"{k} = {str(v).lower()}" for k, v in provider.items()) + " }"
        overrides = {"mcp_servers.cortex.command": json.dumps(server["command"]),
                     "mcp_servers.cortex.args": json.dumps(server["args"]),
                     "mcp_servers.cortex.enabled": "true"}
        # Native auth keeps other installed configuration, including other MCP
        # entries. Table overrides merge; they do not isolate that catalog.
        if not args.native_auth:
            overrides.update(model_provider=json.dumps("live_probe"))
            overrides["model_providers.live_probe"] = inline
        for tool in TOOLS:
            overrides[f"mcp_servers.cortex.tools.{tool}.approval_mode"] = json.dumps("approve")
        command = [executable, "--ask-for-approval", "never", "--no-daemon", "exec",
                   "--ignore-rules", "--ephemeral", "--skip-git-repo-check",
                   "--sandbox", "read-only", "--cd", ready["cwd"], "--model", args.model,
                   "--json", "--output-last-message", str(directory / "last-message.txt")]
        if not args.native_auth:
            command.append("--ignore-user-config")
        for key, value in overrides.items():
            command += ["-c", f"{key}={value}"]
        command.append(prompt)
    else:
        if args.native_auth:
            project = Path(ready["cwd"]) / ".kimi-code"
            project.mkdir(exist_ok=True)
            write_json(project / "mcp.json", {"mcpServers": {"cortex": server}})
            env.update(KIMI_CODE_TRUST_WORKSPACE="1", KIMI_DISABLE_TELEMETRY="1",
                       KIMI_CODE_NO_AUTO_UPDATE="1", KIMI_DISABLE_CRON="1",
                       KIMI_LOOP_MAX_STEPS_PER_TURN="6", KIMI_LOOP_MAX_ATTEMPTS_PER_STEP="1",
                       KIMI_CODE_INFINITE_RETRY="0")
            return [executable, "-p", prompt, "--model", args.model,
                    "--output-format", "stream-json"], env
        home = directory / "kimi-home"
        home.mkdir()
        write_json(home / "mcp.json", {"mcpServers": {"cortex": server}})
        (home / "config.toml").write_text("\n".join(
            f'[[permission.rules]]\ndecision = "allow"\npattern = "mcp__cortex__{tool}"\n'
            for tool in sorted(TOOLS)), encoding="utf-8")
        env.update(KIMI_CODE_HOME=str(home), KIMI_CODE_TRUST_WORKSPACE="1",
                   KIMI_MODEL_NAME=args.model, KIMI_MODEL_API_KEY=secret,
                   KIMI_MODEL_PROVIDER_TYPE="kimi", KIMI_MODEL_BASE_URL=args.router_url.rstrip("/") + "/v1",
                   KIMI_MODEL_MAX_COMPLETION_TOKENS="1536", KIMI_LOOP_MAX_STEPS_PER_TURN="6",
                   KIMI_LOOP_MAX_ATTEMPTS_PER_STEP="1", KIMI_CODE_INFINITE_RETRY="0",
                   KIMI_DISABLE_TELEMETRY="1", KIMI_CODE_NO_AUTO_UPDATE="1", KIMI_DISABLE_CRON="1")
        command = [executable, "-p", prompt, "--output-format", "stream-json"]
    return command, env


def run(args):
    directory = Path(tempfile.mkdtemp(prefix=f"cortex-{args.client}-memory-probe-"))
    print(f"Artifacts: {directory}", flush=True)
    args.nonce = "canary" + uuid.uuid4().hex
    args.memory_id = "probe-" + uuid.uuid4().hex
    (directory / "cortex-empty.toml").write_text("# Synthetic MCP probe; no operator config.\n", encoding="utf-8")
    fixture = error_log = None
    try:
        if args.fixture_binary:
            fixture, error_log, ready = fixture_ready(Path(args.fixture_binary).resolve(), directory)
        else:
            ready = json.loads(Path(args.ready_json).read_text(encoding="utf-8"))
            validate_ready(ready)
            write_json(directory / "fixture.ready.json", ready)
        journal = directory / "mcp.jsonl"
        server = {"command": sys.executable, "args": [str(Path(__file__).resolve()), "--proxy",
                  "--mcp-binary", str(Path(args.mcp_binary).resolve()), "--api-url", ready["api_url"],
                  "--journal", str(journal), "--nonce", args.nonce, "--memory-id", args.memory_id]}
        command, env = client_command(args, directory, ready, server)
        version = subprocess.run([command[0], "--version"], env=env,
                                 capture_output=True, timeout=15, check=False)
        write_json(directory / "client-version.json", {"exit_code": version.returncode,
                   "stdout": version.stdout.decode("utf-8", errors="replace"),
                   "stderr": version.stderr.decode("utf-8", errors="replace")})
        write_json(directory / "invocation.json", {"argv": command, "cwd": ready["cwd"],
                   "model": args.model, "timeout_seconds": args.timeout,
                   "auth_source": "native client" if args.native_auth else "private caller environment",
                   "spend": "actual CLI-reported usage only; unavailable if not reported",
                   "attempts": 1, "nonce": args.nonce, "memory_id": args.memory_id})
        process = subprocess.Popen(command, cwd=ready["cwd"], env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        timed_out = False
        try:
            stdout, stderr = process.communicate(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            stop(process)
            stdout, stderr = process.communicate()
        for name, data in (("client.stdout.log", stdout), ("client.stderr.log", stderr)):
            (directory / name).write_text(redact(data.decode("utf-8", errors="replace")), encoding="utf-8")
        last_message = directory / "last-message.txt"
        if last_message.exists():
            last_message.write_text(redact(last_message.read_text(encoding="utf-8")), encoding="utf-8")
        report = validate_journal(journal, args.nonce, args.memory_id)
        report.update(client=args.client, requested_model=args.model,
                      exit_code=process.returncode, timed_out=timed_out,
                      reported_usage=reported_usage(stdout))
        report["passed"] = report["passed"] and process.returncode == 0 and not timed_out
        write_json(directory / "report.json", report)
        print(json.dumps(redact(report), ensure_ascii=False), flush=True)
        return 0 if report["passed"] else 1
    finally:
        if fixture is not None:
            stop(fixture)
            error_log.close()


def self_test():
    nonce, memory_id = "canarytest", "probe-test"
    capture = {"name": "cortex_capture_memory", "arguments": {"repo": REPO, "kind": "decision",
               "body": f"Synthetic harness decision {nonce}.", "memory_id": memory_id, "status": "accepted"}}
    assert allowed_call(capture, nonce, memory_id)
    assert not allowed_call({**capture, "arguments": {**capture["arguments"], "repo": "operator"}}, nonce, memory_id)
    assert not allowed_call({**capture, "name": "cortex_execute"}, nonce, memory_id)
    directory = Path(tempfile.mkdtemp(prefix="cortex-probe-offline-"))
    path = directory / "mcp.jsonl"
    read = {"name": "cortex_memory_read", "arguments": {"repo": REPO, "query": nonce}}
    result = {"context": f"[memory {memory_id} v1]\n{nonce}",
              "memories": [{"id": memory_id, "version": 1}], "delivery_id": "d1", "query_id": "d1"}
    events = []
    receipt = {"memory_id": memory_id, "version": 1, "durability": "local_committed"}
    for rpc_id, params, value in ((1, capture, receipt), (2, read, result)):
        events += [{"direction": "client_to_proxy", "frame": {"id": rpc_id, "method": "tools/call", "params": params}},
                   {"direction": "proxy_to_client", "frame": {"id": rpc_id, "result": {"isError": False,
                    "content": [{"type": "text", "text": json.dumps(value)}]}}}]
    path.write_text("\n".join(json.dumps(event) for event in events), encoding="utf-8")
    assert validate_journal(path, nonce, memory_id)["passed"]
    result["memories"][0]["version"] = 2
    events[-1]["frame"]["result"]["content"][0]["text"] = json.dumps(result)
    path.write_text("\n".join(json.dumps(event) for event in events), encoding="utf-8")
    assert not validate_journal(path, nonce, memory_id)["passed"]
    result["memories"][0]["version"] = 1
    events[-1]["frame"]["result"]["content"][0]["text"] = json.dumps(result)
    events[-1]["frame"]["result"]["isError"] = True
    path.write_text("\n".join(json.dumps(event) for event in events), encoding="utf-8")
    assert not validate_journal(path, nonce, memory_id)["passed"]
    assert reported_usage(b'{"type":"turn.completed","usage":{"input_tokens":2}}')[0]["usage"]["input_tokens"] == 2
    print(f"Offline checks passed; retained synthetic journal: {directory}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", choices=["claude", "codex", "kimi"])
    parser.add_argument("--model")
    parser.add_argument("--client-binary")
    parser.add_argument("--native-auth", action="store_true",
                        help="Use installed client's own login/provider without reading credentials")
    parser.add_argument("--mcp-binary")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--fixture-binary")
    source.add_argument("--ready-json")
    parser.add_argument("--router-url", default="http://127.0.0.1:20128")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--max-budget-usd", type=float, help="Optional Claude CLI-supported cap only")
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--proxy", action="store_true", help=argparse.SUPPRESS)
    for flag in ("api-url", "journal", "nonce", "memory-id"):
        parser.add_argument("--" + flag, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.proxy:
        return proxy(args)
    if args.self_test:
        return self_test()
    if not args.run:
        parser.print_help()
        print("Preparation only. Pass --run explicitly to invoke one installed client/model.")
        return 0
    if not all((args.client, args.model, args.mcp_binary, args.fixture_binary or args.ready_json)):
        parser.error("--run requires --client, --model, --mcp-binary and --fixture-binary or --ready-json")
    if not 1 <= args.timeout <= 300:
        parser.error("--timeout must be between 1 and 300 seconds")
    return run(args)


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    try:
        sys.exit(main())
    except (OSError, ValueError, queue.Empty, subprocess.TimeoutExpired) as error:
        print(redact(f"Harness failed: {type(error).__name__}: {error}"), file=sys.stderr)
        sys.exit(2)

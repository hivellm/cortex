#!/usr/bin/env python3
"""Optional real Claude SessionStart probe; native client auth, synthetic fixture.

Default is help only. A wrapper journals the packaged hook's real input/output,
then forwards stdout unchanged. The seeded nonce is absent from the user prompt.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

from daily_memory_live_harness import fixture_ready, redact, reported_usage, stop, write_json


def isolated_env(directory):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith("CORTEX_") and key not in {
               "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "KIMI_MODEL_API_KEY"}}
    env.update(HOME=str(directory), USERPROFILE=str(directory),
               CORTEX_CONFIG_FILE=str(directory / "runtime.toml"))
    return env


def wrapper(state_file):
    state = json.loads(Path(state_file).read_text(encoding="utf-8"))
    directory = Path(state["directory"])
    raw_input = sys.stdin.buffer.read()
    (directory / "hook.input.json").write_bytes(raw_input)
    env = isolated_env(directory)
    env.update(CORTEX_ADAPTER_PIPE=state["binding"], CORTEX_ADAPTER_SOCK=state["binding"],
               CORTEX_HOOK_BIN=state["hook_binary"],
               CORTEX_HOOK_STATE_DIR=str(directory / "notice-state"))
    output = subprocess.run([state["bash"], state["packaged_hook"]], input=raw_input,
                            capture_output=True, env=env, timeout=5, check=False)
    (directory / "hook.stdout.json").write_bytes(output.stdout)
    (directory / "hook.stderr.log").write_text(
        redact(output.stderr.decode("utf-8", errors="replace")), encoding="utf-8")
    write_json(directory / "hook.exit.json", {"exit_code": output.returncode})
    sys.stdout.buffer.write(output.stdout)
    sys.stdout.buffer.flush()
    return output.returncode


def quoted(path):
    # Paths are local executable/artifact paths, accepted by Bash and cmd.
    value = str(path).replace("\\", "/")
    if '"' in value:
        raise ValueError("Quote character in hook command path")
    return '"' + value + '"'


def run(args):
    directory = Path(tempfile.mkdtemp(prefix="cortex-claude-lifecycle-probe-"))
    print(f"Artifacts: {directory}", flush=True)
    binaries = {}
    for name, source in (("fixture", args.fixture_binary), ("adapter", args.adapter_binary),
                         ("hook", args.hook_binary)):
        target = directory / Path(source).name
        shutil.copy2(source, target)
        binaries[name] = {"path": str(target), "source": str(Path(source).resolve()),
                          "sha256": hashlib.sha256(target.read_bytes()).hexdigest()}
    fixture, fixture_error, ready = fixture_ready(Path(binaries["fixture"]["path"]), directory)
    adapter = None
    files = []
    try:
        nonce, memory_id = "hookcanary" + uuid.uuid4().hex, "lifecycle-" + uuid.uuid4().hex
        capture = {"repo": ready["repo"], "kind": "decision", "memory_id": memory_id,
                   "title": "Synthetic lifecycle probe", "pinned": True, "status": "accepted",
                   "body": f"Approved synthetic lifecycle probe marker: {nonce}."}
        request = urllib.request.Request(ready["api_url"] + "/v1/ingest",
                                         data=json.dumps(capture).encode(),
                                         headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:
            receipt = json.load(response)
        write_json(directory / "seed.json", {"request": capture, "receipt": receipt})
        binding = "\\\\.\\pipe\\cortex-phase34-" + uuid.uuid4().hex
        runtime = directory / "runtime.toml"
        runtime.write_text("[adapter]\nadapter_admin_port=0\n", encoding="utf-8")
        config = directory / "adapter.toml"
        config.write_text(f"[adapter]\napi_endpoint='{ready['api_url']}'\nendpoint='http://127.0.0.1:1'\n",
                          encoding="utf-8")
        files = [(directory / name).open("wb") for name in ("adapter.stdout.log", "adapter.stderr.log")]
        adapter = subprocess.Popen([binaries["adapter"]["path"], "--config", str(config), "--bind", binding],
                                   env=isolated_env(directory), cwd=directory,
                                   stdout=files[0], stderr=files[1])
        time.sleep(0.5)
        if adapter.poll() is not None:
            raise ValueError("Synthetic adapter exited before Claude launch; inspect adapter logs")
        state = {"directory": str(directory), "binding": binding,
                 "hook_binary": binaries["hook"]["path"], "bash": args.bash,
                 "packaged_hook": str(Path(args.packaged_hook).resolve())}
        state_file = directory / "wrapper-state.json"
        write_json(state_file, state)
        hook_command = (quoted(sys.executable) + " " + quoted(Path(__file__).resolve())
                        + " --hook-wrapper " + quoted(state_file))
        settings = directory / "claude-settings.json"
        write_json(settings, {"hooks": {"SessionStart": [{"matcher": "*", "hooks": [
                   {"type": "command", "command": hook_command, "timeout": 5}]}]}})
        mcp = directory / "empty-mcp.json"
        write_json(mcp, {"mcpServers": {}})
        prompt = ("What is approved synthetic lifecycle probe marker from session-start context? "
                  "Return exact marker only. If context lacks marker, reply MISSING.")
        command = [args.claude_binary, "-p", prompt, "--setting-sources", "", "--settings", str(settings),
                   "--strict-mcp-config", "--mcp-config", str(mcp), "--tools", "",
                   "--system-prompt", "Answer only from supplied session-start context. Do not invent a marker or use tools.",
                   "--model", args.model, "--permission-mode", "dontAsk", "--no-session-persistence",
                   "--verbose", "--output-format", "stream-json", "--effort", "low",
                   "--max-budget-usd", str(args.max_budget_usd)]
        assert nonce not in prompt and nonce not in json.dumps(command)
        write_json(directory / "invocation.json", {"argv": command, "cwd": ready["cwd"],
                   "binaries": binaries, "packaged_hook_sha256": hashlib.sha256(
                       Path(args.packaged_hook).read_bytes()).hexdigest(),
                   "auth_source": "native client; no credential-store access",
                   "nonce_absent_from_user_prompt": True, "timeout_seconds": args.timeout})
        env = os.environ.copy()
        env.pop("CORTEX_LIVE_ROUTER_KEY", None)
        process = subprocess.Popen(command, cwd=ready["cwd"], env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        timed_out = False
        try:
            stdout, stderr = process.communicate(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            stop(process)
            stdout, stderr = process.communicate()
        for name, value in (("client.stdout.log", stdout), ("client.stderr.log", stderr)):
            (directory / name).write_text(redact(value.decode("utf-8", errors="replace")), encoding="utf-8")
        hook_stdout = directory / "hook.stdout.json"
        hook_value = json.loads(hook_stdout.read_text(encoding="utf-8")) if hook_stdout.exists() else {}
        context = hook_value.get("hookSpecificOutput", {}).get("additionalContext", "")
        model_results = []
        for line in stdout.splitlines():
            try:
                event = json.loads(line)
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            if event.get("type") == "result":
                model_results.append(event.get("result", ""))
        # This database was created by this run's fixture, never a user store.
        database_path = Path(ready["db_path"].removeprefix("\\\\?\\"))
        with sqlite3.connect(database_path.as_uri() + "?mode=ro", uri=True) as database:
            deliveries = [json.loads(row[0]) for row in database.execute(
                "SELECT data FROM daily_memory_deliveries WHERE repo=?", (ready["repo"],))]
        write_json(directory / "prepared-deliveries.json", deliveries)
        checks = {"seed_committed": receipt.get("memory_id") == memory_id and receipt.get("version") == 1,
                  "exact_persistent_prepared_receipt": any(
                      delivery.get("stage") == "prepared" and any(
                          item.get("id") == memory_id and item.get("version") == 1
                          for item in delivery.get("request", {}).get("memories", []))
                      for delivery in deliveries),
                  "real_packaged_hook_returned_nonce": nonce in context and memory_id in context,
                  "model_result_contains_nonce_absent_from_prompt": any(nonce in result for result in model_results),
                  "client_completed": process.returncode == 0 and not timed_out}
        report = {"passed": all(checks.values()), "checks": checks, "memory_id": memory_id,
                  "nonce": nonce, "model_results": model_results, "exit_code": process.returncode,
                  "timed_out": timed_out, "reported_usage": reported_usage(stdout),
                  "evidence_scope": "Probe scope: actual Claude SessionStart context delivery; no general recall/compliance claim"}
        write_json(directory / "report.json", report)
        print(json.dumps(redact(report), ensure_ascii=False), flush=True)
        return 0 if report["passed"] else 1
    finally:
        if adapter is not None:
            stop(adapter)
        for file in files:
            file.close()
        stop(fixture)
        fixture_error.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hook-wrapper", help=argparse.SUPPRESS)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--fixture-binary")
    parser.add_argument("--adapter-binary")
    parser.add_argument("--hook-binary")
    parser.add_argument("--claude-binary", default="claude")
    parser.add_argument("--bash", default="C:/Program Files/Git/bin/bash.exe")
    parser.add_argument("--packaged-hook", default="packages/cortex-claude-plugin/hooks/cortex-session-start.sh")
    parser.add_argument("--model", default="claude-fable-5-1")
    parser.add_argument("--max-budget-usd", type=float, default=3)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if args.hook_wrapper:
        return wrapper(args.hook_wrapper)
    if not args.run:
        parser.print_help()
        return 0
    if not all((args.fixture_binary, args.adapter_binary, args.hook_binary)):
        parser.error("--run requires --fixture-binary, --adapter-binary and --hook-binary")
    if not 1 <= args.timeout <= 300:
        parser.error("--timeout must be between 1 and 300 seconds")
    return run(args)


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(redact(f"Hook probe failed: {type(error).__name__}: {error}"), file=sys.stderr)
        sys.exit(2)

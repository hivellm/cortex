#!/usr/bin/env python3
"""Review or apply narrow daily Cortex client edits; dry-run unless --apply.

Never opens provider credential stores or changes Codex hook trust. Backups stay next
to private client files. Expected-state manifests contain paths/hashes, no config.
"""

import argparse
import copy
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
import tomllib


EVENTS = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse",
          "Stop", "SubagentStop", "Notification")


def digest(raw):
    return hashlib.sha256(raw).hexdigest() if raw is not None else None


def encoded(value):
    return json.dumps(value, ensure_ascii=False)


def toml_table(text, name, values):
    """Patch single-line values only; preserve every other TOML byte."""
    lines = text.splitlines(keepends=True)
    headers = [(i, match.group(1).strip()) for i, line in enumerate(lines)
               if (match := re.match(r"\s*\[([^\[\]]+)\]\s*(?:#.*)?$", line))]
    starts = [i for i, key in headers if key == name]
    if len(starts) > 1:
        raise ValueError(f"Duplicate TOML table: {name}")
    newline = "\r\n" if "\r\n" in text else "\n"
    if not starts:
        return text + (newline if text and not text.endswith("\n") else "") + (
            f"[{name}]" + newline + "".join(f"{k} = {encoded(v)}{newline}"
                                           for k, v in values.items()))
    start = starts[0]
    end = next((i for i, _ in headers if i > start), len(lines))
    remaining = dict(values)
    for i in range(start + 1, end):
        match = re.match(r"\s*([A-Za-z_][A-Za-z0-9_]*)\s*=", lines[i])
        if match and match.group(1) in remaining:
            key = match.group(1)
            # Refuse unusual/multiline syntax rather than corrupting it.
            tomllib.loads(lines[i])
            lines[i] = f"{key} = {encoded(remaining.pop(key))}{newline}"
    lines[end:end] = [f"{k} = {encoded(v)}{newline}" for k, v in remaining.items()]
    return "".join(lines)


def patch_mcp(server, command, env):
    if server.get("type", "stdio") != "stdio" or server.get("url"):
        raise ValueError("Existing Cortex server is not stdio")
    server.update(command=command, args=["serve"])
    server.setdefault("env", {}).update(env)


def patch_hooks(data, hook, pipe):
    if any(v for k, v in data.get("enabledPlugins", {}).items() if "cortex" in k.lower()):
        raise ValueError("Enabled Cortex plugin may duplicate user hooks; review first")
    for event in EVENTS:
        found = []
        for group in data.get("hooks", {}).get(event, []):
            found.extend(h for h in group.get("hooks", [])
                         if "cortex" in h.get("command", "").lower())
        if len(found) != 1:
            raise ValueError(f"Expected exactly one existing Cortex {event} hook")
        # Git Bash consumes backslashes inside double quotes before native argv.
        # Use the Win32 slash spelling only in hook commands, never MCP env/config.
        hook_pipe = pipe.replace("\\", "/") if pipe.startswith("\\\\.\\pipe\\") else pipe
        command = f'"{hook}" {event} --pipe "{hook_pipe}"'
        previous_pinned = f'"{hook}" {event} --pipe "{pipe}"'
        if event not in ("SessionStart", "UserPromptSubmit", "PreToolUse"):
            command += " --fire-forget"
            previous_pinned += " --fire-forget"
        old = found[0]["command"]
        if old not in (command, previous_pinned) and not re.fullmatch(
                rf"cortex-hook {event}(?: --fire-forget)?", old):
            raise ValueError(f"Unexpected Cortex {event} command; review first")
        found[0]["command"] = command


def plan(args):
    home, repo, binary = args.home.resolve(), args.repo.resolve(), args.bin_dir.resolve()
    runtime = args.runtime_config.resolve()
    if not runtime.is_file():
        raise ValueError(f"Runtime config missing: {runtime}")
    runtime_data = tomllib.loads(runtime.read_text(encoding="utf-8-sig"))
    if runtime_data.get("runtime", {}).get("profile") != "daily":
        raise ValueError("Runtime config must explicitly select daily")
    bins = {name: binary / name for name in ("cortex-mcp-server.exe", "cortex-hook.exe")}
    for path in bins.values():
        if not path.is_file():
            raise ValueError(f"Pinned binary missing: {path}")
    command = bins["cortex-mcp-server.exe"].as_posix()
    env = {"CORTEX_API_URL": args.api_url, "CORTEX_RUNTIME_PROFILE": "daily",
           "CORTEX_CONFIG_FILE": runtime.as_posix(), "CORTEX_ADAPTER_PIPE": args.pipe,
           "CORTEX_ADAPTER_SOCK": args.pipe}
    paths = [(home / ".claude.json", "mcp"), (repo / ".mcp.json", "mcp"),
             (home / ".claude/settings.json", "hooks"),
             (home / ".codex/config.toml", "toml"),
             (home / ".kimi-code/mcp.json", "mcp")]
    changes = []
    for path, kind in paths:
        before = path.read_bytes() if path.exists() else None
        if before is None and path != home / ".kimi-code/mcp.json":
            raise ValueError(f"Expected existing client file: {path}")
        text = before.decode("utf-8-sig") if before is not None else "{}"
        if kind == "toml":
            original = tomllib.loads(text)
            if "cortex" not in original.get("mcp_servers", {}):
                raise ValueError("Expected existing Codex Cortex entry")
            expected = copy.deepcopy(original)
            patch_mcp(expected["mcp_servers"]["cortex"], command, env)
            revised = toml_table(text, "mcp_servers.cortex", {"command": command, "args": ["serve"]})
            revised = toml_table(revised, "mcp_servers.cortex.env", env)
            if tomllib.loads(revised) != expected:
                raise ValueError("Codex semantic guard failed: unrelated setting changed")
            after = revised.encode("utf-8")
        else:
            original = json.loads(text)
            expected = copy.deepcopy(original)
            if kind == "hooks":
                patch_hooks(expected, bins["cortex-hook.exe"].as_posix(), args.pipe)
            else:
                servers = expected.setdefault("mcpServers", {})
                patch_mcp(servers.setdefault("cortex", {}), command, env)
            # Preserve existing bytes when already active (idempotent reruns).
            after = before if expected == original else (
                json.dumps(expected, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
        changes.append((path, before, after))
    manifest = {"version": 1,
                "binaries": {str(path): digest(path.read_bytes()) for path in bins.values()},
                "runtime": {"path": str(runtime), "sha256": digest(runtime.read_bytes())},
                "files": [{"path": str(path), "before_sha256": digest(before),
                           "after_sha256": digest(after)} for path, before, after in changes]}
    return changes, manifest


def apply(changes, expected):
    # Validate every file before first write, then repeat immediately per file.
    for path, before, _ in changes:
        current = path.read_bytes() if path.exists() else None
        if current != before:
            raise ValueError(f"Client file changed during review: {path}")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    for path, before, after in changes:
        if before == after:
            continue
        current = path.read_bytes() if path.exists() else None
        if current != before:
            raise ValueError(f"Client file changed before write: {path}")
        path.parent.mkdir(parents=True, exist_ok=True)
        if before is not None:
            backup = path.with_name(path.name + f".cortex-daily-{stamp}.bak")
            if backup.exists():
                raise ValueError(f"Backup already exists: {backup}")
            shutil.copy2(path, backup)
            if backup.read_bytes() != before:
                raise ValueError(f"Backup verification failed: {backup}")
        # Write existing file in place to preserve its inherited/native ACL.
        path.write_bytes(after)
        if digest(path.read_bytes()) != expected[str(path)]:
            raise ValueError(f"Post-write verification failed: {path}")
        print(json.dumps({"updated": str(path), "backup_created": before is not None}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path, default=Path("C:/Users/Bolado"))
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--runtime-config", type=Path, required=True)
    parser.add_argument("--api-url", default="http://127.0.0.1:17000")
    parser.add_argument("--pipe", default=r"\\.\pipe\cortex-adapter-claude")
    parser.add_argument("--write-manifest", type=Path)
    parser.add_argument("--expected-manifest", type=Path)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    changes, manifest = plan(args)
    if args.apply:
        if not args.expected_manifest or args.write_manifest:
            parser.error("--apply requires --expected-manifest, without --write-manifest")
        approved = json.loads(args.expected_manifest.read_text(encoding="utf-8"))
        if approved != manifest:
            raise ValueError("Expected state differs; rerun dry-run and review new manifest")
        apply(changes, {row["path"]: row["after_sha256"] for row in manifest["files"]})
    elif args.write_manifest:
        # Exclusive creation prevents overwriting an already reviewed manifest.
        with args.write_manifest.open("x", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")
    print(json.dumps({"mode": "apply" if args.apply else "dry-run", "manifest": manifest,
                      "codex_hooks_changed": False, "provider_auth_changed": False}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(f"Activation stopped: {error}", file=sys.stderr)
        sys.exit(1)

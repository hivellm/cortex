#!/usr/bin/env python3
"""Remove legacy Cortex client entries once the plugin owns them; dry-run unless --apply.

Phase35 replaced per-client absolute-path activation with the Cortex plugins,
so this script now only migrates away from it. It removes Cortex hook and MCP
entries from ~/.claude/settings.json, ~/.claude.json, ~/.codex/config.toml and
~/.codex/hooks.json, and a hivellm-cortex marketplace registration whose
directory no longer exists. Nothing else in those files changes (checked
semantically before any write), and no other file is touched.

  dry run   : prints the plan and a manifest of path/hash pairs
  --apply   : requires the reviewed manifest (--expected-manifest) and an
              enabled, installed Claude Code plugin; writes hash-verified
              backups and a rollback record under ~/.cortex/migrations/
  --rollback: restores every file of a rollback record byte-for-byte
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

PLUGIN_ID = "cortex@hivellm-cortex"
MARKETPLACE = "hivellm-cortex"


def digest(raw):
    return hashlib.sha256(raw).hexdigest() if raw is not None else None


def legacy_command(command):
    return "CLAUDE_PLUGIN_ROOT" not in command and "PLUGIN_ROOT" not in command and (
        "cortex-hook" in command or "hooks/cortex-" in command)


def strip_hooks(data):
    hooks = data.get("hooks")
    if not isinstance(hooks, dict):
        return []
    removed = []
    for event in list(hooks):
        groups = hooks[event] if isinstance(hooks[event], list) else []
        kept_groups = []
        for group in groups:
            entries = group.get("hooks", []) if isinstance(group, dict) else []
            kept = [h for h in entries if not legacy_command(str(h.get("command", "")))]
            removed += [f"hooks.{event}: {h.get('command')}" for h in entries if h not in kept]
            if kept or not entries:
                kept_groups.append({**group, "hooks": kept} if entries else group)
        if kept_groups:
            hooks[event] = kept_groups
        elif groups:
            del hooks[event]
    if not hooks:
        del data["hooks"]
    return removed


def strip_settings(data):
    removed = strip_hooks(data)
    markets = data.get("extraKnownMarketplaces", {})
    source = markets.get(MARKETPLACE, {}).get("source", {})
    if source.get("source") == "directory" and not Path(source.get("path", "")).exists():
        del markets[MARKETPLACE]
        removed.append(f"extraKnownMarketplaces.{MARKETPLACE}: {source.get('path')}")
        if not markets:
            del data["extraKnownMarketplaces"]
    return removed


def cortex_server(server):
    return isinstance(server, dict) and "cortex-mcp-server" in str(server.get("command", ""))


def strip_claude_json(data):
    removed = []
    scopes = [("mcpServers", data)] + [
        (f"projects.{name}.mcpServers", project)
        for name, project in data.get("projects", {}).items() if isinstance(project, dict)]
    for label, owner in scopes:
        servers = owner.get("mcpServers")
        if isinstance(servers, dict) and cortex_server(servers.get("cortex")):
            del servers["cortex"]
            removed.append(f"{label}.cortex")
    return removed


def strip_codex_toml(text):
    """Remove [mcp_servers.cortex] and its subtables line-wise; bytes elsewhere kept."""
    lines = text.splitlines(keepends=True)
    header = re.compile(r"\s*\[\[?([^\[\]]+)\]\]?\s*(?:#.*)?$")
    out, skipping = [], False
    for line in lines:
        match = header.match(line)
        if match:
            name = match.group(1).strip()
            skipping = name == "mcp_servers.cortex" or name.startswith("mcp_servers.cortex.")
        if not skipping:
            out.append(line)
    revised = "".join(out)
    expected = tomllib.loads(text)
    removed = []
    if "cortex" in expected.get("mcp_servers", {}):
        del expected["mcp_servers"]["cortex"]
        removed.append("mcp_servers.cortex")
    if tomllib.loads(revised) != expected:
        raise ValueError("Codex semantic guard failed: unrelated setting changed")
    return revised, removed


def json_change(before, strip):
    original = json.loads(before.decode("utf-8-sig"))
    revised = copy.deepcopy(original)
    removed = strip(revised)
    if not removed:
        return before, []
    return (json.dumps(revised, indent=2, ensure_ascii=False) + "\n").encode("utf-8"), removed


def plan(home):
    targets = [(home / ".claude/settings.json", lambda b: json_change(b, strip_settings)),
               (home / ".claude.json", lambda b: json_change(b, strip_claude_json)),
               (home / ".codex/hooks.json", lambda b: json_change(b, strip_hooks)),
               (home / ".codex/config.toml", lambda b: (lambda r: (r[0].encode("utf-8"), r[1]))(
                   strip_codex_toml(b.decode("utf-8-sig"))))]
    changes = []
    for path, transform in targets:
        if not path.exists():
            continue
        before = path.read_bytes()
        after, removed = transform(before)
        changes.append({"path": path, "before": before, "after": after, "removed": removed})
    manifest = {"version": 2, "files": [
        {"path": str(c["path"]), "before_sha256": digest(c["before"]),
         "after_sha256": digest(c["after"]), "removed": c["removed"]} for c in changes]}
    return changes, manifest


def plugin_verified(home):
    settings = json.loads((home / ".claude/settings.json").read_text(encoding="utf-8-sig"))
    installed_path = home / ".claude/plugins/installed_plugins.json"
    installed = json.loads(installed_path.read_text(encoding="utf-8-sig")) if installed_path.exists() else {}
    return settings.get("enabledPlugins", {}).get(PLUGIN_ID) is True and bool(
        installed.get("plugins", {}).get(PLUGIN_ID))


def apply(home, changes, stamp):
    for change in changes:
        if change["path"].read_bytes() != change["before"]:
            raise ValueError(f"Client file changed during review: {change['path']}")
    record = {"version": 1, "stamp": stamp, "files": []}
    for change in changes:
        path, before, after = change["path"], change["before"], change["after"]
        if before == after:
            continue
        backup = path.with_name(path.name + f".cortex-migrate-{stamp}.bak")
        if backup.exists():
            raise ValueError(f"Backup already exists: {backup}")
        shutil.copy2(path, backup)
        if digest(backup.read_bytes()) != digest(before):
            raise ValueError(f"Backup verification failed: {backup}")
        if path.read_bytes() != before:
            raise ValueError(f"Client file changed before write: {path}")
        # Write the existing file in place so its native ACL is preserved.
        path.write_bytes(after)
        if digest(path.read_bytes()) != digest(after):
            raise ValueError(f"Post-write verification failed: {path}")
        record["files"].append({"path": str(path), "backup": str(backup),
                                "before_sha256": digest(before), "after_sha256": digest(after)})
    target = home / ".cortex/migrations" / f"clients-{stamp}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("x", encoding="utf-8") as output:
        json.dump(record, output, indent=2)
        output.write("\n")
    return target, record


def rollback(record_path, force=False):
    record = json.loads(Path(record_path).read_text(encoding="utf-8"))
    rows = record["files"]
    for row in rows:
        path, backup = Path(row["path"]), Path(row["backup"])
        if digest(backup.read_bytes()) != row["before_sha256"]:
            raise ValueError(f"Backup no longer matches its recorded hash: {backup}")
        current = digest(path.read_bytes()) if path.exists() else None
        if current != row["after_sha256"] and not force:
            raise ValueError(f"{path} changed since migration; rerun with --force to restore anyway")
    for row in rows:
        path = Path(row["path"])
        path.write_bytes(Path(row["backup"]).read_bytes())
        if digest(path.read_bytes()) != row["before_sha256"]:
            raise ValueError(f"Rollback verification failed: {path}")
    return [row["path"] for row in rows]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--home", type=Path, default=Path.home())
    parser.add_argument("--write-manifest", type=Path)
    parser.add_argument("--expected-manifest", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--rollback", type=Path, metavar="RECORD")
    parser.add_argument("--force", action="store_true", help="rollback even if files changed since")
    args = parser.parse_args(argv)
    home = args.home.resolve()
    if args.rollback:
        restored = rollback(args.rollback, args.force)
        print(json.dumps({"mode": "rollback", "restored": restored}, indent=2))
        return 0
    changes, manifest = plan(home)
    if args.apply:
        if not args.expected_manifest or args.write_manifest:
            parser.error("--apply requires --expected-manifest, without --write-manifest")
        if json.loads(args.expected_manifest.read_text(encoding="utf-8")) != manifest:
            raise ValueError("Expected state differs; rerun the dry run and review the new manifest")
        if not plugin_verified(home):
            raise ValueError(f"Install and enable the {PLUGIN_ID} plugin before migrating")
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        target, record = apply(home, changes, stamp)
        print(json.dumps({"mode": "apply", "rollback_record": str(target),
                          "updated": [row["path"] for row in record["files"]]}, indent=2))
        return 0
    if args.write_manifest:
        with args.write_manifest.open("x", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")
    print(json.dumps({"mode": "dry-run", "manifest": manifest}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(f"Migration stopped: {error}", file=sys.stderr)
        sys.exit(1)

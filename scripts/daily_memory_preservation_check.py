#!/usr/bin/env python3
"""Read-only native-memory hashes and quiescent Cortex SQLite inventory.

Never prints memory bodies, native filenames, repository identities or keys.
SQLite inspection refuses a nonempty WAL: immutable read-only access must not
hide live WAL changes or create shared-memory sidecars. No schema migration.
"""

import argparse
from contextlib import closing
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile


JOB = "retention.memory_consolidate"
DAILY_TABLES = ("daily_memory_records", "daily_memory_evidence", "daily_memory_revisions",
                "daily_memory_deliveries", "daily_memory_feedback", "daily_memory_retries",
                "daily_memory_advice_attempts", "daily_memory_advice_cache", "daily_memory_advice_counters")


def file_hash(path):
    digest = hashlib.sha256()
    before = path.stat()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(128 * 1024), b""):
            digest.update(chunk)
    after = path.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise ValueError("File changed while hashing; retry after writer stops")
    return digest.hexdigest()


def native_inventory(root):
    root = root.resolve(strict=True)
    files = {}
    for path in root.glob("*/memory/*.md"):
        # Only top-level native Markdown files, matching the historical 535 set.
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError("Native memory symlink/outside-root path refused")
        relative = path.relative_to(root).as_posix()
        key = hashlib.sha256(relative.encode("utf-8")).hexdigest()
        files[key] = file_hash(path)
    ordered = dict(sorted(files.items()))
    digest = hashlib.sha256(json.dumps(ordered, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {"count": len(files), "set_sha256": digest, "files": ordered,
            "path_keys": "SHA-256 of relative native path; filenames are not recorded"}


def compare(current, previous):
    old, new = previous["files"], current["files"]
    changed = sum(old[key] != new[key] for key in old.keys() & new.keys())
    added, missing = len(new.keys() - old.keys()), len(old.keys() - new.keys())
    return {"passed": changed == added == missing == 0, "changed": changed,
            "added": added, "missing": missing, "baseline_count": len(old)}


def signature(path):
    if not path.exists():
        return None
    info = path.stat()
    return {"size": info.st_size, "mtime_ns": info.st_mtime_ns}


def sqlite_inventory(path):
    path = path.resolve(strict=True)
    wal = Path(str(path) + "-wal")
    shm = Path(str(path) + "-shm")
    before = {str(p): signature(p) for p in (path, wal, shm)}
    if before[str(wal)] and before[str(wal)]["size"]:
        return {"available": False, "reason": "nonempty_wal_refused_to_avoid_stale_immutable_read",
                "path": str(path), "sidecars": {"wal": before[str(wal)], "shm": before[str(shm)]}}
    result = {"available": True, "path": str(path), "access": "mode=ro&immutable=1; query_only=ON",
              "file_before": before[str(path)]}
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True, timeout=3)) as connection:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        tables = {r[0] for r in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        result["table_count"] = len(tables)
        if "cron_jobs" in tables:
            row = connection.execute(
                "SELECT enabled,last_status,last_run_at,failure_streak FROM cron_jobs WHERE name=?", (JOB,)).fetchone()
            result["scheduler"] = {"row_present": row is not None}
            if row:
                result["scheduler"].update(enabled=row[0], disabled=row[0] == 0,
                    last_status=row[1] if row[1] in (None, "running", "success", "failed") else "other",
                    last_run_at=row[2], failure_streak=row[3])
        else:
            result["scheduler"] = {"row_present": False, "table_present": False}
        result["running_retention_sweeps"] = connection.execute(
            "SELECT count(*) FROM retention_sweeps WHERE status='running'").fetchone()[0] \
            if "retention_sweeps" in tables else None
        result["daily_tables"] = {}
        for table in DAILY_TABLES:
            if table in tables:
                result["daily_tables"][table] = {
                    "rows": connection.execute('SELECT count(*) FROM "' + table + '"').fetchone()[0],
                    "columns": [{"name": r[1], "type": r[2]} for r in connection.execute('PRAGMA table_info("' + table + '")')]}
        if "daily_memory_records" in tables:
            result["daily_record_status_counts"] = dict(connection.execute(
                "SELECT CASE WHEN status IN ('proposed','accepted','superseded','deprecated') THEN status ELSE 'other' END,count(*) "
                "FROM daily_memory_records GROUP BY 1"))
        connection.rollback()
    after = {str(p): signature(p) for p in (path, wal, shm)}
    result["source_files_unchanged_during_read"] = before == after
    if before != after:
        result["available"] = False
        result["reason"] = "concurrent_source_change; snapshot_untrusted"
    return result


def self_test():
    root = Path(tempfile.mkdtemp(prefix="cortex-preservation-offline-"))
    native = root / "projects/example/memory"
    native.mkdir(parents=True)
    (native / "MEMORY.md").write_text("synthetic sample", encoding="utf-8")
    first = native_inventory(root / "projects")
    assert first["count"] == 1 and "example" not in json.dumps(first)
    assert compare(first, first)["passed"]
    (native / "MEMORY.md").write_text("synthetic changed", encoding="utf-8")
    assert compare(native_inventory(root / "projects"), first)["changed"] == 1
    db = root / "metadata.sqlite"
    with sqlite3.connect(db) as c:
        c.execute("CREATE TABLE cron_jobs(name TEXT,enabled INTEGER,last_status TEXT,last_run_at TEXT,failure_streak INTEGER)")
        c.execute("INSERT INTO cron_jobs VALUES (?,0,'success','2026-10-09',0)", (JOB,))
        c.execute("CREATE TABLE retention_sweeps(status TEXT)")
    snapshot = sqlite_inventory(db)
    assert snapshot["scheduler"]["disabled"] and snapshot["running_retention_sweeps"] == 0
    assert snapshot["source_files_unchanged_during_read"] and not snapshot["daily_tables"]
    Path(str(db) + "-wal").write_bytes(b"synthetic nonempty WAL")
    assert not sqlite_inventory(db)["available"]
    print("Offline checks passed; only retained synthetic fixture files written: " + str(root))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-root", type=Path, default=Path.home() / ".claude/projects")
    parser.add_argument("--metadata-db", type=Path, default=Path.home() / ".cortex/metadata.sqlite")
    parser.add_argument("--baseline", type=Path, help="Prior report; compare native hashes without reading memory content into output")
    parser.add_argument("--output", type=Path, help="New artifact path only; refuses overwrite")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    report = {"captured_at": datetime.now(timezone.utc).isoformat(), "mode": "read_only",
              "native": native_inventory(args.native_root), "sqlite": sqlite_inventory(args.metadata_db)}
    if args.baseline:
        baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
        report["comparison"] = compare(report["native"], baseline["native"])
    if args.output:
        with args.output.open("x", encoding="utf-8") as output:
            json.dump(report, output, indent=2)
            output.write("\n")
        summary = {**report, "native": {k: v for k, v in report["native"].items() if k != "files"}, "artifact": str(args.output)}
    else:
        summary = report
    print(json.dumps(summary, ensure_ascii=True))
    return 1 if report.get("comparison", {}).get("passed") is False else 0


if __name__ == "__main__":
    raise SystemExit(main())

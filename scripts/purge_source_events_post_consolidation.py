"""
2026-05-19 — post-consolidation source purge.

Reads `Kind::Consolidation` envelopes directly from the Parquet
archive (where the full `source_event_ids` payload lives — the
Meili projection only stamps `source_event_count`), collects every
referenced source event_id, and calls `/v1/admin/forget` once per
id. The forget endpoint cascades the delete across Vectorizer +
Meili + Nexus + Parquet archive.

Run:
    python scripts/purge_source_events_post_consolidation.py [--dry-run]

Exit 0 on success, 1 on any unrecoverable forget call.
"""
from __future__ import annotations

import argparse
import io
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

import zstandard as zstd

API_URL = os.environ.get("CORTEX_API_URL", "http://127.0.0.1:17000")
TOKEN = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE"
HOME = Path(
    os.environ.get("CORTEX_HOME") or (Path.home() / ".cortex")
) / "archive" / "events"


def iter_envelopes(path: Path):
    raw = path.read_bytes()
    d = zstd.ZstdDecompressor()
    try:
        body = d.decompress(raw)
    except zstd.ZstdError:
        body = d.stream_reader(io.BytesIO(raw)).read()
    for line in body.split(b"\n"):
        if not line.strip():
            continue
        try:
            yield json.loads(line)
        except json.JSONDecodeError:
            continue


def collect_source_event_ids() -> tuple[set[str], int]:
    consolidation_count = 0
    sources: set[str] = set()
    if not HOME.is_dir():
        print(f"archive root not found: {HOME}", file=sys.stderr)
        return sources, 0
    for p in sorted(HOME.rglob("*.parquet")):
        for env in iter_envelopes(p):
            if env.get("kind") != "consolidation":
                continue
            consolidation_count += 1
            payload = env.get("payload", {})
            ids = payload.get("source_event_ids", [])
            if isinstance(ids, list):
                for eid in ids:
                    if isinstance(eid, str) and eid:
                        sources.add(eid)
    return sources, consolidation_count


def forget(event_id: str, dry_run: bool) -> tuple[bool, str]:
    if dry_run:
        return True, "dry-run"
    body = json.dumps(
        {
            "event_id": event_id,
            "confirmation_token": TOKEN,
            "dry_run": False,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        f"{API_URL}/v1/admin/forget",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return True, f"HTTP {resp.status}"
    except urllib.error.HTTPError as e:
        return False, f"HTTP {e.code}: {e.read().decode('utf-8')[:160]}"
    except Exception as exc:
        return False, f"err: {exc}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Cap at the first N source ids (0 = all). For sampling.",
    )
    args = parser.parse_args()

    sources, cons_count = collect_source_event_ids()
    print(f"consolidations scanned: {cons_count}")
    print(f"distinct source event_ids to purge: {len(sources)}")
    if args.limit > 0 and len(sources) > args.limit:
        sources = set(list(sources)[: args.limit])
        print(f"limited to first {args.limit}")

    ok = 0
    failed = 0
    sorted_sources = sorted(sources)
    for i, eid in enumerate(sorted_sources, 1):
        success, msg = forget(eid, args.dry_run)
        if success:
            ok += 1
        else:
            failed += 1
        if i % 100 == 0 or not success:
            print(f"  [{i}/{len(sorted_sources)}] {eid}: {'OK' if success else msg}")

    print()
    print(f"purged: {ok}")
    print(f"failed: {failed}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

"""
2026-05-19 — one-shot archive purge for `kind=consolidation` envelopes.

Walks `${CORTEX_HOME}/events/**/*.parquet`, decompresses each file
(zstd-compressed NDJSON despite the .parquet extension — see
`crates/cortex-workers/src/retention/parquet_rollup.rs` for the
on-disk format note), drops every line whose envelope carries
`kind == "consolidation"`, and rewrites the file in place.

Why a Python script instead of a Rust binary: the archive is the
historical record; rewriting parquets is a one-shot operator
action triggered by the 2026-05-19 incident where the LLM
consolidator emitted ~700 "Session incomplete" envelopes that
poisoned the dashboard. The session producer filter
(producer/session.rs) prevents recurrence; this script removes
the existing damage.

Run from the repo root:
    python scripts/purge_consolidation_envelopes.py [--dry-run]

Exit 0 on success, 1 on any unrecoverable parse / write error.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import sys
from pathlib import Path

import zstandard as zstd


def cortex_home() -> Path:
    env = os.environ.get("CORTEX_HOME")
    if env:
        return Path(env)
    home = Path.home() / ".cortex"
    return home


def rewrite_file(path: Path, dry_run: bool) -> tuple[int, int]:
    """Return (kept, dropped)."""
    raw = path.read_bytes()
    decompressor = zstd.ZstdDecompressor()
    try:
        decoded = decompressor.decompress(raw)
    except zstd.ZstdError as exc:
        # `decompress` requires a known frame size; fall back to
        # streaming decompression for files written without the
        # size header (cortex-ingestion does this on rotation).
        with decompressor.stream_reader(io.BytesIO(raw)) as reader:
            decoded = reader.read()
        _ = exc  # noqa: F841

    kept_lines: list[bytes] = []
    dropped = 0
    for line in decoded.split(b"\n"):
        if not line.strip():
            continue
        try:
            env = json.loads(line)
        except json.JSONDecodeError:
            # Keep unparseable lines verbatim so the script never
            # silently destroys data the loader could still read.
            kept_lines.append(line)
            continue
        if env.get("kind") == "consolidation":
            dropped += 1
            continue
        kept_lines.append(line)

    if dropped == 0:
        return (len(kept_lines), 0)

    if dry_run:
        return (len(kept_lines), dropped)

    # Re-encode + atomic replace. zstd level 3 matches the writer
    # default in `cortex-storage::archive::ArchiveLayout`.
    body = b"\n".join(kept_lines)
    if body and not body.endswith(b"\n"):
        body += b"\n"
    compressor = zstd.ZstdCompressor(level=3)
    encoded = compressor.compress(body)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(encoded)
    os.replace(tmp, path)
    return (len(kept_lines), dropped)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--home",
        type=Path,
        default=None,
        help="Override archive home (defaults to $CORTEX_HOME or ~/.cortex)",
    )
    args = parser.parse_args()

    home = args.home or cortex_home()
    # The on-disk layout is `<home>/archive/events/year=…` per
    # `ArchiveLayout::ROOT_SEGMENT`; legacy installs used
    # `<home>/events/` directly. Try the modern layout first.
    candidates = [home / "archive" / "events", home / "events"]
    events_root = next((c for c in candidates if c.is_dir()), candidates[0])
    if not events_root.is_dir():
        print(f"events root not found: {events_root}", file=sys.stderr)
        return 1

    total_files = 0
    total_dropped = 0
    total_kept = 0
    rewritten = 0
    failed: list[str] = []
    for path in sorted(events_root.rglob("*.parquet")):
        total_files += 1
        try:
            kept, dropped = rewrite_file(path, args.dry_run)
        except Exception as exc:  # noqa: BLE001
            failed.append(f"{path}: {exc}")
            continue
        if dropped:
            rewritten += 1
            print(
                f"{'WOULD ' if args.dry_run else ''}REWRITE {path} "
                f"kept={kept} dropped={dropped}"
            )
        total_dropped += dropped
        total_kept += kept

    print()
    print(f"files scanned   : {total_files}")
    print(f"files rewritten : {rewritten}")
    print(f"envelopes kept  : {total_kept}")
    print(f"envelopes dropped (kind=consolidation): {total_dropped}")
    if failed:
        print(f"failed files: {len(failed)}", file=sys.stderr)
        for line in failed:
            print(f"  {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

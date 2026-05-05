#!/usr/bin/env python3
"""
cypher_load.py — stream a Cypher script into Nexus over HTTP /cypher.

Reads a .cypher file (one statement per line, semicolon-terminated, comments
prefixed with `//`), POSTs each statement to Nexus, and prints progress.

Workers run in parallel via a thread pool. Idempotent statements (MERGE) are
safe to retry. Designed for the output of `graph_correlate.py`.

stdlib only.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from threading import Lock


def iter_statements(path: Path):
    """Yield (line_no, statement) — strip comments and trailing semicolons."""
    with path.open("r", encoding="utf-8") as f:
        for ln, raw in enumerate(f, start=1):
            line = raw.strip()
            if not line or line.startswith("//"):
                continue
            if line.endswith(";"):
                line = line[:-1]
            if line:
                yield ln, line


def post_cypher(url: str, query: str, timeout: float, retries: int = 4) -> tuple[int, str]:
    """POST with exponential backoff. Retries on connection errors / 5xx."""
    body = json.dumps({"query": query}).encode("utf-8")
    last_code, last_body = -1, ""
    for attempt in range(retries + 1):
        req = urllib.request.Request(
            url,
            data=body,
            headers={"content-type": "application/json", "accept": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.getcode(), resp.read().decode("utf-8", errors="replace")[:300]
        except urllib.error.HTTPError as e:
            last_code = e.code
            last_body = e.read().decode("utf-8", errors="replace")[:300]
            if e.code < 500 or attempt >= retries:
                return last_code, last_body
        except (urllib.error.URLError, TimeoutError, ConnectionError, OSError) as e:
            last_code = -1
            last_body = str(e)[:300]
            if attempt >= retries:
                return last_code, last_body
        # Backoff: 0.2, 0.6, 1.4, 3.0s
        time.sleep(0.2 * (2 ** attempt) + 0.05 * attempt)
    return last_code, last_body


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="POST a Cypher dump into Nexus, one statement per request.")
    p.add_argument("input", type=Path, help="Path to .cypher file")
    p.add_argument("--url", default="http://127.0.0.1:17002/cypher", help="Nexus /cypher endpoint")
    p.add_argument("--workers", type=int, default=16, help="Parallel HTTP workers")
    p.add_argument("--timeout", type=float, default=30.0, help="Per-request timeout (s)")
    p.add_argument("--limit", type=int, default=-1, help="Stop after N statements (debug)")
    p.add_argument("--skip", type=int, default=0, help="Skip the first N statements (resume)")
    p.add_argument("--fail-fast", action="store_true", help="Abort on first error")
    p.add_argument("--errors-out", type=Path, default=Path("./scripts/graph/cypher_load.errors.txt"))
    p.add_argument("--probe-only", action="store_true", help="Send only the first statement, then exit")
    args = p.parse_args(argv)

    if not args.input.exists():
        print(f"[cypher_load] missing {args.input}", file=sys.stderr)
        return 2

    total = 0
    statements: list[tuple[int, str]] = []
    for i, (ln, stmt) in enumerate(iter_statements(args.input)):
        if i < args.skip:
            continue
        statements.append((ln, stmt))
        total += 1
        if args.limit > 0 and total >= args.limit:
            break

    if args.probe_only:
        statements = statements[:1]

    print(f"[cypher_load] {len(statements)} statements ready  workers={args.workers}  url={args.url}")

    if not statements:
        return 0

    errors_lock = Lock()
    args.errors_out.parent.mkdir(parents=True, exist_ok=True)
    err_f = args.errors_out.open("w", encoding="utf-8")

    counters = {"ok": 0, "err": 0}
    counters_lock = Lock()
    t0 = time.time()
    last_print = t0

    def submit(line_no: int, stmt: str) -> tuple[int, int, str]:
        code, body = post_cypher(args.url, stmt, args.timeout)
        return line_no, code, body

    aborted = False
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futs = {pool.submit(submit, ln, s): (ln, s) for ln, s in statements}
        for fut in as_completed(futs):
            ln, stmt = futs[fut]
            try:
                line_no, code, body = fut.result()
            except Exception as e:  # noqa: BLE001
                line_no, code, body = ln, -2, repr(e)[:300]

            ok = (code == 200)
            with counters_lock:
                if ok:
                    counters["ok"] += 1
                else:
                    counters["err"] += 1
            if not ok:
                with errors_lock:
                    err_f.write(f"line={line_no} code={code} body={body}\n")
                    err_f.write(f"  stmt={stmt[:300]}\n")
                if args.fail_fast and not aborted:
                    aborted = True
                    print(f"[cypher_load] FAIL-FAST line={line_no} code={code} body={body}", file=sys.stderr)
                    for f in futs:
                        f.cancel()
                    break

            now = time.time()
            done = counters["ok"] + counters["err"]
            if now - last_print > 2.0 or done == len(statements):
                rate = done / max(1e-3, now - t0)
                eta_s = (len(statements) - done) / max(1e-3, rate)
                print(
                    f"[cypher_load] {done}/{len(statements)}  "
                    f"ok={counters['ok']}  err={counters['err']}  "
                    f"rate={rate:.0f}/s  eta={eta_s:.0f}s",
                    flush=True,
                )
                last_print = now

    err_f.close()
    elapsed = time.time() - t0
    print(
        f"[cypher_load] done in {elapsed:.1f}s  ok={counters['ok']}  err={counters['err']}  "
        f"errors -> {args.errors_out}"
    )
    if args.probe_only:
        # Print probe response inline
        print("\n--- probe response ---")
        ln, stmt = statements[0]
        code, body = post_cypher(args.url, stmt, args.timeout)
        print(f"line {ln}  HTTP {code}\n{body}")

    return 0 if counters["err"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

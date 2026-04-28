#!/usr/bin/env bash
# scripts/backfill-graph-symbols.sh
#
# Phase4e operations runbook: replay the archived event stream through
# the live `cortex-graph-worker` so the existing Nexus instance gains
# the Symbol nodes and DEFINES edges that phase4c shipped.
#
# Steps (all idempotent — Nexus MERGEs every node and every edge):
#   1. Bootstrap the graph schema (constraints + indexes).
#   2. Replay $CORTEX_ARCHIVE_ROOT through `cortex-graph-backfill`.
#   3. Probe Nexus for non-zero Symbol + Artifact counts.
#   4. Probe Nexus for the canonical PreThinkingTool -> tools.rs DEFINES.
#
# `--dry-run` prints the four steps (with the exact Cypher that would
# run) without touching Nexus or the filesystem. CI uses this mode to
# guard the script's surface against drift.
#
# Exit codes:
#   0 — every step succeeded; both probes returned the expected shape.
#   1 — usage error (missing env, missing binary).
#   2 — bootstrap or replay failed.
#   3 — a probe returned an unexpected shape (sym=0, art=0, or the
#       PreThinkingTool lookup did not include the expected file).

set -euo pipefail

DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|--dry)
      DRY_RUN=1
      ;;
    -h|--help)
      sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      echo "usage: $0 [--dry-run]" >&2
      exit 1
      ;;
  esac
done

BIN="${CORTEX_GRAPH_BACKFILL_BIN:-cortex-graph-backfill}"
PROBE1='MATCH (s:Symbol)-[:DEFINES]->(a:Artifact) RETURN count(s) AS sym, count(DISTINCT a) AS art'
PROBE2='MATCH (s:Symbol {name: "PreThinkingTool"})-[:DEFINES]->(a:Artifact) RETURN a.repo AS repo, a.path AS path'
EXPECTED_REPO='Cortex'
EXPECTED_PATH='crates/cortex-mcp-server/src/tools.rs'

if [ "$DRY_RUN" -eq 1 ]; then
  # Hermetic — does not touch Nexus, the archive, or env vars.
  cat <<EOF
DRY RUN — phase4e graph-symbol backfill runbook
binary: ${BIN}
nexus:  \${CORTEX_NEXUS_URL}
archive root: \${CORTEX_ARCHIVE_ROOT}

step 1/4: bootstrap graph schema
  ${BIN} --ensure-schema-only

step 2/4: replay archive
  ${BIN} --archive-root "\${CORTEX_ARCHIVE_ROOT}"

step 3/4: probe Symbol + Artifact counts
  ${BIN} --probe '${PROBE1}'
  expected: sym>0 AND art>0

step 4/4: probe PreThinkingTool DEFINES Artifact
  ${BIN} --probe '${PROBE2}'
  expected: rows include repo=${EXPECTED_REPO} path=${EXPECTED_PATH}
EOF
  exit 0
fi

# Live mode — env vars are now mandatory.
: "${CORTEX_NEXUS_URL:?CORTEX_NEXUS_URL must be set in live mode}"
: "${CORTEX_ARCHIVE_ROOT:?CORTEX_ARCHIVE_ROOT must be set in live mode}"

if ! command -v "$BIN" >/dev/null 2>&1; then
  echo "FAIL: $BIN not found on PATH (set CORTEX_GRAPH_BACKFILL_BIN to override)" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "FAIL: python3 is required for probe assertions" >&2
  exit 1
fi

echo "[1/4] bootstrapping graph schema against ${CORTEX_NEXUS_URL}..."
if ! "$BIN" --ensure-schema-only; then
  echo "FAIL: schema bootstrap" >&2
  exit 2
fi

echo "[2/4] replaying archive at ${CORTEX_ARCHIVE_ROOT}..."
if ! "$BIN" --archive-root "${CORTEX_ARCHIVE_ROOT}"; then
  echo "FAIL: backfill replay" >&2
  exit 2
fi

echo "[3/4] probing Symbol + Artifact counts..."
PROBE1_OUT="$("$BIN" --probe "$PROBE1")" || {
  echo "FAIL: probe1 failed to execute" >&2
  exit 2
}
echo "$PROBE1_OUT"
python3 - "$PROBE1_OUT" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
rows = data.get("rows") or []
if not rows:
    print("FAIL: probe1 returned no rows", file=sys.stderr)
    sys.exit(3)
sym, art = rows[0][0], rows[0][1]
print(f"sym={sym} art={art}")
if not (isinstance(sym, int) and isinstance(art, int) and sym > 0 and art > 0):
    print(f"FAIL: expected sym>0 and art>0, got sym={sym} art={art}", file=sys.stderr)
    sys.exit(3)
PY

echo "[4/4] probing PreThinkingTool DEFINES Artifact..."
PROBE2_OUT="$("$BIN" --probe "$PROBE2")" || {
  echo "FAIL: probe2 failed to execute" >&2
  exit 2
}
echo "$PROBE2_OUT"
python3 - "$PROBE2_OUT" "$EXPECTED_REPO" "$EXPECTED_PATH" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
expected_repo, expected_path = sys.argv[2], sys.argv[3]
rows = data.get("rows") or []
if not rows:
    print("FAIL: no DEFINES edge for PreThinkingTool", file=sys.stderr)
    sys.exit(3)
for row in rows:
    print(f"  {row[0]} :: {row[1]}")
ok = any(r[0] == expected_repo and r[1] == expected_path for r in rows)
if not ok:
    print(f"FAIL: expected {expected_repo} / {expected_path} in matches", file=sys.stderr)
    sys.exit(3)
PY

echo
echo "phase4e backfill complete."

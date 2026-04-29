#!/usr/bin/env bash
# Phase8c — pretty-print the cortex-api `/v1/health/versions` drift
# report and exit non-zero when any running binary is behind the
# workspace HEAD. Designed for operator use ("did everyone restart
# after the last `cargo build`?") + CI smoke jobs.
#
# Usage:
#   scripts/doctor-versions.sh                 # default endpoint
#   scripts/doctor-versions.sh -u http://...   # custom endpoint
#
# Exit codes:
#   0 — all_in_sync = true
#   1 — at least one binary's git_sha != workspace HEAD
#   2 — could not reach /v1/health/versions at all
set -u

ENDPOINT="${CORTEX_API_URL:-http://127.0.0.1:17000}/v1/health/versions"
while [ $# -gt 0 ]; do
  case "$1" in
    -u|--url)
      ENDPOINT="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,15p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if ! command -v curl >/dev/null 2>&1; then
  echo "curl not found on PATH; install curl to use this script" >&2
  exit 2
fi

RAW=$(curl -fsS --max-time 5 "$ENDPOINT" 2>/dev/null)
RC=$?
if [ $RC -ne 0 ] || [ -z "$RAW" ]; then
  echo "could not reach cortex-api $ENDPOINT" >&2
  exit 2
fi

# Stable JSON shape: { head_sha, head_sha_short, running_binaries: [...],
#                      drift: [...], all_in_sync: bool }.
HEAD_SHA=$(printf '%s' "$RAW" \
  | tr ',' '\n' \
  | grep -E '"head_sha_short"' \
  | head -1 \
  | sed -E 's/.*"head_sha_short"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')

ALL_IN_SYNC=$(printf '%s' "$RAW" \
  | tr ',' '\n' \
  | grep -E '"all_in_sync"' \
  | head -1 \
  | sed -E 's/.*"all_in_sync"[[:space:]]*:[[:space:]]*([a-z]+).*/\1/')

echo "endpoint:    $ENDPOINT"
echo "head_sha:    $HEAD_SHA"
echo "all_in_sync: $ALL_IN_SYNC"
echo
echo "running_binaries (name → git_sha_short, dirty?):"
printf '%s' "$RAW" \
  | tr '{}' '\n' \
  | grep -E '"name"' \
  | awk -F'"' '
      {
        name=""; sha=""; dirty="";
        for (i = 1; i <= NF; i++) {
          if ($i == "name") { name = $(i+2) }
          if ($i == "git_sha_short") { sha = $(i+2) }
        }
        # `git_dirty` is unquoted; pull from the surrounding line.
        if (match($0, /"git_dirty"[[:space:]]*:[[:space:]]*(true|false)/, m)) {
          dirty = m[1]
        }
        if (name != "") {
          printf "  %-30s %-10s dirty=%s\n", name, sha, dirty
        }
      }'

case "$ALL_IN_SYNC" in
  true)  exit 0 ;;
  false) exit 1 ;;
  *)
    echo "could not parse all_in_sync from response" >&2
    exit 2
    ;;
esac

#!/usr/bin/env bash
# Phase8h — CI smoke boot helper. Spawns cortex-ingestion,
# cortex-api, and cortex-adapter-claude-code in the background and
# waits for cortex-api's /v1/health to report `overall: ok` before
# returning. Pid file at $CORTEX_PIDS_FILE so the matching
# `teardown-stack.sh` can kill them.
#
# External dependencies (Vectorizer, Nexus, Synap, Meili) are NOT
# booted — the cortex-api in-memory lane fallbacks (`MemoryVectorLane`,
# `MemoryKeywordLane`, `MemoryGraphLane`) let the smoke run exercise
# the adapter → ingestion → archive path end-to-end without those
# services. The doctor checks downstream report the missing services
# as `degraded` not `down`, which is the expected smoke shape.
#
# Required env:
#   CORTEX_HOME — isolated working dir (so concurrent CI jobs don't
#                 collide on ~/.cortex). Created if missing.
#
# Optional env:
#   CORTEX_PIDS_FILE — pid file path (default $CORTEX_HOME/pids).
#   CORTEX_BOOT_TIMEOUT_SECS — wait budget (default 60).
#
# Exit codes:
#   0 — stack booted; /v1/health overall=ok
#   1 — boot failed (timeout / spawn error)
set -uo pipefail

if [ -z "${CORTEX_HOME:-}" ]; then
  echo "CORTEX_HOME must be set (concurrent CI runs require isolated dirs)" >&2
  exit 1
fi
mkdir -p "$CORTEX_HOME/archive" "$CORTEX_HOME/logs"
PIDS_FILE="${CORTEX_PIDS_FILE:-$CORTEX_HOME/pids}"
TIMEOUT_SECS="${CORTEX_BOOT_TIMEOUT_SECS:-60}"

: > "$PIDS_FILE"

echo "boot-stack: CORTEX_HOME=$CORTEX_HOME pids=$PIDS_FILE timeout=${TIMEOUT_SECS}s"

# 1. cortex-ingestion (port 17010 by default).
export CORTEX_ARCHIVE_ROOT="$CORTEX_HOME/archive"
nohup cargo run --quiet --release -p cortex-ingestion \
  > "$CORTEX_HOME/logs/cortex-ingestion.log" 2>&1 &
echo $! >> "$PIDS_FILE"

# 2. cortex-api (port 17000 by default).
nohup cargo run --quiet --release -p cortex-api \
  > "$CORTEX_HOME/logs/cortex-api.log" 2>&1 &
echo $! >> "$PIDS_FILE"

# 3. cortex-adapter-claude-code daemon (admin port 17011).
nohup cargo run --quiet --release -p cortex-adapter-claude-code \
  > "$CORTEX_HOME/logs/cortex-adapter.log" 2>&1 &
echo $! >> "$PIDS_FILE"

# Poll /v1/health until overall != "down" or timeout. The smoke run
# accepts `degraded` as a valid boot state because the lane fallbacks
# render the missing-services case as soft-degraded rather than down.
echo "boot-stack: waiting for /v1/health to come up…"
deadline=$(( $(date +%s) + TIMEOUT_SECS ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  if status=$(curl -fsS --max-time 2 http://127.0.0.1:17000/v1/health 2>/dev/null); then
    overall=$(printf '%s' "$status" \
      | tr ',' '\n' \
      | grep -E '"overall"' \
      | head -1 \
      | sed -E 's/.*"overall"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
    case "$overall" in
      ok|degraded)
        echo "boot-stack: ready (overall=$overall)"
        exit 0
        ;;
      down)
        echo "boot-stack: stack reported overall=down; failing fast"
        exit 1
        ;;
    esac
  fi
  sleep 1
done

echo "boot-stack: timeout after ${TIMEOUT_SECS}s waiting for /v1/health"
exit 1

#!/usr/bin/env bash
# Phase8h — companion to boot-stack.sh. Reads $CORTEX_PIDS_FILE and
# sends SIGTERM (then SIGKILL after 5s) to every pid. Idempotent —
# missing pid file or already-dead processes return 0.
set -u

PIDS_FILE="${CORTEX_PIDS_FILE:-${CORTEX_HOME:-/tmp}/pids}"
if [ ! -f "$PIDS_FILE" ]; then
  echo "teardown-stack: $PIDS_FILE missing; nothing to kill"
  exit 0
fi

while IFS= read -r pid; do
  if [ -z "$pid" ]; then continue; fi
  if kill -0 "$pid" 2>/dev/null; then
    echo "teardown-stack: SIGTERM $pid"
    kill -TERM "$pid" 2>/dev/null || true
  fi
done < "$PIDS_FILE"

# Grace window then SIGKILL anything still up.
sleep 5
while IFS= read -r pid; do
  if [ -z "$pid" ]; then continue; fi
  if kill -0 "$pid" 2>/dev/null; then
    echo "teardown-stack: SIGKILL $pid"
    kill -KILL "$pid" 2>/dev/null || true
  fi
done < "$PIDS_FILE"

rm -f "$PIDS_FILE"
echo "teardown-stack: done"

#!/usr/bin/env bash
# Phase8a — pretty-print the cortex-api `/v1/health` aggregator and
# exit on the worst observed state. Intended as a quick "is the
# stack healthy?" probe for operators + CI smoke jobs.
#
# Usage:
#   scripts/doctor/health.sh                 # default endpoint
#   scripts/doctor/health.sh -u http://...   # custom endpoint
#
# Exit codes match `cortex_health::HealthState::exit_code`:
#   0 — overall=ok
#   1 — overall=degraded
#   2 — overall=down
#   3 — could not reach `/v1/health` at all
set -u

ENDPOINT="${CORTEX_API_URL:-http://127.0.0.1:17000}/v1/health"
while [ $# -gt 0 ]; do
  case "$1" in
    -u|--url)
      ENDPOINT="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,16p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 3
      ;;
  esac
done

if ! command -v curl >/dev/null 2>&1; then
  echo "curl not found on PATH; install curl to use scripts/doctor/health.sh" >&2
  exit 3
fi

# Pull the report. -fsS lets curl print the body on success and exit
# non-zero on transport / 4xx-5xx — both are aggregator-down signals.
RAW=$(curl -fsS --max-time 5 "$ENDPOINT" 2>/dev/null)
RC=$?
if [ $RC -ne 0 ] || [ -z "$RAW" ]; then
  echo "could not reach cortex-api $ENDPOINT" >&2
  exit 3
fi

# Quick pretty-print + state extraction. Avoid jq dependency by
# using a tiny portable awk-based parser; the field shape is stable
# (cortex_health::HealthReport).
OVERALL=$(printf '%s' "$RAW" \
  | tr ',' '\n' \
  | grep -E '"overall"' \
  | head -1 \
  | sed -E 's/.*"overall"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')

if [ -z "$OVERALL" ]; then
  echo "could not parse overall state from response: $RAW" >&2
  exit 3
fi

echo "endpoint:  $ENDPOINT"
echo "overall:   $OVERALL"
echo
echo "subsystems:"
printf '%s' "$RAW" \
  | tr '{}' '\n' \
  | grep -E '"name"' \
  | awk -F'"' '
      {
        name=""; state=""; latency=""; err="";
        for (i = 1; i <= NF; i++) {
          if ($i == "name") { name = $(i+2) }
          if ($i == "state") { state = $(i+2) }
          if ($i == "last_error") { err = $(i+2) }
        }
        if (name != "") {
          printf "  %-30s %-9s %s\n", name, state, err
        }
      }'

case "$OVERALL" in
  ok)       exit 0 ;;
  degraded) exit 1 ;;
  down)     exit 2 ;;
  *)
    echo "unknown overall state: $OVERALL" >&2
    exit 3
    ;;
esac

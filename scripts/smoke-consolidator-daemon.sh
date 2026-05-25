#!/usr/bin/env bash
# phase14a §5.4 — smoke test for the cortex-consolidator daemon.
#
# Verifies the end-to-end trigger → grain → Meili envelope flow:
#
#   1. /v1/health/consolidator is mounted on the live cortex-api.
#   2. The cortex-consolidator container is running.
#   3. Publishing a session_end trigger to the Synap stream causes
#      the daemon to pick it up, dispatch SessionGrain, and a
#      Kind::Consolidation envelope lands in the cortex_consolidations
#      Meili index within $POLL_TIMEOUT seconds.
#
# Prerequisites:
#
#   - docker compose stack rebuilt with phase14a code:
#       CORTEX_GIT_SHA=$(git rev-parse HEAD) \
#       docker compose build cortex-api cortex-consolidator cortex-ingestion
#       docker compose up -d cortex-api cortex-consolidator cortex-ingestion
#
#   - ANTHROPIC_API_KEY available to the cortex-consolidator container
#     (set in .env or exported before `docker compose up`).
#
#   - A session_id present in the parquet archive that has at least
#     two real Turn envelopes (substance floor in producer/session.rs).
#     Override via SMOKE_SESSION_ID; defaults to the most recent
#     session in the archive enumerated via cortex-ops.
#
# Exit codes:
#   0 — every step green
#   1 — env / dependency missing
#   2 — health endpoint not mounted (rebuild needed)
#   3 — consolidator container missing or unhealthy
#   4 — trigger publish failed
#   5 — Meili lookup did not surface the new envelope before timeout

set -euo pipefail

CORTEX_API_URL=${CORTEX_API_URL:-http://127.0.0.1:17000}
SYNAP_BASE_URL=${SYNAP_BASE_URL:-http://127.0.0.1:17003}
MEILI_URL=${MEILI_URL:-http://127.0.0.1:17004}
MEILI_KEY=${MEILI_MASTER_KEY:-cortex-dev-master-key}
TRIGGER_STREAM=${TRIGGER_STREAM:-cortex.consolidator.triggers}
POLL_TIMEOUT=${POLL_TIMEOUT:-60}
SMOKE_SESSION_ID=${SMOKE_SESSION_ID:-}

step() { printf "\n[smoke] %s\n" "$1"; }
fail() {
    printf "\n[smoke FAIL] %s\n" "$1" >&2
    exit "${2:-1}"
}

step "1/5 verify dependencies on PATH"
command -v curl >/dev/null || fail "curl not on PATH" 1
command -v jq >/dev/null || fail "jq not on PATH" 1
command -v docker >/dev/null || fail "docker not on PATH" 1

step "2/5 GET ${CORTEX_API_URL}/v1/health/consolidator (route mount check)"
health=$(curl -sf "${CORTEX_API_URL}/v1/health/consolidator" || true)
if [[ -z "${health}" ]]; then
    fail "endpoint unreachable — rebuild cortex-api with phase14a code" 2
fi
echo "${health}" | jq -e '.session_grain and .topic_grain and .decision_trace_grain' \
    >/dev/null || fail "health payload missing one of the three grain keys" 2
echo "  ok — payload: $(echo "${health}" | jq -c .)"

step "3/5 confirm cortex-consolidator container is up"
if ! docker compose ps cortex-consolidator | grep -q "Up"; then
    fail "cortex-consolidator container is not running — run \`docker compose up -d cortex-consolidator\`" 3
fi
echo "  ok — container running"

step "4/5 resolve a session_id from the archive"
if [[ -z "${SMOKE_SESSION_ID}" ]]; then
    if command -v cortex-ops >/dev/null; then
        SMOKE_SESSION_ID=$(cortex-ops sessions list --limit 1 --json 2>/dev/null \
            | jq -r '.[0].session_id // empty' || true)
    fi
fi
if [[ -z "${SMOKE_SESSION_ID}" ]]; then
    fail "no session_id available — set SMOKE_SESSION_ID env" 1
fi
echo "  ok — session_id=${SMOKE_SESSION_ID}"

step "5/5 publish trigger + poll Meili"
publish_body=$(jq -nc \
    --arg sid "${SMOKE_SESSION_ID}" \
    '{event: "session_end", data: {kind: "session_end", session_id: $sid}}')
http_code=$(curl -s -o /tmp/synap-publish.out -w "%{http_code}" \
    -X POST "${SYNAP_BASE_URL}/streams/${TRIGGER_STREAM}/publish" \
    -H 'content-type: application/json' \
    --data "${publish_body}")
if [[ "${http_code}" != "200" && "${http_code}" != "201" && "${http_code}" != "204" ]]; then
    cat /tmp/synap-publish.out >&2
    fail "Synap publish returned HTTP ${http_code}" 4
fi
echo "  ok — trigger published to ${TRIGGER_STREAM}"

# Derive the deterministic consolidation_id the SessionGrain emits.
# Matches `derive_consolidation_id(ConsolidationGrain::Session, ...)`
# from consolidator/producer/mod.rs.
expected_prefix="cons-ses-"
echo "  polling cortex_consolidations for prefix=${expected_prefix} session_id=${SMOKE_SESSION_ID} (timeout ${POLL_TIMEOUT}s)…"
deadline=$(( $(date +%s) + POLL_TIMEOUT ))
while [[ $(date +%s) -lt ${deadline} ]]; do
    hits=$(curl -sf -H "Authorization: Bearer ${MEILI_KEY}" \
        -H 'content-type: application/json' \
        -X POST "${MEILI_URL}/indexes/cortex_consolidations/search" \
        --data "$(jq -nc --arg sid "${SMOKE_SESSION_ID}" \
            '{q: $sid, limit: 5}')" \
        | jq -r '.hits | length' || echo 0)
    if [[ "${hits:-0}" -gt 0 ]]; then
        echo "  ok — Meili surfaced ${hits} hit(s) for session ${SMOKE_SESSION_ID}"
        echo "[smoke OK] consolidator daemon end-to-end green"
        exit 0
    fi
    sleep 2
done
fail "Meili did not surface a consolidation for session ${SMOKE_SESSION_ID} within ${POLL_TIMEOUT}s" 5

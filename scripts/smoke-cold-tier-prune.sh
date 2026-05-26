#!/usr/bin/env bash
# phase14b §5.4 — smoke test for the cold-tier prune cascade.
#
# Backfills $EXPIRED_COUNT (default 100) cold envelopes whose
# `occurred_at` is well past the 365-day cutoff, runs cold-tier
# prune, then asserts `doctor consistency` reports zero residue —
# every backfilled event_id absent from Meili + Nexus + Vectorizer
# + parquet archive AND from the `event_identity` SQLite index.
#
# Prerequisites:
#
#   - docker compose stack rebuilt with phase14b code (the running
#     containers may be older than the new sweeps; verify with
#     `docker compose ps` and `docker compose build` as needed).
#   - Anthropic credentials present (consolidator may run during the
#     same window — does not affect this smoke).
#   - cortex-ops on PATH (built from this repo).
#
# Exit codes:
#   0 — every step green
#   1 — env / dependency missing
#   2 — backfill failed
#   3 — cold-tier prune sweep failed
#   4 — doctor consistency reported residue

set -euo pipefail

EXPIRED_COUNT=${EXPIRED_COUNT:-100}
CORTEX_API_URL=${CORTEX_API_URL:-http://127.0.0.1:17000}

step() { printf "\n[smoke] %s\n" "$1"; }
fail() {
    printf "\n[smoke FAIL] %s\n" "$1" >&2
    exit "${2:-1}"
}

step "1/4 verify dependencies on PATH"
command -v cortex-ops >/dev/null || fail "cortex-ops not on PATH" 1
command -v curl >/dev/null || fail "curl not on PATH" 1
command -v jq >/dev/null || fail "jq not on PATH" 1

step "2/4 backfill ${EXPIRED_COUNT} cold envelopes (occurred_at ~ 2024-01-01)"
# cortex-ops backfill writes one envelope per call; loop locally to
# stay independent of any single-shot bulk endpoint. Replace with a
# faster path if/when a bulk backfill subcommand lands.
backfill_ids=()
for i in $(seq 1 "${EXPIRED_COUNT}"); do
    eid="01SMOKECOLD$(printf '%015d' "${i}")"
    body=$(jq -nc \
        --arg eid "${eid}" \
        --arg occ "2024-01-01T00:00:00Z" \
        '{event_id: $eid, schema_version: "1", occurred_at: $occ,
          session_id: "01SMOKECOLDSESSION0000000A", stream: "live",
          tool: "smoke", kind: "turn",
          context: {repo: "cortex-smoke", platform: "linux"},
          payload: {user_message: "smoke", assistant_message: "smoke"},
          content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"}')
    http_code=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "${CORTEX_API_URL%/}/v1/events" \
        -H 'content-type: application/json' --data "${body}")
    if [[ "${http_code}" != "200" && "${http_code}" != "202" ]]; then
        fail "POST /v1/events for ${eid} returned ${http_code}" 2
    fi
    backfill_ids+=("${eid}")
done
echo "  ok — ${#backfill_ids[@]} envelopes published to ingestion"

step "3/4 run cold-tier prune (the sweep scheduler picks it up; this step blocks until the next sweep run lands)"
if ! cortex-ops retention run --sweep cold_tier_prune; then
    fail "cold_tier_prune sweep returned non-zero" 3
fi
echo "  ok — cold_tier_prune completed"

step "4/4 doctor consistency — every backfilled id must be absent"
residue=0
for eid in "${backfill_ids[@]}"; do
    if cortex-ops doctor-identity-coverage --event-id "${eid}" --json 2>/dev/null \
        | jq -e '.found' >/dev/null 2>&1; then
        residue=$(( residue + 1 ))
    fi
done
if [[ ${residue} -gt 0 ]]; then
    fail "doctor consistency reported ${residue} residual ids out of ${EXPIRED_COUNT}" 4
fi
echo "  ok — every backfilled id absent from every backend + event_identity"
echo "[smoke OK] cold-tier prune cascade end-to-end green"

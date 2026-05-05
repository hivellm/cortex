#!/usr/bin/env bash
# Phase11s §5.2 — per-repo coverage probe.
#
# Reads doc / vector / node counts from Meili, Vectorizer, and
# Nexus for each repo registered in the metadata DB and prints a
# single table. Flags any repo whose Nexus or Vectorizer count is
# < 50 % of the Meili count (the §5 runbook's re-bootstrap
# threshold).
#
# Reads:
#   MEILI_URL       (default http://127.0.0.1:17003)
#   VECTORIZER_URL  (default http://127.0.0.1:17001)
#   NEXUS_URL       (default http://127.0.0.1:17002)
#   MEILI_KEY       (optional Meili master / search key)
#
# Exits non-zero when any repo trips the < 50 % threshold so the
# §4.2 drain-recovery IT can use this script as its assertion.

set -euo pipefail

MEILI_URL="${MEILI_URL:-http://127.0.0.1:17003}"
VECTORIZER_URL="${VECTORIZER_URL:-http://127.0.0.1:17001}"
NEXUS_URL="${NEXUS_URL:-http://127.0.0.1:17002}"
MEILI_KEY="${MEILI_KEY:-}"

REPOS_RAW="${CORTEX_REPOS:-cortex rulebook nexus vectorizer synap lexum expert hivehubcloud tml transmutation umicp compressionprompt hivegpu vectorizersync tmltextmate tmldocs transmutationlite}"
read -r -a REPOS <<<"$REPOS_RAW"

threshold_pct=50
any_failure=0

meili_args=(--max-time 5 -fsS)
if [[ -n "$MEILI_KEY" ]]; then
  meili_args+=(-H "Authorization: Bearer $MEILI_KEY")
fi

printf '%-32s  %12s  %12s  %12s  %s\n' \
  "repo" "meili" "vectorizer" "nexus" "status"
printf '%-32s  %12s  %12s  %12s  %s\n' \
  "----" "-----" "----------" "-----" "------"

for repo in "${REPOS[@]}"; do
  # Meili — sum over the per-repo families.
  meili_total=0
  for family in code docs decisions turns governance analyses knowledge learnings consolidations topic_cards misc; do
    index="cortex-${repo}-${family}"
    body="$(curl "${meili_args[@]}" "$MEILI_URL/indexes/$index/stats" 2>/dev/null || echo '{}')"
    n="$(printf '%s' "$body" | grep -oE '"numberOfDocuments":[0-9]+' | head -n1 | grep -oE '[0-9]+' || true)"
    meili_total=$((meili_total + ${n:-0}))
  done

  # Vectorizer — aggregate over the per-repo collections.
  vec_total=0
  for family in code docs decisions turns governance analyses knowledge learnings consolidations topic_cards misc; do
    coll="cortex-${repo}-${family}"
    body="$(curl --max-time 5 -fsS "$VECTORIZER_URL/collections/$coll" 2>/dev/null || echo '{}')"
    n="$(printf '%s' "$body" | grep -oE '"vector_count":[0-9]+' | head -n1 | grep -oE '[0-9]+' || true)"
    vec_total=$((vec_total + ${n:-0}))
  done

  # Nexus — count Artifact nodes scoped to the repo.
  cypher="{\"query\":\"MATCH (a:Artifact) WHERE a.repo = '$repo' RETURN count(a) AS n\"}"
  body="$(curl --max-time 5 -fsS -H 'Content-Type: application/json' \
    -d "$cypher" "$NEXUS_URL/data/cypher" 2>/dev/null || echo '{}')"
  nexus_count="$(printf '%s' "$body" | grep -oE '"n":[0-9]+' | head -n1 | grep -oE '[0-9]+' || true)"
  nexus_count="${nexus_count:-0}"

  status="ok"
  if [[ "$meili_total" -gt 0 ]]; then
    vec_pct=$((vec_total * 100 / meili_total))
    nexus_pct=$((nexus_count * 100 / meili_total))
    if [[ "$vec_pct" -lt "$threshold_pct" || "$nexus_pct" -lt "$threshold_pct" ]]; then
      status="DIVERGED (vec=${vec_pct}%, nexus=${nexus_pct}%)"
      any_failure=1
    fi
  elif [[ "$vec_total" -gt 0 || "$nexus_count" -gt 0 ]]; then
    status="meili empty but downstream non-empty"
    any_failure=1
  fi

  printf '%-32s  %12d  %12d  %12d  %s\n' \
    "$repo" "$meili_total" "$vec_total" "$nexus_count" "$status"
done

if [[ "$any_failure" -ne 0 ]]; then
  echo
  echo "FAIL: at least one repo's downstream coverage is below ${threshold_pct}% of Meili." >&2
  echo "      Follow docs/cortex/pipeline-drainage-runbook.md to recover." >&2
  exit 1
fi
echo
echo "OK: every repo's downstream coverage is ≥ ${threshold_pct}% of Meili."

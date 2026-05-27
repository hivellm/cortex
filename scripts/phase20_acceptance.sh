#!/usr/bin/env bash
# phase20_retrieval-relevance-recovery — acceptance harness
#
# Runs all 10 success-criteria probes from proposal.md and fails
# fast on any miss. Each probe prints `PASS` or `FAIL: <reason>`
# and increments the appropriate counter. Exit 0 only when every
# probe passes.
#
# Usage:
#   scripts/phase20_acceptance.sh [API_URL]
#
# Defaults to http://127.0.0.1:17000.

set -uo pipefail

API="${1:-http://127.0.0.1:17000}"
PASS=0
FAIL=0
SKIP=0

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
yel()   { printf '\033[33m%s\033[0m\n' "$*"; }

probe() {
    local name="$1"; shift
    local expected_min="$1"; shift
    local body
    body="$("$@" 2>&1)"
    local actual
    actual="$(printf '%s' "$body" | python3 -c '
import json, sys
try:
    raw = sys.stdin.read()
    d = json.loads(raw)
    # generic counters
    if isinstance(d, dict):
        if "hits" in d and isinstance(d["hits"], list):
            print(len(d["hits"]))
            sys.exit(0)
        if "buckets" in d and isinstance(d["buckets"], list):
            print(len(d["buckets"]))
            sys.exit(0)
        if "results" in d:
            r = d["results"]
            if isinstance(r, dict) and "snippets" in r:
                print(len(r["snippets"]))
                sys.exit(0)
        if "coverage" in d:
            c = d["coverage"]
            backends = c.get("backends", []) if isinstance(c, dict) else []
            # report worst missing %
            worst = 0
            for b in backends:
                exp = b.get("expected", 0) or 0
                miss = b.get("missing", 0) or 0
                if exp > 0:
                    pct = int(100 * miss / exp)
                    worst = max(worst, pct)
            print(worst)
            sys.exit(0)
        if "active_tasks" in d:
            print(len(d["active_tasks"]))
            sys.exit(0)
        if "rows" in d:
            print(len(d["rows"]))
            sys.exit(0)
    print(0)
except Exception as e:
    print(0)
' 2>/dev/null)"

    if [[ -z "$actual" ]]; then actual=0; fi
    if [[ "$actual" -ge "$expected_min" ]]; then
        green "PASS [$name] actual=$actual >= $expected_min"
        PASS=$((PASS+1))
    else
        red   "FAIL [$name] actual=$actual < $expected_min"
        FAIL=$((FAIL+1))
    fi
}

# Reverse probe — pass when value is LOW (missing %)
probe_le() {
    local name="$1"; shift
    local expected_max="$1"; shift
    local body
    body="$("$@" 2>&1)"
    local actual
    actual="$(printf '%s' "$body" | python3 -c '
import json, sys
try:
    d = json.loads(sys.stdin.read())
    c = d.get("coverage", {}) if isinstance(d, dict) else {}
    worst = 0
    for b in c.get("backends", []):
        exp = b.get("expected", 0) or 0
        miss = b.get("missing", 0) or 0
        if exp > 0:
            pct = int(100 * miss / exp)
            worst = max(worst, pct)
    print(worst)
except Exception:
    print(100)
' 2>/dev/null)"
    if [[ "$actual" -le "$expected_max" ]]; then
        green "PASS [$name] missing=${actual}% <= ${expected_max}%"
        PASS=$((PASS+1))
    else
        red   "FAIL [$name] missing=${actual}% > ${expected_max}%"
        FAIL=$((FAIL+1))
    fi
}

echo "=== phase20 acceptance harness against $API ==="

# 1. cortex_query returns >=5 snippets per query
probe "1.query_snippets" 5 \
    curl -sS -X POST "$API/v1/query" -H 'content-type: application/json' \
    --data-raw '{"intent":"free_search","query":"phase20 retrieval relevance","scope":{"repo":"cortex"},"limit":20}'

# 2. Vectorizer coverage >=95% (missing <=5%)
probe_le "2.vectorizer_coverage_missing_pct" 5 \
    curl -sS "$API/v1/status"

# 3. Meili coverage >=95% (covered by same status backends array)
#    (single probe covers both — recorded as one probe; flag separately if needed)
probe_le "3.meili_coverage_missing_pct" 5 \
    curl -sS "$API/v1/status"

# 4. topic_search returns >=1 hit
probe "4.topic_search" 1 \
    curl -sS -X POST "$API/v1/topic-cards/search" -H 'content-type: application/json' \
    --data-raw '{"topic_prefix":"tool:claude-code","repo":"cortex","limit":5}'

# 5. consolidations_recent shows >=3 auto-generated docs over last 14d
#    (post-filter: model != manual-operator-*)
AUTO_DOCS="$(curl -sS "$API/v1/consolidations/recent?repo=cortex&limit=50" 2>/dev/null \
    | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
    auto = sum(1 for h in d.get("hits", [])
               if not (h.get("ext",{}).get("consolidation",{}).get("model","") or "").startswith("manual-operator"))
    print(auto)
except Exception:
    print(0)
' 2>/dev/null || echo 0)"
if [[ "$AUTO_DOCS" -ge 3 ]]; then
    green "PASS [5.consolidations_auto] $AUTO_DOCS auto-generated docs"
    PASS=$((PASS+1))
else
    red   "FAIL [5.consolidations_auto] $AUTO_DOCS auto-generated docs (need >=3)"
    FAIL=$((FAIL+1))
fi

# 6. consolidation_costs returns non-empty buckets
probe "6.costs_buckets" 1 \
    curl -sS -X POST "$API/v1/consolidations/costs" -H 'content-type: application/json' \
    --data-raw '{"since":"2026-04-01T00:00:00Z","until":"2026-05-27T23:59:59Z","group_by":["model","grain"],"repo":"cortex"}'

# 7. consolidation_lineage returns non-empty for a recent doc
LINEAGE_DOC_ID="$(curl -sS "$API/v1/consolidations/recent?repo=cortex&limit=1" 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["hits"][0]["id"] if d.get("hits") else "")' 2>/dev/null)"
if [[ -n "$LINEAGE_DOC_ID" ]]; then
    LINEAGE="$(curl -sS "$API/v1/consolidations/$LINEAGE_DOC_ID/lineage?repo=cortex" 2>/dev/null)"
    NON_EMPTY="$(printf '%s' "$LINEAGE" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
    n = (len(d.get("source_session_ids", []) or []) +
         len(d.get("decisions", []) or []) +
         len(d.get("files", []) or []))
    print(n)
except Exception:
    print(0)
' 2>/dev/null)"
    if [[ "$NON_EMPTY" -ge 1 ]]; then
        green "PASS [7.lineage] $NON_EMPTY refs"
        PASS=$((PASS+1))
    else
        red   "FAIL [7.lineage] empty (id=$LINEAGE_DOC_ID)"
        FAIL=$((FAIL+1))
    fi
else
    yel   "SKIP [7.lineage] no consolidations to probe"
    SKIP=$((SKIP+1))
fi

# 8. graph_query neighbors returns non-empty n.id
GRAPH="$(curl -sS -X POST "$API/v1/search/graph" -H 'content-type: application/json' \
    --data-raw '{"mode":"neighbors","node_id":"07H7BDPEWW3K6MDB08VNNF54JJ","depth":1}' 2>/dev/null)"
WITH_ID="$(printf '%s' "$GRAPH" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
    nodes = d.get("nodes", []) or []
    with_id = sum(1 for n in nodes if isinstance(n, dict) and n.get("id"))
    print(with_id)
except Exception:
    print(0)
' 2>/dev/null)"
if [[ "$WITH_ID" -ge 1 ]]; then
    green "PASS [8.graph_neighbors_with_id] $WITH_ID nodes carry n.id"
    PASS=$((PASS+1))
else
    red   "FAIL [8.graph_neighbors_with_id] no nodes carry n.id (writer not stamping properties)"
    FAIL=$((FAIL+1))
fi

# 9. law_violations with law_id filter returns matching subset
probe "9.law_violations_filtered" 1 \
    curl -sS -X POST "$API/v1/laws/violations" -H 'content-type: application/json' \
    --data-raw '{"repo":"cortex","law_id":"LAW-CORTEX-001","limit":5}'

# 10. active_work reflects on-disk active task
probe "10.active_work" 1 \
    curl -sS "$API/v1/active-work?repo=cortex"

echo
echo "=== summary ==="
echo "PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
exit 0

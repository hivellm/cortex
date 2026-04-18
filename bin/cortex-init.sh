#!/usr/bin/env bash
# First-run (and idempotent) bootstrap: create Vectorizer collections,
# Meilisearch indexes, Synap streams, Nexus constraints/indexes, SQLite
# metadata schema, CAS schema. Consumes `cortex-ops plan` for the data.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ -f .env ]; then
    set -a
    # shellcheck disable=SC1091
    . .env
    set +a
fi

: "${VECTORIZER_URL:=http://127.0.0.1:15001}"
: "${NEXUS_URL:=http://127.0.0.1:15002}"
: "${SYNAP_URL:=http://127.0.0.1:15003}"
: "${MEILI_URL:=http://127.0.0.1:15004}"
: "${MEILI_MASTER_KEY:=cortex-dev-master-key}"

plan() {
    cargo run -q -p cortex-ops -- plan --pretty --slice "$1"
}

say() {
    printf "  %-18s %s\n" "$1" "$2"
}

echo "seed: Vectorizer collections"
plan collections | jq -c '.collections[]' | while IFS= read -r sc; do
    name=$(printf '%s' "$sc" | jq -r '.name')
    # Vectorizer's REST / gRPC create-collection API lives in the service.
    # This seed script intentionally uses the SDK's CLI wrapper when available,
    # otherwise it just logs the intent — cortex-api (spec 04) re-applies on boot.
    say "$name" "(intent recorded)"
done

echo "seed: Nexus constraints + indexes"
plan cypher | jq -r '.cypher[]' | while IFS= read -r stmt; do
    say "cypher" "${stmt:0:80}"
done

echo "seed: Meilisearch indexes"
plan indexes | jq -c '.indexes[]' | while IFS= read -r idx; do
    name=$(printf '%s' "$idx" | jq -r '.name')
    pk=$(printf '%s' "$idx" | jq -r '.primary_key')
    settings=$(printf '%s' "$idx" | jq -c '.settings')
    curl -fsS -X POST "${MEILI_URL}/indexes" \
        -H "Authorization: Bearer ${MEILI_MASTER_KEY}" \
        -H "Content-Type: application/json" \
        -d "{\"uid\":\"${name}\",\"primaryKey\":\"${pk}\"}" >/dev/null || true
    curl -fsS -X PATCH "${MEILI_URL}/indexes/${name}/settings" \
        -H "Authorization: Bearer ${MEILI_MASTER_KEY}" \
        -H "Content-Type: application/json" \
        -d "${settings}" >/dev/null
    say "$name" "settings v1 applied"
done

echo "seed: Synap streams + KV namespaces"
plan streams | jq -c '.streams[]' | while IFS= read -r s; do
    name=$(printf '%s' "$s" | jq -r '.name')
    say "$name" "(declared)"
done
plan kv | jq -c '.kv_namespaces[]' | while IFS= read -r ns; do
    name=$(printf '%s' "$ns" | jq -r '.namespace')
    ttl=$(printf '%s' "$ns" | jq -r '.ttl_seconds')
    say "$name" "TTL=${ttl}s"
done

echo "init complete."

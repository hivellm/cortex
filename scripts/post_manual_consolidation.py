"""
Manual consolidation test — bypasses cortex-consolidator + LLM. Writes
a hand-crafted summary based on real session content (see
session_dump.txt for source) and POSTs it directly to
/v1/events. Verifies the dashboard renders the FULL body without
truncation.

Session source: 0MEZ8TKQMEXJEFCXAQE9ZSF1D8 (cortex, 2026-05-04T03:45
→ 2026-05-05T02:06, 42 turns + 669 tool calls).
"""
import hashlib
import json
import os
import urllib.request

SESSION_ID = "0MEZ8TKQMEXJEFCXAQE9ZSF1D8"
INGEST_URL = os.environ.get("CORTEX_INGESTION_URL", "http://127.0.0.1:17010") + "/v1/events"


def ulid_like(now_us: int) -> str:
    # Simple deterministic ULID-like id: 26 chars, Crockford-ish base32.
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    raw = now_us.to_bytes(10, "big")
    out = []
    bits = 0
    val = 0
    for b in raw:
        val = (val << 8) | b
        bits += 8
        while bits >= 5:
            bits -= 5
            out.append(alphabet[(val >> bits) & 0x1F])
    return ("".join(out) + "0" * 26)[:26]


def build_envelope() -> dict:
    title = "Cortex ops sweep: CPU debug, vectorizer 3.3.0, parquet quarantine"
    assert len(title) <= 80, f"title {len(title)} chars > 80"

    summary = (
        "Single Cortex operator session covering 22h on 2026-05-04/05.\n\n"
        "## Diagnostico 172% CPU\n"
        "cortex-api consumindo 50% por archive_loader em loop 30s sobre 104 "
        "parquets sem cache mtime, decomprime zstd integral por tick + "
        "parquet corrompido em year=2026/month=04/day=28/hour=16 (loga WARN "
        "indefinido, sem quarentena automatica). cortex-nexus 100% sem "
        "queries 6+min, sem error/warn, busy-loop interno (healthcheck "
        "falso-unhealthy por `curl` ausente na imagem).\n\n"
        "## Fix aplicado\n"
        "Quarentena manual do parquet corrompido em "
        "/var/lib/cortex/archive/quarantine/raw-00000.parquet.corrupt "
        "as 07:29 UTC. Loop WARN parou. Cortex-nexus continua sob "
        "investigacao.\n\n"
        "## Vectorizer 3.2.0 -> 3.3.0\n"
        "Issue #265 fechada: endpoint POST /collections/{src}/vectors/move "
        "+ SDK methods delete_vector / delete_vectors / move_to_collection. "
        "Bumps: docker-compose.yml linha 3, Cargo.toml linha 56, Cargo.lock "
        "(vectorizer-sdk + vectorizer-protocol). Quality gate: cargo check "
        "--workspace PASS, docker pull hivehub/vectorizer:3.3.0 OK, "
        "docker compose up -d vectorizer aplicado.\n\n"
        "## Saldo da sessao\n"
        "42 turns substantivos, 669 tool calls, ~33k chars de prosa real. "
        "Trabalho cobre debugging de producao + atualizacao de dependencia "
        "+ documentacao em rulebook. Conteudo digno de consolidacao "
        "permanente."
    )
    assert 200 <= len(summary.encode("utf-8")) <= 2000, (
        f"summary {len(summary.encode('utf-8'))} bytes outside [200, 2000]"
    )

    payload = {
        "consolidation_id": "cons-ses-MANUAL-TEST-0ME-2026-05-19",
        "grain": "session",
        "scope": {"kind": "session_id", "value": SESSION_ID},
        "title": title,
        "summary_markdown": summary,
        "takeaways": [
            "archive_loader precisa cache mtime + quarentena automatica de parquets corrompidos",
            "healthchecks docker baseados em `curl` falham silenciosamente quando o binario nao existe na imagem",
            "vectorizer 3.3.0 ja oferece move_to_collection nativo (era hack via reqwest antes)",
        ],
        "source_event_ids": [
            "01KQRHE5GA4V5QWJ7EGNPT7V7H",
            "01KQRHFR210PQER4FANSR121N3",
            "01KQRVEJY582M53XANY0HD91DF",
            "01KQRVY6BKKX8VE71YYFFJ2MJW",
            "01KQRY7R02C5693QEA59027F28",
            "01KQRYCN43QPCEXS0TWSTDD6YB",
            "01KQTCP84HJTEEXB7Z59B2MPBG",
            "01KQTCZCPVXWNH1JYDHRB0EVQZ",
        ],
        "source_event_count": 711,
        "model": "manual-operator-2026-05-19",
        "depth": "shallow",
        "outcome_distribution": {"success": 6, "info": 2},
        "temporal_span": {
            "start_ms": 1778262315274,
            "end_ms": 1778335594145,
            "duration_ms": 73278871,
        },
        "repos": ["cortex"],
        "tags": ["manual-test", "phase13b"],
    }
    payload_str = json.dumps(payload, sort_keys=True)
    content_hash = "sha256:" + hashlib.sha256(payload_str.encode("utf-8")).hexdigest()

    envelope = {
        "event_id": ulid_like(0x1779188888888888),
        "schema_version": "1",
        "occurred_at": "2026-05-19T11:15:00Z",
        "session_id": SESSION_ID,
        "stream": "live",
        "tool": "claude-code",
        "kind": "consolidation",
        "context": {"repo": "Cortex", "platform": "linux"},
        "payload": payload,
        "content_hash": content_hash,
    }
    return envelope


def main() -> int:
    env = build_envelope()
    body = json.dumps(env).encode("utf-8")
    req = urllib.request.Request(
        INGEST_URL,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        resp = urllib.request.urlopen(req)
        text = resp.read().decode("utf-8")
        print(f"HTTP {resp.status}: {text}")
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code}: {e.read().decode('utf-8')}")
        return 1
    print(f"posted event_id={env['event_id']}")
    print(f"summary bytes: {len(env['payload']['summary_markdown'].encode('utf-8'))}")
    print(f"title chars: {len(env['payload']['title'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

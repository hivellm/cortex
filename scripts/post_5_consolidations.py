"""
Five hand-written consolidation envelopes based on real session
content extracted from the archive. Each summary written by an
operator after reading the actual Turn / ToolCall content, not by
the consolidator's LLM call. Used to seed the dashboard with
quality records the user can review.
"""
import hashlib
import json
import os
import urllib.request

INGEST_URL = os.environ.get("CORTEX_INGESTION_URL", "http://127.0.0.1:17010") + "/v1/events"


def ulid_like(seed: int) -> str:
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    raw = seed.to_bytes(10, "big")
    out, bits, val = [], 0, 0
    for b in raw:
        val = (val << 8) | b
        bits += 8
        while bits >= 5:
            bits -= 5
            out.append(alphabet[(val >> bits) & 0x1F])
    return ("".join(out) + "0" * 26)[:26]


SESSIONS = [
    {
        "seed": 0xAAAA000000000001,
        "session_id": "1FCFZ7M77VJ2356QAXPYP8GCFT",
        "occurred_at": "2026-05-17T20:05:00Z",
        "repo": "ar-v1-documentacao",
        "consolidation_id": "cons-ses-MANUAL-1FCF-realiza-docs",
        "title": "RealizaTi infrastructure documentation pass: 10 parallel agents, 3 critical findings, full pt-BR atlas",
        "summary": (
            "Four-day documentation pass over the entire RealizaTi/ar-v1 ecosystem "
            "(2026-05-13 to 2026-05-17). User-driven mandate: catalog every repo, "
            "group by theme, identify which are actually live, produce pt-BR docs "
            "with an index for developer onboarding.\n\n"
            "## Execution shape\n"
            "10 background agents spawned in parallel, one per thematic group "
            "(infra/security, microservices, mobile, dashboards, legacy, etc.). "
            "Each agent wrote directly to "
            "`E:/RealizaTi/ar-v1-documentacao/<grupo>/README.md`. Master cross-group "
            "connection map + status table consolidated at the end.\n\n"
            "## Critical findings\n"
            "1. **vault repo is an empty shell** — cloned from `deploy/vault.git` "
            "but contains only the default GitLab README template. No HashiCorp "
            "Vault config versioned anywhere. Secrets live in scattered `.env` "
            "files on VMs. High risk.\n"
            "2. **PostgreSQL backup gap** — `backup-postgresql` is not even a git "
            "repo, just a local drop zone with `.sql.gz` dumps (latest "
            "`ar_email_prod.sql_20260511.gz`). No automated pipeline, no S3 "
            "upload, unlike the `mongodb-s3-backup` which has full Docker + S3 "
            "fallback.\n"
            "3. **PKI ca.key committed** — the internal CA private key is "
            "versioned alongside `ca.crt`. Anyone with repo access can forge "
            "valid certs for `*.dev.ar` / `*.qa.ar`. Cert validity 9000 days "
            "(~24y) without rotation."
        ),
        "takeaways": [
            "RealizaTi has ~50 repos but only ~30% are actually active per git activity windows",
            "vault repository is a phantom (template README only); centralized secret mgmt is a myth",
            "postgres backup pipeline must be promoted from `.sql.gz` drop zone to a real S3-backed cron",
            "PKI key rotation needs to happen — current CA key + 24y cert lifetimes are an audit liability",
        ],
        "source_event_count": 3531,
        "model": "manual-operator-2026-05-19",
        "depth": "deep",
        "outcome_distribution": {"success": 10, "info": 3, "error": 0},
        "temporal_span": {"start_ms": 1779013915686, "end_ms": 1779415503376, "duration_ms": 401587690},
        "repos": ["ar-v1-documentacao", "15-servidores-producao"],
        "tags": ["manual-test", "documentation", "infrastructure-audit"],
        "source_event_ids": [
            "01KRGAVDD6XNZ4TT0EQR9D7Y6M", "01KRGAXFABNMBXBZA6AYZ7X5SV",
            "01KRGB5CN7Z5Q3JB1R44XPM57Q", "01KRGBAEJ5REZAS1P981VB1J06",
        ],
    },
    {
        "seed": 0xAAAA000000000002,
        "session_id": "04MVCX9XXPTFBN1PMFDHC8346V",
        "occurred_at": "2026-05-04T04:27:00Z",
        "repo": "vectorizer",
        "consolidation_id": "cons-ses-MANUAL-04MV-sdk-go-csharp",
        "title": "Vectorizer phase20: Go + C# SDK parity vs Rust REST surface, 11-phase parallel agent rollout",
        "summary": (
            "Implementation pass for `phase20_sdk-go-csharp-rest-parity` on the "
            "Vectorizer repo. 4.3h of sustained work parallelized across 11 phase "
            "agents covering admin, observability, collections, vectors, search, "
            "indexing, and migration surfaces.\n\n"
            "## Surface implemented\n"
            "Go: `sdks/go/admin.go` (206 lines, 17 admin/observability methods), "
            "plus the remaining collection/vector/search/indexing files. C# "
            "mirror via `Vectorizer.Sdk` namespace. Both wrap the same REST "
            "endpoints the Rust SDK already exposed.\n\n"
            "## Key design decisions\n"
            "- `GetStats()` name collision in Go: `client.go` already had it "
            "returning `*DatabaseStats`. Admin variant renamed `GetServerStats() "
            "-> *Stats` to expose the richer `Stats` payload (uptime + version).\n"
            "- Envelope unwrap consistency: `GetLogs` strips "
            "`{\"logs\":[...]}`, `ListBackups` strips `{\"backups\":[...]}`, "
            "`ListWorkspaces` strips `{\"workspaces\":[...]}` — all cross-verified "
            "against `admin.rs` line numbers cited in the per-agent report.\n"
            "- `ConfigSnapshot`, `WorkspaceConfig` returned as named "
            "`map[string]interface{}` types rather than bare maps so the public "
            "API stays self-documenting.\n"
            "- `ListEmptyCollections` handles both bare-array and "
            "envelope-wrapped responses per Rust source comment.\n\n"
            "## Verification\n"
            "`go build ./...` exit 0, no warnings. dotnet build OK across both "
            "SDKs. Cross-agent dependencies (phases waiting on phases) resolved "
            "in a single coordination pass."
        ),
        "takeaways": [
            "Parallel agent dispatch saves wall time on SDK ports when phase boundaries are well-defined",
            "Cross-language SDK parity work benefits from explicit Rust line citations per method",
            "Go's no-overload constraint forces rename of `GetStats`/`GetServerStats` even when they hit the same route",
        ],
        "source_event_count": 1811,
        "model": "manual-operator-2026-05-19",
        "depth": "shallow",
        "outcome_distribution": {"success": 11, "info": 1},
        "temporal_span": {"start_ms": 1778199277482, "end_ms": 1778214830811, "duration_ms": 15553329},
        "repos": ["vectorizer"],
        "tags": ["manual-test", "sdk", "parity", "phase20"],
        "source_event_ids": [
            "01KQR5099AZN4CRNQAQ5SR1RMB", "01KQR5Q51ZR5RDPDRPB04903HV",
            "01KQR5QG6MV8YX4DDSV87EX9MP", "01KQR5QJVAW7VQP821PZY9T4QJ",
        ],
    },
    {
        "seed": 0xAAAA000000000003,
        "session_id": "0F22VYYZVD7RET96S3DP6MRTHW",
        "occurred_at": "2026-05-06T20:37:52Z",
        "repo": "cortex",
        "consolidation_id": "cons-ses-MANUAL-0F22-cortex-rework-analysis",
        "title": "Cortex rework analysis: 4 agents diagnose 80% structural debt, propose 7 ADRs and 3-phase recovery",
        "summary": (
            "Operator-mandated review of Cortex implementation after frustration: "
            "consolidation does not work, memory cleanup requires brute force, "
            "retrieved data has no relevance. Five-file analysis in "
            "`docs/analysis/rework/` (1142 lines) produced by 4 agents in "
            "parallel (3 researchers + 1 architect).\n\n"
            "## Diagnosis\n"
            "**80% structural debt / 20% upstream patches.** Stack itself "
            "(Synap/Vectorizer/Nexus/Meili/SQLite) is correct; the plumbing "
            "between them is ad-hoc.\n\n"
            "## Top defects identified\n"
            "- **Consolidator silent in prod**: no nightly daemon, only manual CLI. "
            "`publish_consolidation()` posts to `localhost:17010` which is not "
            "reachable from inside the container — every envelope drops.\n"
            "- **Memory cleanup brute force**: archive Parquet only has per-event "
            "delete via `/v1/admin/forget` (no cron sweep). CAS vacuum silent-fails "
            "behind a 50% safeguard. Tool-call digest is opt-in.\n"
            "- **Relevance death**: queries without `scope.repo` fall to "
            "`cortex-unknown-*` (zero hits). 2 of 3 repos have empty Meili indexes. "
            "Graph topology flat (2 of ~12 edge types in use).\n"
            "- **Common root**: 6 retention gaps + consolidator gap are all the "
            "same shape — missing `Sweep` trait, missing `EnvelopeProducer` trait, "
            "missing typed `Lane`, missing `EventIdentity`.\n\n"
            "## Recommendation\n"
            "Medium rework — Phase A (10d, codify traits, no new feature), "
            "Phase B (10d, rewrite consolidator/pruner/dashboard on top of "
            "traits), Phase C (10d, multi-repo + golden-set eval). Phase A is a "
            "mandatory gate. 7 ADRs (009-015) drafted with explicit trade-offs."
        ),
        "takeaways": [
            "Cortex stack choice is defensible; the inter-module plumbing is the regret",
            "Six retention bugs share one shape: no Sweep trait — that is the load-bearing fix",
            "Consolidator never ran in prod because publish target was unreachable inside the container",
            "Relevance failures trace to missing scope.repo defaults + empty per-repo Meili indexes",
        ],
        "source_event_count": 558,
        "model": "manual-operator-2026-05-19",
        "depth": "deep",
        "outcome_distribution": {"success": 5, "info": 3},
        "temporal_span": {"start_ms": 1778457382337, "end_ms": 1778526672220, "duration_ms": 69289883},
        "repos": ["cortex"],
        "tags": ["manual-test", "rework", "architecture", "adr"],
        "source_event_ids": [
            "01KQXEVKT13JGQB1HB8CFNE1B6", "01KQXEWDTB03QBN1QAHY8SR76W",
            "01KQXFKEMPFACAHZKD6ARAR5Y5", "01KQXFY82TXM1PPRQ2GBD61KV5",
            "01KQXGA7ZH86DK5H8121WYEB44",
        ],
    },
    {
        "seed": 0xAAAA000000000004,
        "session_id": "07H7BDPEWW3K6MDB08VNNF54JJ",
        "occurred_at": "2026-05-05T00:19:51Z",
        "repo": "nexus",
        "consolidation_id": "cons-ses-MANUAL-07H7-nexus-cpu-debug",
        "title": "Nexus 100% CPU debug: missing property indexes + broken distroless healthcheck",
        "summary": (
            "Investigation of cortex-nexus container saturating CPU. 16h debug "
            "session producing root-cause analysis + two follow-up rulebook "
            "tasks.\n\n"
            "## Root cause #1 — missing property indexes (CRITICAL)\n"
            "`CALL db.indexes()` reveals only label-lookup indexes (`Artifact`, "
            "`Turn`, `ToolCall`, `Symbol`). Zero indexes on properties despite "
            "production traffic doing:\n"
            "- `MERGE (n:Artifact { natural_key: $k })` from Vectorizer ingestion "
            "  → full scan on every MERGE\n"
            "- `WHERE a.path CONTAINS $q OR a.natural_key CONTAINS $q` → "
            "  substring scan over entire Artifact + TOUCHED relation set\n"
            "- `MATCH (a:Session)-[r:HAS_TURN]->(b:Turn) ... LIMIT 30000` × 4 "
            "  variants every ~30s → full scans in a loop\n\n"
            "Single-writer Rust model + scans on growing Artifact graph "
            "(Vectorizer running for hours with E:\\HiveLLM\\Vectorizer\\sdks\\csharp\\*.cs "
            "paths) → every new query queues behind a multi-minute scan.\n\n"
            "## Root cause #2 — broken healthcheck (cosmetic)\n"
            "`FailingStreak: 23796`, `Output: /bin/sh: line 1: curl: command not found`. "
            "Commit `ee69e117` switched runtime to distroless trixie but the "
            "HEALTHCHECK in the Dockerfile still calls `curl`. Container shows "
            "unhealthy though it works. Not the CPU cause but pollutes operator "
            "signal.\n\n"
            "## Follow-ups created\n"
            "- `phase6_merge-unindexed-property-warning` — planner emits Neo4j-"
            "compatible Notification + rate-limited WARN when MERGE/MATCH "
            "selects by `(label, prop)` without an index.\n"
            "- `phase6_slow-query-log-and-active-queries` — RAII registry of "
            "active queries + tick log + `GET /admin/queries` endpoint + "
            "`nexus.queries.list` procedure. Solves \"100% CPU with no way to "
            "know what's running\"."
        ),
        "takeaways": [
            "Nexus shipped without property indexes — first production traffic exposed it as a full-scan trap",
            "Distroless image migrations must update HEALTHCHECK commands (curl/wget often absent)",
            "Single-writer graph engines need active-query introspection to debug saturation",
        ],
        "source_event_count": 682,
        "model": "manual-operator-2026-05-19",
        "depth": "shallow",
        "outcome_distribution": {"success": 4, "info": 2},
        "temporal_span": {"start_ms": 1778225575597, "end_ms": 1778285991356, "duration_ms": 60415759},
        "repos": ["nexus"],
        "tags": ["manual-test", "performance", "debugging", "cpu"],
        "source_event_ids": [
            "01KQRYF1ND21V1GSZD5FBZCZCF", "01KQRYXX1M7PQSTPDJV112SG4H",
            "01KQRYYEXE042D2TDHK6M02MR1", "01KQRZ94QB2RNGP48EJZAR0DG3",
        ],
    },
    {
        "seed": 0xAAAA000000000005,
        "session_id": "1P3493S6R8BBKNDMAE6G2EHM44",
        "occurred_at": "2026-05-06T10:54:37Z",
        "repo": "rulebook",
        "consolidation_id": "cons-ses-MANUAL-1P34-rulebook-opencode",
        "title": "Rulebook phase2_add-opencode-support shipped: 57-item checklist closed",
        "summary": (
            "Continuation session for open Rulebook tasks. Primary outcome: "
            "phase2_add-opencode-support landed (57 of 57 items checked, status "
            "completed). 1.6h of focused work.\n\n"
            "## Phase summary\n"
            "Added OpenCode (the open-source successor / sibling to the official "
            "Claude Code adapter) as a first-class harness inside Rulebook. The "
            "task covered: adapter discovery and templating, OpenCode-specific "
            "hook shim mappings, CLAUDE.md and AGENTS.md cross-references, "
            "settings.json schema additions, and the cortex-adapter hooks that "
            "share the existing `cortex-hook` binary path.\n\n"
            "## Why this matters\n"
            "Rulebook had a Claude Code-only assumption baked into multiple "
            "templates and the adapter discovery loop. OpenCode users could "
            "install Rulebook but the hook auto-wiring would silently skip — no "
            "indexing into Cortex from the OpenCode side. The phase normalized "
            "the adapter discovery, so any future harness (Cursor, Codex, "
            "Gemini, etc.) ships as a thin descriptor entry rather than a "
            "fork.\n\n"
            "## Tail validations\n"
            "- All 57 checklist items marked `[x]` and persistently stored\n"
            "- Mandatory tail (docs + tests + verify) green per v5.3.0 archive "
            "gate\n"
            "- Archive performed cleanly into `.rulebook/archive/<date>-phase2_"
            "add-opencode-support/`"
        ),
        "takeaways": [
            "Adapter-agnostic harness discovery in Rulebook unblocks OpenCode + future SDKs without per-tool branches",
            "Rulebook v5.3.0 task-tail gate (docs + tests + verify) catches `[ ]` items pre-archive — confirmed working",
            "phase2_add-opencode-support is the template; phase16a-d (Cursor/Codex/Gemini) inherit the same shape",
        ],
        "source_event_count": 518,
        "model": "manual-operator-2026-05-19",
        "depth": "shallow",
        "outcome_distribution": {"success": 3, "info": 1},
        "temporal_span": {"start_ms": 1778483739631, "end_ms": 1778489677923, "duration_ms": 5938292},
        "repos": ["rulebook"],
        "tags": ["manual-test", "rulebook", "opencode", "adapters"],
        "source_event_ids": [
            "01KQY94K7FF2HTH6R5Q36CBV9R", "01KQYAAPJQEWCAXQE3XG1C0Z8S",
        ],
    },
]


def build_envelope(rec: dict) -> dict:
    payload = {
        "consolidation_id": rec["consolidation_id"],
        "grain": "session",
        "scope": {"kind": "session_id", "value": rec["session_id"]},
        "title": rec["title"],
        "summary_markdown": rec["summary"],
        "takeaways": rec["takeaways"],
        "source_event_ids": rec["source_event_ids"],
        "source_event_count": rec["source_event_count"],
        "model": rec["model"],
        "depth": rec["depth"],
        "outcome_distribution": rec["outcome_distribution"],
        "temporal_span": rec["temporal_span"],
        "repos": rec["repos"],
        "tags": rec["tags"],
    }
    payload_str = json.dumps(payload, sort_keys=True)
    content_hash = "sha256:" + hashlib.sha256(payload_str.encode("utf-8")).hexdigest()
    return {
        "event_id": ulid_like(rec["seed"]),
        "schema_version": "1",
        "occurred_at": rec["occurred_at"],
        "session_id": rec["session_id"],
        "stream": "live",
        "tool": "claude-code",
        "kind": "consolidation",
        "context": {"repo": rec["repo"], "platform": "linux"},
        "payload": payload,
        "content_hash": content_hash,
    }


def main() -> int:
    ok = 0
    for i, rec in enumerate(SESSIONS, 1):
        env = build_envelope(rec)
        body = json.dumps(env).encode("utf-8")
        req = urllib.request.Request(
            INGEST_URL,
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        title_chars = len(rec["title"])
        summary_bytes = len(rec["summary"].encode("utf-8"))
        try:
            resp = urllib.request.urlopen(req)
            text = resp.read().decode("utf-8")
            print(f"[{i}/5] {rec['repo']}: HTTP {resp.status} | title={title_chars}c | summary={summary_bytes}b | {text}")
            ok += 1
        except urllib.error.HTTPError as e:
            print(f"[{i}/5] {rec['repo']}: HTTP {e.code} | title={title_chars}c | summary={summary_bytes}b | {e.read().decode('utf-8')}")
    print(f"posted {ok}/5")
    return 0 if ok == 5 else 1


if __name__ == "__main__":
    raise SystemExit(main())

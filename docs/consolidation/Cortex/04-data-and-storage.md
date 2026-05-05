# Cortex — Data Model & Storage

## Core Entity Types (Nexus Nodes)

All captured and derived entities map to labeled nodes in the Nexus graph:

| Entity | Purpose | Emitted by |
|--------|---------|------------|
| **Session** | One AI session (Claude Code IDE, agent invocation, conversation) | adapter or bootstrap |
| **Turn** | Single exchange (user prompt → assistant response) | adapter (Stop hook, commit 15b8931) |
| **ToolCall** | Tool invocation (Bash, Edit, Read, MCP tool, etc.) | adapter (PostToolUse hook) |
| **AgentCall** | Sub-agent invocation | adapter (SubagentStop hook) |
| **Artifact** | File, diff, snippet, external resource referenced by other entities | adapter or bootstrap |
| **Memory** | Persisted entry (user, feedback, project reference) | bootstrap (from `.rulebook/memory/`) |
| **Decision** | Formalized decision record (ADR-style) | bootstrap (from `.rulebook/decisions/`) |
| **Analysis** | Deep-analysis report on complex topic | bootstrap (from `docs/analysis/`) |
| **Topic** | Classifier-assigned theme (e.g., "auth", "performance", "ci") | cortex-classifier-worker |
| **Law** | Development rule that must be followed | bootstrap (from `.rulebook/specs/*/laws/`) |
| **LawViolation** | Observed breach of a Law | (governance engine not yet built) |

## Relation Types (Nexus Edges)

Cortex builds a property graph with typed relationships:

```
(Session)-[:CONTAINS]->(Turn)
(Turn)-[:INVOKED]->(ToolCall|AgentCall)
(ToolCall)-[:TOUCHED|READ|WROTE|EXECUTED]->(Artifact)
(Turn)-[:PRODUCED]->(Memory|Decision|Analysis)
(Decision)-[:SUPERSEDES]->(Decision)
(Decision)-[:REFERENCES]->(Analysis|Memory|Artifact)
(LawViolation)-[:OF]->(Law)
(LawViolation)-[:OBSERVED_IN]->(Turn|ToolCall)
(*)-[:ABOUT]->(Topic)
(*)-[:MENTIONS]->(Entity)
(Session|ToolCall|Artifact)-[:IN_REPO]->(Repo)
(*)-[:SIMILAR_TO {score}]->(*)   // derived via Vectorizer KNN
```

**Current topology (2026-04-28 audit):** Only `IN_REPO` (10245 edges) and `REMEMBERS` (30 edges) exist. Symbol-level relations (`DEFINES`, `CALLS`, `IMPORTS`) not yet emitted (phase4c).

## Meilisearch Indexes (Full-Text)

Per-project isolation: **`cortex-{repo}-{family}`** (decision 006, routing via `cortex-fulltext/src/routing.rs:105-107`).

| Index | Scope | Coverage (2026-04-28) |
|-------|-------|----------------------|
| `cortex-cortex-misc` | Cortex repo, miscellaneous events | ✅ ~589 docs |
| `cortex-cortex-decisions` | Cortex decisions | ✅ Populated |
| `cortex-vectorizer-misc` | Vectorizer repo | ❌ Not indexed despite bootstrap walker coverage |
| `cortex-rulebook-misc` | Rulebook repo | ❌ Not indexed despite bootstrap walker coverage |
| `cortex-synap-misc` | Synap repo | ❌ Not indexed despite bootstrap walker coverage |
| (legacy) `cortex-v0-*` | Pre-slug naming | 🟡 6 empty indexes not cleaned up (phase4a cleanup) |

**Issue:** Worker consumer state or offset tracking not persisting across restarts. Phase4a diagnosis planned.

## Vectorizer Collections (Semantic Embeddings)

Per-project collection isolation: **`cortex-{repo}-{family}`** (learning 2026-04-27).

| Collection | Vector dim | Chunks (approx) |
|------------|------------|-----------------|
| `cortex-cortex-code` | 512 | ~128k (3 repos total across all families) |
| `cortex-vectorizer-code` | 512 | (partial; source of upsert drift) |
| `cortex-rulebook-code` | 512 | (partial; source of upsert drift) |

**Known drifts:**
- `total_failed=4-5` reported per batch despite vectors being queryable downstream (Vectorizer SDK 3.0.3 issue, anti-pattern recorded).
- No `/list_vectors` path in SDK; worked around by `LiveVectorizerClient::list_stored_chunk_ids` (reqwest direct).

## Archive (Parquet + Zstd)

**Location:** `${CORTEX_ARCHIVE_ROOT}` (default `~/.cortex/archive/`; docker mount `/var/lib/cortex/archive`).

**Structure:** Immutable Parquet-zstd shards, one per batch. Spec 02. Enables:
- Full-text search on raw text (fallback if Meili fails).
- Compliance audit trail.
- Bootstrapping new Cortex instances.

**Consumer state:** SQLite offset table co-located. Decision 008 — durable resumability without relying on upstream stream state.

## Redaction & PII Handling

**Redactors applied before any persistence:**
- Secret patterns (API keys, passwords, JWTs, `.env` values).
- Email addresses and phone numbers.
- Detected PII (names, addresses, identifiers).

See `cortex-core/src/redactor.rs` for rules. Redaction happens at ingestion boundary (adapter and bootstrap); all downstream sees sanitized payloads.

## Data Flow and Lifecycle

```
┌─ Raw event ingestion
│  (adapter or bootstrap) → Synap cortex.events.raw|bootstrap
│                         → cortex-storage archive (Parquet + Zstd)
│
├─ Enrichment
│  (classifier-worker)   → Synap cortex.events.enriched (+ Parquet archive)
│                        → per-event topics, severity, PII risk assessment
│
├─ Indexing (fan-out to three backends)
│  ├─ Vectorizer         (cortex-embedder-worker)
│  │                     → semantic search on Tree-sitter chunks
│  ├─ Nexus              (cortex-graph-worker)
│  │                     → property graph (nodes + typed edges)
│  └─ Meilisearch        (cortex-fulltext-worker)
│                        → keyword search (BM25 + facets)
│
└─ Retrieval
   (cortex-api)          → RRF fusion of vector + keyword + graph lanes
                         → dashboard aggregator (reads Meili directly for some views)
                         → pre-thinking bundle assembly
```

## Consistency Issues & Known Gaps

| Gap | Symptom | Workaround | Planned Fix |
|-----|---------|------------|-------------|
| **Vectorizer upsert reporting** | SDK returns `total_failed=4-5/batch` but vectors are queryable | Trust queryable-downstream signal; instrument post-upsert verification | Phase4d (doctor consistency check) |
| **Nexus UNWIND silently drops** | Batch writes don't land; SDK returns 200 OK | Per-row Cypher + post-write assert (commit 5bd0185) | Track Nexus releases for UNWIND fix |
| **Symbol-level topology missing** | Chunker emits `symbol` field but mapper drops it | No workaround; flat graph | Phase4c (emit symbol relations) |
| **Meili single-repo coverage** | Only Cortex repo indexed of 3 walked | Use archive for fallback search | Phase4a (diagnose worker offset, cleanup legacy indexes) |

## Retention Policy (Not Yet Formalized)

Archive is immutable and unbounded. Planned retention tiers:
- Hot (1 year): Full document + metadata in all backends.
- Warm (5 years): Archive-only with on-demand reindex.
- Cold (forever): Immutable archive for compliance.

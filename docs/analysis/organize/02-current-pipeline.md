# 02 — Cortex ingestion + retrieval pipeline (current state)

Reference for where every new Claude-archive envelope lands.
Every line in this file is verified against the source tree at
`e:\HiveLLM\Cortex` on 2026-05-01.

## 1. Envelope canonical shape

[`crates/cortex-core/src/events.rs:14-47`](../../../crates/cortex-core/src/events.rs#L14-L47)

```rust
pub struct Envelope {
    pub event_id: Ulid,
    pub schema_version: String,        // "1"
    pub occurred_at: DateTime<Utc>,
    pub ingested_at: Option<DateTime<Utc>>,
    pub session_id: Option<String>,
    pub stream: Stream,                // Live | Bootstrap
    pub tool: String,                  // "claude-code", "openai-codex", "bootstrap"
    pub model: Option<String>,
    pub kind: Kind,
    pub context: Context,              // repo, branch, commit, cwd, user, ide, extras
    pub payload: serde_json::Value,    // matches per-Kind schema
    pub redactions: Vec<Redaction>,
    pub content_hash: Option<String>,
    pub parent_event_id: Option<Ulid>,
}
```

`Kind` (events.rs:62-90): `Turn`, `ToolCall`, `AgentCall`, `Memory`,
`Decision`, `Analysis`, `LawViolation`, `Artifact`, `Knowledge`,
`Learning`.

## 2. Flow diagram (verified)

```
┌──────────────────────────────────────┐
│ Source                               │
│   adapter (claude-code-adapter)      │
│   bootstrap (cortex-bootstrap CLI)   │
│   archive_loader (cortex-api boot)   │
│   *** cortex-claude-archive ***  ←   │  new in this plan
│       (~/.claude/projects)           │
└──────────────────┬───────────────────┘
                   │ POST /v1/events  or  direct Synap publish
                   ▼
┌──────────────────────────────────────┐
│ cortex-ingestion                     │
│   crates/cortex-ingestion/src/main.rs│
│   router: src/router.rs:66           │
│     1. pick_stream()                 │
│     2. stamp_server_fields()         │
│     3. cortex_core::redact()         │
│     4. cortex_core::validate_event() │
│     5. archive.write() (parquet/zstd)│
│     6. publisher.publish(stream, env)│
└──────────────────┬───────────────────┘
                   │ Synap
                   ▼
        cortex.events.raw            cortex.events.bootstrap
                   │                           │
                   └─────────┬─────────────────┘
                             ▼
              ┌─────────────────────────────┐
              │ cortex-classifier-worker    │
              │   classify_batch()          │
              │   stamps topics, severity   │
              └─────────────┬───────────────┘
                            ▼
                    cortex.events.enriched
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
┌─────────────┐  ┌──────────────┐  ┌──────────────┐
│ embedder    │  │ fulltext     │  │ graph-writer │
│ (FP32 + PQ) │  │ (Meili docs) │  │ (Cypher)     │
└──────┬──────┘  └──────┬───────┘  └──────┬───────┘
       ▼                ▼                 ▼
┌────────────┐  ┌─────────────┐  ┌────────────┐
│ Vectorizer │  │ Meilisearch │  │ Nexus      │
└────┬───────┘  └──────┬──────┘  └────┬───────┘
     │                 │              │
     └──────┬──────────┴──────────────┘
            ▼
     ┌────────────────┐
     │ cortex-api     │
     │ /v1/query      │   ← RRF fusion
     └────────┬───────┘
              ▼
     ┌──────────────────────┐
     │ cortex-pre-thinking  │
     │ (markdown bundle)    │
     └──────────────────────┘
```

## 3. Family taxonomy

`code, docs, decisions, turns, governance, analyses, knowledge,
learnings, misc` — declared in
[`crates/cortex-workers/src/fulltext/routing.rs:213`](../../../crates/cortex-workers/src/fulltext/routing.rs#L213).

Per-Kind routing (`routing.rs:37-110`):

| `Kind` | Family |
|---|---|
| `ToolCall` | `code` |
| `Turn` | `turns` |
| `AgentCall` | `turns` |
| `Decision` | `decisions` |
| `LawViolation` | `governance` |
| `Analysis` | `analyses` |
| `Knowledge` | `knowledge` |
| `Learning` | `learnings` |
| `Memory` | `misc` |
| `Artifact` | extension lookup → `code` / `docs` / `misc` |

Collection naming: `cortex-{slug}-{family}` for per-repo, plus
global indexes `cortex_turns`, `cortex_decisions`, `cortex_analyses`,
`cortex_memories`, `cortex_laws`, `cortex_knowledge`,
`cortex_learnings`. Slug rule:
[`cortex-storage/src/names.rs:153-186`](../../../crates/cortex-storage/src/names.rs#L153-L186).

`slug_for_repo("Claude Archive") == "claude-archive"` — valid out of
the box; no naming changes required.

## 4. Bootstrap CLI

[`crates/cortex-cli/src/bin/cortex-bootstrap.rs`](../../../crates/cortex-cli/src/bin/cortex-bootstrap.rs)
walks **git repos**: it reads `cortex.toml` (or workspace override —
phase11 fix), uses `git2` to enumerate commits, and emits one
`Envelope` per artifact with `stream=Bootstrap`.

**Hard blocker for the Claude archive:** the walker assumes a `.git/`
directory. `~/.claude/projects/<project>/*.jsonl` has no git layer.
The plan creates a sibling crate `cortex-claude-archive` that
mirrors the bootstrap loop without git.

## 5. Archive loader (boot-time keyword seed)

[`crates/cortex-api/src/archive_loader.rs:80-300`](../../../crates/cortex-api/src/archive_loader.rs#L80-L300)
walks `<CORTEX_ARCHIVE_ROOT>/events/year=…/…/raw-NNNNN.parquet`
(zstd-NDJSON despite the extension), parses each line as a canonical
`Envelope`, and seeds `MemoryKeywordLane` so `/v1/query` returns
results before the live workers wire up.

**Reuse window for this plan:** if the new `cortex-claude-archive`
crate writes to `<CORTEX_ARCHIVE_ROOT>/events/year=…/…/bootstrap-claude-NNNNN.parquet`
in the same zstd-NDJSON format, it gets seeded for free at
cortex-api boot. We adopt this path: it doubles as a debugging
artifact and survives Synap downtime.

## 6. Query API + RRF fusion

`/v1/query` resolves intent → strategy
([spec 11](../../specs/11-query-api.md), §Intent → strategy):

| Intent | Vector lane | Keyword lane | Graph lane |
|---|---|---|---|
| `pre_change_context` | `cortex-{slug}-{code,docs}` | per-repo + decisions | artifact-touched neighbours |
| `decision_lookup` | `cortex.decision.fp32` | `cortex_decisions` | supersession chain |
| `similar_problems` | `cortex.turn.fp32 / .pq` | `cortex_turns` | turn → analysis → decision |
| `law_check` | — | `cortex_laws` | law → violation → turn |
| `free_search` | per-repo `code` | per-repo `code` | — |

RRF fusion lives in `cortex-api/src/orchestrator.rs` and
`fusion.rs`. Today the per-lane weights are hard-coded equal; phase
11i §3 (recency / scope / outcome boosts) tunes them.

## 7. Pre-thinking budget

[`crates/cortex-pre-thinking/src/lib.rs`](../../../crates/cortex-pre-thinking/src/lib.rs)
runs after `/v1/query` and renders the deterministic markdown
bundle injected into the agent's `UserPromptSubmit` hook context.
Default budget 32 KiB, sections are tail-clipped in priority order
(graph_neighbors → similar_turns → violations → decisions →
snippets), enforced by the `phase11c` clipper at the API layer
([`cortex-api/src/budget.rs`](../../../crates/cortex-api/src/budget.rs)).

The clipper means we can dump millions of historical turns into
Vectorizer/Meili without flooding the prompt — it's bounded at the
exit. Whatever the lanes return, the pre-thinking bundle fits.

## 8. Extension points used by phase 11i

| Layer | File | Change required |
|---|---|---|
| Source | `crates/cortex-claude-archive/` (new crate) | walker + JSONL → Envelope converter |
| Schema | `crates/cortex-core/src/events.rs` | none for v1 (v2 may add Kind::SessionMeta if telemetry warrants) |
| Classifier | `crates/cortex-workers/src/classifier/kinds.rs:19` | add bootstrap kind strings: `"turn.claude-code"`, `"tool_call.claude-code"`, `"agent_call.claude-code"` |
| Family routing | `crates/cortex-workers/src/fulltext/routing.rs:37` | no change (existing kinds → existing families) |
| Embedder routing | `crates/cortex-workers/src/embedder/routing.rs:18` | no change |
| Archive loader | `crates/cortex-api/src/archive_loader.rs:235` | no change (already handles Turn / ToolCall / AgentCall) |
| Query strategy | `crates/cortex-api/src/strategies.rs` | new lanes: `cortex.claude-archive.turns.fp32`, `cortex_claude_archive_turns` for `similar_problems` + `pre_change_context` |
| Scope filters | `crates/cortex-core/src/types.rs` (Scope struct) | add `session_id`, `model`, `since_decay` fields |
| RRF fusion | `crates/cortex-api/src/fusion.rs` | per-lane recency decay; same-session boost |
| Pre-thinking | `crates/cortex-pre-thinking/src/lib.rs` | new section "Past sessions" rendering top-N similar turns |
| Bootstrap CLI | `cortex.toml` schema in `cortex-bootstrap` | add `[cortex.archive_sources.claude_code]` block |

The minimum-viable cut (phase 1 of the implementation plan) only
touches the new crate plus the classifier-kind mapping — the rest
of the pipeline auto-absorbs the new envelopes.

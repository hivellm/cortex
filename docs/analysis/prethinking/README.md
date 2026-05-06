# Cortex Pre-Thinking Analysis — Executive Summary

> **Analysis ID:** PRE-001
> **Date:** 2026-05-05
> **Scope:** Complete system analysis of how Cortex can assist with LLM pre-thinking
> **Status:** Complete

---

## 1. Executive Summary

Cortex is the **cognitive substrate of the HiveLLM ecosystem** — an orchestrator that captures, classifies, embeds, indexes, and retrieves every meaningful AI interaction across all codebases. Its pre-thinking subsystem (`cortex-pre-thinking`) is the mechanism through which Cortex transforms a "blind" LLM session into an **analytically grounded** one by injecting institutional memory **before** the model produces its first token.

The pre-thinking pipeline operates in **5 stages** (scope derivation → intent selection → hybrid query → deterministic formatting → budget clipping) and surfaces 10 distinct context bands from 3 retrieval lanes (vector, keyword, graph) fused via Reciprocal Rank Fusion. The entire pipeline is **fail-open** — every error path returns empty context rather than breaking the session.

**Key finding:** Cortex pre-thinking addresses 7 distinct cognitive gaps that occur when LLMs operate without institutional memory. The system is already implemented and operational across 10 crates, with the pre-thinking module at `crates/cortex-pre-thinking/` being the central entry point.

---

## 2. How Cortex Pre-Thinking Assists LLMs

| Cognitive Gap | How Cortex Addresses It | Mechanism |
|---|---|---|
| **No memory of prior decisions** | Surfaces accepted/superseded ADRs and their rationale | `decision_lookup` intent → decisions overlay |
| **Unaware of active governance rules** | Injects active laws (never trimmed, load-bearing) | Laws section in bundle; blocking on PreToolUse |
| **Cannot recognize recurring problems** | Matches past similar turns by semantic similarity | `similar_problems` intent → vector search on turns |
| **Lacks file-level context awareness** | Derives scope from cwd + git status + prompt-extracted paths | `scope_derive()` pure function |
| **No understanding of code relationships** | Expands graph 1-2 hops from seed artifacts | `IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES` relations |
| **Cannot benefit from past mistakes** | Consolidates raw sessions into evergreen summaries | Consolidations + Topic Cards (living synthesis) |
| **No cross-session learning transfer** | Topical synthesis rewritten in place as evidence accumulates | Topic cards with contradiction detection |

---

## 3. Architecture of Pre-Thinking

```
user_prompt + cwd + recent_files
        │
        ▼
  scope_derive()  ──────▶  intent_select()  ──────▶  QueryRequest
        │                                                  │
        │                                           hybrid query (vector+keyword+graph)
        │                                           fused via RRF (alpha=0.7, k=60)
        │                                                  │
        │                                                  ▼
        └────────────────────── bundle_format() ◀──── QueryResponse
                                        │
                                        ▼
                              clip_to_budget() (6-step trim ladder)
                                        │
                                        ▼
                              additionalContext (≤ 32 KB Markdown)
```

### Intent Routing (6 intents)

| Intent | Trigger keywords | Retrieval strategy |
|---|---|---|
| `explain` | "how does", "what is", "explain", "show me", "where is" | vector+keyword on code+docs, no policy overlays |
| `decision_lookup` | "why did we pick", "why do we use", "history of" | decisions collection search + supersession graph |
| `similar_problems` | "have we seen", "stuck", "keep failing", "doesn't work" | vector on turns + analysis-decision graph |
| `law_check` | "is this allowed", "would this violate", "blocked" | keyword on governance + violations overlay |
| `pre_change_context` | "refactor", "modify", "rewrite", "change", "edit" | 3-lane fan-out + full overlay set (default) |
| `free_search` | (fallback / explicit) | generic vector+keyword search |

### Context Bands (rendered in fixed order)

1. **Laws** (load-bearing, never dropped) — capped at 10
2. **Topic Cards** (living synthesis) — top-priority when fresh; 1 card, 1400 bytes
3. **Consolidated Context** (evergreen session/topic summaries) — capped at 3; replaces past sessions
4. **Decisions** (ADRs with outcome glyphs) — capped at 5
5. **Similar Past Turns** (vector-matched conversations) — capped at 5
6. **Past Sessions** (fallback when no consolidations) — capped at 3
7. **Relevant Snippets** (code/doc chunks with path:symbol headers) — capped at 5
8. **Connected Files** (IMPORTS_FILE graph edges)
9. **Documented Under** (DOCUMENTED_BY graph edges)
10. **Cited From** (CITES graph edges)

---

## 4. Key Design Decisions

1. **Rules, not models, for intent selection** — 55 keyword rules in a precedence-ordered table; fast, deterministic, debuggable
2. **Laws are load-bearing, never trimmed** — the 6-step trim ladder sacrifices everything else first
3. **Fixed section order, no LLM prose** — deterministic Rust string assembly; byte-identical across runs
4. **Empty result = empty bundle** — silence is more honest than "No relevant context found"
5. **Fail-open everywhere** — timeout/error/panic → empty string; session never breaks
6. **Topic cards rewrite in place** — 3 contradiction detectors surface conflicting evidence
7. **Consolidations replace raw sessions** — higher-fidelity summaries suppress the legacy past-sessions block

---

## 5. Observability

- **6 counters** tracking calls, empty bundles, timeouts, truncation steps
- **3 histograms** on bundle bytes, section counts, latency
- **Audit envelopes** carrying intent, intent_trigger, query_id, scope_hash
- **Structured tracing** with session_id, turn_id, intent, sections summary
- **Health endpoints** (`/healthz`, `/v1/health`, `/v1/health/freshness`, `/v1/health/divergence`)
- **CI smoke gate** running synthetic canary through real IPC on every PR

---

## 6. Implementation Status

| Component | Status | Crate |
|---|---|---|
| Event schema (12 kinds) | Implemented | `cortex-core` |
| Storage layout (3-tier) | Implemented | `cortex-storage` |
| Classification (Haiku) | Implemented | `cortex-workers` |
| Embedding (Vectorizer) | Implemented | `cortex-workers` |
| Graph writing (Nexus) | Implemented | `cortex-workers` |
| Full-text indexing (Meilisearch) | Implemented | `cortex-workers` |
| Query API (hybrid + RRF) | Implemented | `cortex-api` |
| **Pre-thinking injection** | **Implemented** | **`cortex-pre-thinking`** |
| Claude Code adapter | Implemented | `cortex-adapter-claude-code` |
| MCP server | Implemented | `cortex-mcp-server` |
| Bootstrap CLI | Implemented | `cortex-cli` |
| Health system (11 phases) | Implemented | `cortex-health` |
| Laws DSL | Drafted | — |
| Governance engine | Drafted | — |
| Deep analysis | Drafted | — |
| Dashboard | Drafted | — |

---

## 7. References

- [Complete Findings](findings.md) — 16 numbered findings with evidence
- [Execution Plan](execution-plan.md) — phased plan to maximize pre-thinking impact
- Spec 12: `docs/specs/12-pre-thinking-injection.md`
- Architecture: `docs/architecture.md`
- PRD: `docs/prd.md`
- Source: `crates/cortex-pre-thinking/src/`

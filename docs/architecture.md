# Cortex — Architecture

> **Status:** Draft v0.1 — initial architecture proposal
> **Owners:** HiveLLM core team
> **Last updated:** 2026-04-17

---

## 1. Vision

Cortex is the **cognitive substrate of the HiveLLM ecosystem**: a single, queryable, governed memory of every meaningful interaction an AI agent has with our codebases. Today, when an LLM (Claude, Cursor, Gemini, Codex, Copilot) works on one of our projects, three things are lost the moment the session ends:

1. **What happened** — the conversation, the tool calls, the agent invocations, the rationale.
2. **What was decided** — and why one path was chosen over another.
3. **What was learned** — patterns that worked, dead ends, recurring bugs, conventions that emerged.

Each new session starts blind. The model falls back to its weights and a few CLAUDE.md files. We've been losing institutional knowledge at every `/clear`.

**Cortex changes the contract.** Every interaction is captured, classified by a small local model, embedded, indexed, related, and made retrievable. Before any model proposes a change, it consults Cortex. Decisions are formalized and audited. Laws govern behavior, and violations are tracked. A dashboard makes the whole substrate observable.

The end state: an AI workflow that is **analytical rather than purely generative** — grounded in our own history, not just the model's training data.

---

## 2. Goals & Non-Goals

### Goals

- **G1.** Capture 100% of AI interactions (conversation turns, tool calls, agent calls, memory ops, decisions) across every supported AI tool.
- **G2.** Classify, embed, and relate captured artifacts using local models — never ship payloads to third parties for indexing.
- **G3.** Provide a single retrieval API that serves hybrid (semantic + keyword + graph) queries with sub-100ms latency for the hot path.
- **G4.** Force consultation of Cortex before any non-trivial code change via hooks (PreToolUse / pre-commit) integrated through Rulebook.
- **G5.** Codify, enforce, and audit a set of development laws; track violations and apply graduated responses ("punishment").
- **G6.** Expose everything through an operator dashboard: timelines, decision trails, tool-usage analytics, law-violation reports.
- **G7.** Reuse existing HiveLLM components (Vectorizer, Nexus, Synap, Rulebook) wherever possible — Cortex is the orchestrator, not a re-implementation. Use external services where no production-ready Hive component exists (Meilisearch for full-text; **Claude Haiku via Claude Code CLI** for classification — see §5.2.1).

### Non-Goals (for v1)

- **NG1.** Cortex is **not** a new vector DB, graph DB, or search engine. It composes the existing Hive services.
- **NG2.** Cortex is **not** a coding agent. It does not generate code; it informs the agents that do.
- **NG3.** Cortex does **not** replace per-tool memory (Claude Code's `memory/`, Cursor rules, etc.). It complements and federates them.
- **NG4.** v1 does **not** target multi-tenant SaaS. Single-org deployment first; HivehubCloud integration comes later.

---

## 3. Ecosystem Context

Cortex sits **above** the data services and **below** the AI tools:

```
┌──────────────────────────────────────────────────────────────────┐
│  AI Tools / IDEs                                                 │
│  Claude Code · Cursor · Gemini · Codex · Copilot · Windsurf      │
└──────────────────────────────────────────────────────────────────┘
                       ▲                       ▲
                       │ capture (hooks)       │ retrieve (MCP)
                       ▼                       │
┌──────────────────────────────────────────────────────────────────┐
│                          C O R T E X                             │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐  │
│  │ Ingestion  │→ │ Processing │→ │ Retrieval  │  │ Governance │  │
│  │  (events)  │  │ (classify, │  │ (hybrid    │  │ (laws,     │  │
│  │            │  │  embed)    │  │  search)   │  │  audit)    │  │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘  │
│  ┌──────────────────────────────────────────────────────────────┐│
│  │ Dashboard · Analytics · Decision Reports · Deep Analysis     ││
│  └──────────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────────┘
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│  HiveLLM Data Services (already built — Cortex composes them)    │
│  ┌──────────┐  ┌────────┐  ┌──────┐  ┌──────┐  ┌────────┐        │
│  │Vectorizer│  │ Nexus  │  │Synap │  │ Meili│  │ Haiku  │        │
│  │ (vector) │  │(graph) │  │(K/V, │  │search│  │(via    │        │
│  │          │  │        │  │ pub) │  │(full │  │ CC CLI)│        │
│  │          │  │        │  │      │  │ text)│  │        │        │
│  └──────────┘  └────────┘  └──────┘  └──────┘  └────────┘        │
└──────────────────────────────────────────────────────────────────┘
```

| Service        | Role in Cortex                                                                            |
|----------------|-------------------------------------------------------------------------------------------|
| **Vectorizer** | Stores embeddings for every artifact; HNSW search; primary semantic retrieval            |
| **Nexus**      | Stores relationships (`Conversation→ToolCall→File→Decision→LawViolation`); Cypher        |
| **Synap**      | Hot cache, event streams (ingestion bus), pub/sub for live dashboard                     |
| **Meilisearch**| Full-text inverted index for keyword/BM25 search across raw text. Used as a stand-in until **Lexum** reaches production parity, at which point we'll migrate. |
| **Expert**     | *Not used in v1.* Reserved as a **future option** if/when Haiku usage costs or latency justify migrating to a local model. See §5.2.1. |
| **Rulebook**   | Provides hook contracts, law DSL, and federation with per-tool memory                    |

Cortex itself contributes: the **event schema**, the **ingestion router**, the **classification pipeline**, the **query orchestrator**, the **governance engine**, and the **dashboard**.

---

## 4. Conceptual Data Model

Everything Cortex captures is an **Event**. Events become **Artifacts** after enrichment. Artifacts have **Embeddings**, are stored in the **Graph**, and link to one another via typed **Relations**.

### 4.1 Core entity types

| Entity            | Description                                                           |
|-------------------|-----------------------------------------------------------------------|
| `Session`         | One AI session (conversation, IDE session, agent invocation)         |
| `Turn`            | A single exchange within a Session (user prompt → assistant response) |
| `ToolCall`        | An invocation of a tool (Bash, Edit, Read, MCP tool, etc.)            |
| `AgentCall`       | An invocation of a sub-agent (Task tool, code-reviewer, etc.)         |
| `Memory`          | A persisted memory entry (user, feedback, project, reference)         |
| `Decision`        | A formalized decision record (ADR-style, often output of a deep analysis) |
| `Analysis`        | A deep-analysis report on a complex topic                             |
| `Law`             | A development rule that must be followed                              |
| `LawViolation`    | An observed breach of a Law                                           |
| `Artifact`        | A file, diff, snippet, or external resource referenced by other entities |
| `Topic`           | A classifier-assigned theme (e.g., "auth", "graph-traversal", "ci")  |
| `Entity`          | An NER-extracted concept (function name, repo, person, package)       |

### 4.2 Relation types (Nexus edges)

```
(Session)-[:CONTAINS]->(Turn)
(Turn)-[:INVOKED]->(ToolCall|AgentCall)
(ToolCall)-[:TOUCHED]->(Artifact)
(ToolCall)-[:READ|WROTE|EXECUTED]->(Artifact)
(Turn)-[:PRODUCED]->(Memory|Decision|Analysis)
(Decision)-[:SUPERSEDES]->(Decision)
(Decision)-[:REFERENCES]->(Analysis|Memory|Artifact)
(*)-[:ABOUT]->(Topic)
(*)-[:MENTIONS]->(Entity)
(LawViolation)-[:OF]->(Law)
(LawViolation)-[:OBSERVED_IN]->(Turn|ToolCall)
(*)-[:SIMILAR_TO {score}]->(*)   // derived via Vectorizer KNN
```

### 4.3 Event envelope (ingestion wire format)

```jsonc
{
  "event_id": "01HXYZ...",            // ULID
  "occurred_at": "2026-04-17T12:34:56.789Z",
  "session_id": "...",
  "tool": "claude-code",              // or cursor, gemini, codex, ...
  "model": "claude-opus-4-7",
  "kind": "tool_call",                // turn | tool_call | agent_call | memory | decision | analysis | law_violation
  "payload": { /* kind-specific */ },
  "context": {                        // captured automatically by adapter
    "repo": "e:/HiveLLM/Cortex",
    "branch": "main",
    "commit": "abc123",
    "cwd": "...",
    "user": "andre@hivellm"
  },
  "redactions": ["secret:.env", "secret:api_key"],
  "schema_version": "1"
}
```

---

## 5. Layered Architecture

### 5.1 Layer A — Capture (Ingestion adapters)

The capture layer turns whatever an AI tool emits into normalized Cortex events.

| Adapter           | Mechanism                                                            |
|-------------------|----------------------------------------------------------------------|
| **Claude Code**   | Hooks: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStop`, `Stop`, `Notification`. Hooks POST to local Cortex daemon. |
| **Cursor**        | Cursor `rules` + a thin MCP server registered in `.cursor/mcp.json` that mirrors tool calls. |
| **Codex / Gemini**| Generic logging proxy (HTTP middleware) wrapping their tool-use API. |
| **Generic MCP**   | Cortex exposes itself as an MCP server; any MCP client gets capture for free. |

All adapters publish to the **ingestion bus**: a Synap stream (`cortex.events.raw`).

**Privacy gate at capture time.** A redactor pass strips secrets (API keys, `.env` contents, tokens) before the event leaves the user's machine. Pattern catalog is shared across adapters and versioned.

### 5.2 Layer B — Processing (Classify, embed, link)

A pool of stateless workers consumes `cortex.events.raw` and runs each event through a pipeline:

```
raw event ──▶ normalize ──▶ classify (Haiku) ──▶ extract entities
                                                       │
                                                       ▼
                            embed (Vectorizer) ◀── chunk + summarize
                                       │
                                       ▼
                       persist to Vectorizer + Nexus + Meilisearch
                                       │
                                       ▼
                  publish enriched event to cortex.events.enriched
```

#### 5.2.1 Classifier (Claude Haiku via Claude Code CLI)

**v1 decision: no local model, no training.** We use **Claude Haiku 4.5** invoked through the **Claude Code CLI** in headless mode. Rationale: we already have a Claude Code subscription with ample Haiku quota; this eliminates training pipelines, GPU dependency, model-serving infrastructure, and the entire `cortex-classifier` Python codebase from v1. We iterate on the *prompt*, not on adapter weights.

**Invocation (default — CLI):**

```bash
claude -p "$PROMPT" --model claude-haiku-4-5 --output-format json
```

**Invocation (optimization — direct SDK):** for high-throughput bootstrap and live batches, workers can call the Anthropic SDK directly (same model, same JSON contract). The CLI overhead (~300 ms per invocation) is amortized away. Both paths share the exact same prompt template — they're interchangeable per worker config.

**Output schema (single event or batch):**

```jsonc
{
  "events": [
    {
      "id": "01HXYZ...",
      "kind_refinement": "git_push",         // refines coarse `kind` from envelope
      "topics": ["git", "deployment"],       // multi-label, ~200-term controlled vocab
      "severity": "notable",                 // info | notable | critical
      "pii_risk": "low",                     // low | medium | high
      "redaction_suggestions": [],           // patterns the static redactor missed
      "summary": "..."                       // 1-2 sentences, used as embed input if payload >4KB
    }
  ]
}
```

**Batching is mandatory for live throughput.** A single Haiku call classifies up to **N=32 events** by passing them as a JSON array in the prompt. At ~1.5 s per call, a single worker handles ~20 events/sec; a pool of 25 workers easily covers the 500 eps target. Batches are formed from the Synap stream within a 200 ms time window or when N=32 fills, whichever comes first.

**Caching.** Classifier results keyed by `content_hash` of the input. Identical inputs (common in code bootstrap — same imports, same boilerplate) hit cache and skip the API call.

**Cost ceiling and fallback.** A budget tracker in `cortex-workers` watches daily spend; on threshold breach it (a) drops `severity` and `redaction_suggestions` from the prompt to shorten output tokens, (b) raises batch size, (c) falls back to a static rule-based classifier (kind→topic mapping table) for low-priority events. Threshold and behavior are configurable per deployment.

**Privacy boundary.** Event payloads sent to Haiku traverse the network to Anthropic. The static redactor (§5.1) runs *before* the classifier, so secrets are stripped at the edge. Deployments with stricter requirements can flip a switch to route classification to Expert (when ready) or to a self-hosted Haiku alternative — the worker interface is a single trait, swap is mechanical.

**Embedding.** Vectorizer's BM25 + dense (FastEmbed / MiniLM) hybrid pipeline. Per-`kind` collections so we can tune HNSW params separately. Chunks > 4 KB are summarized first by the Haiku classifier (the `summary` field, §5.2.1) before embedding.

**Linking (Nexus).** Workers write nodes and edges as defined in §4.2. Heavy traversal queries (e.g., "find all decisions that touched this file") become Cypher queries against Nexus.

### 5.3 Layer C — Retrieval (Query orchestrator)

A single query API serves three audiences: AI agents (pre-thinking enrichment), the dashboard, and the analysis engine.

```
POST /api/query
{
  "intent": "pre_change_context" | "decision_lookup" | "similar_problems"
          | "law_check" | "free_search",
  "scope": { "repo": "...", "files": [...], "topics": [...] },
  "query": "user prompt or natural-language question",
  "limit": 20,
  "k": 50,
  "include": ["snippets", "decisions", "violations", "graph_neighbors"]
}
```

The orchestrator runs in parallel:
1. **Vector search** (Vectorizer KNN, top-k=k, multi-collection)
2. **Keyword search** (Meilisearch — typo-tolerant BM25)
3. **Graph expansion** (Nexus Cypher: from seed nodes, expand 1–2 hops along chosen relation types)
4. **Decision/law overlay** (annotate results with linked decisions and active laws)

Results are fused with **Reciprocal Rank Fusion** (RRF) and returned as a structured context bundle the agent can drop straight into its system prompt.

**Hot path target:** P50 < 50 ms, P95 < 150 ms (cached); cold path < 500 ms. A Synap-backed result cache with semantic-key hashing (embedding of the query) absorbs repeated lookups.

### 5.4 Layer D — Governance (Laws & enforcement)

Laws are versioned Markdown files with YAML frontmatter, stored under `laws/` in Cortex and indexed like any other artifact:

```yaml
---
id: LAW-007
title: Never bypass pre-commit hooks
severity: critical
applies_to: ["git", "commit"]
detector: hook:pre_commit_no_skip
remediation: "Fix the hook failure; do not pass --no-verify."
introduced: 2026-04-17
supersedes: null
---
The model MUST NOT pass `--no-verify` to git commit unless the user has
explicitly authorized it in this session.
```

**Detector contract.** Each Law declares a *detector* — a small program (TypeScript, evaluated in a sandbox) that inspects an event and returns `{ violated: bool, evidence: ... }`. Detectors run synchronously in the `PreToolUse` hook for **blocking laws** (severity=critical) and asynchronously after `PostToolUse` for **observational laws**.

**Punishment ladder** (configurable per law):
| Tier | Action                                                                  |
|------|-------------------------------------------------------------------------|
| 1    | Annotation in dashboard; no agent feedback                              |
| 2    | Inject a reminder into the next system prompt of the offending model    |
| 3    | Block the offending tool call; require human override                   |
| 4    | Down-weight the model in the Cortex router (prefer other models for that scope) |

A **trust score** per `(model, repo)` is recomputed nightly from violations and decision-following accuracy. The score is exposed via the dashboard and consumed by Rulebook to influence model selection in HivehubCloud routing.

### 5.5 Layer E — Deep Analysis

For "stuck" topics (recurring bug, contested architecture choice, unfamiliar domain), the user (or an agent) launches an **Analysis**:

```
cortex analysis start "Why does our HNSW recall drop above 1M vectors?"
```

This spawns a workflow:
1. Cortex retrieves all relevant historical context (turns, decisions, similar incidents).
2. A panel of agents (configurable: 2–5 models) debate the question with the context as ground truth.
3. Each round is captured as Turns linked to the Analysis node.
4. A judge agent (or human) finalizes a **Decision** record.
5. The Decision is indexed and becomes citable from future queries.

Analyses are first-class citizens: searchable, supersedable, and used by `intent: similar_problems`.

### 5.6 Layer F — Dashboard

Web app (React + TS, reusing the Vectorizer dashboard scaffold) talking to a Cortex-API backend (Rust, Axum). Views:

- **Live timeline** — sessions, turns, tool calls in real time (Synap pub/sub via SSE).
- **Memory browser** — searchable, faceted by model/repo/topic/severity.
- **Decision register** — ADR-style list, supersession graph view.
- **Law dashboard** — active laws, violation rates, trust scores per model.
- **Analysis library** — completed deep analyses with the debate transcript.
- **Tool analytics** — heatmap of tool usage, slow tools, failed tools, cost.
- **Graph explorer** — embedded Nexus graph view for arbitrary Cypher queries.

---

## 6. Bootstrap: Indexing the existing HiveLLM corpus

Cortex starts cold. The capture layer (§5.1) only sees *new* AI interactions — but our institutional knowledge is already scattered across **~17 existing repos** in `e:/HiveLLM/` (Vectorizer, Nexus, Synap, Lexum, Expert, Rulebook, HivehubCloud, Tml, Transmutation, Umicp, Synap, Assets, etc.). Before Phase 1 of the roadmap is useful, we must **backfill** that corpus so the very first pre-thinking query has something to retrieve.

This bootstrap is a **one-time + incremental** ingestion job, separate from the live capture pipeline but sharing the same processing layer (§5.2) and storage backends.

### 6.1 What gets indexed

For each repo under `e:/HiveLLM/`:

| Source                                | Treated as              | Notes                                                    |
|---------------------------------------|-------------------------|----------------------------------------------------------|
| Source code (`*.rs`, `*.ts`, `*.py`, etc.) | `Artifact:code`     | Chunked by symbol (function/struct/class) when possible  |
| Docs (`docs/`, `*.md`, `README.md`, `CHANGELOG.md`) | `Artifact:doc` | Section-level chunking                                   |
| Specs / OpenSpec (`openspec/`, `proposal.md`, `tasks.md`) | `Decision` | Treated as historical decisions                          |
| ADRs / design docs                    | `Decision`              | Promoted to first-class Decision nodes                   |
| `CLAUDE.md`, `AGENTS.md`              | `Memory:project`        | Imported as project memories                             |
| Git history (commit messages, PR descriptions) | `Turn:historical` | Each commit ≈ one historical "turn" with diff as evidence |
| Issues / PRs (via `gh` CLI when available) | `Turn:historical` | Linked to the commits/files they touched                 |
| Existing rule / law files (`rulebook/`, `.cursor/rules/`) | `Law:imported` | Seeds the law registry                                   |

### 6.2 Pipeline

```
                       ┌─────────────────────────┐
                       │  cortex-bootstrap CLI    │
                       │  (Rust, parallel walker) │
                       └────────────┬────────────┘
                                    ▼
        ┌───────────────────────────────────────────────────┐
        │ Per-repo discovery: git log, file walk, doc parse │
        │ → emits "synthetic events" with same envelope §4.3│
        └────────────┬──────────────────────────────────────┘
                     ▼
              cortex.events.bootstrap        (Synap stream, separate channel)
                     │
                     ▼
       ┌────────────────────────────┐
       │ Same processing workers as │   (classify · chunk · embed · link · persist)
       │ live ingestion (§5.2)      │
       └────────────────────────────┘
```

**Key decisions:**

1. **Same workers, different stream.** Bootstrap reuses the live processing pipeline; we just route to a separate Synap stream so we can throttle/pause it without affecting live capture.
2. **Symbol-level chunking for code.** Use Tree-sitter to chunk by function/struct/class instead of fixed-size windows. Improves retrieval precision dramatically. Tree-sitter grammars per language; fall back to 512-token windows for unsupported langs.
3. **Git as a free historical signal.** Every commit is a `Turn:historical` event with the message as the prompt-equivalent and the diff as the tool-call-equivalent. PR titles/bodies fill in rationale. This gives us *years* of "free" context for retrieval.
4. **Decisions are gold.** Anything that smells like a decision (ADRs, OpenSpec proposals, RFC-tagged docs) is upgraded to a `Decision` node. These get extra weight in retrieval.
5. **Idempotent.** Each artifact gets a stable `(repo, path, content_hash)` identity so re-running bootstrap is a no-op for unchanged content. Modified files are re-indexed; deleted files are tombstoned.

### 6.3 Repo-by-repo plan

Initial target list (in priority order — pick highest-traffic first to validate retrieval quality):

1. `Vectorizer` — has rich docs, tests, multi-year history; best candidate to validate code-chunking
2. `Nexus` — graph-heavy domain; validates that we link function calls / Cypher procedures correctly
3. `Rulebook` — already has a notion of memory + laws; great seed for the Law registry
4. `Synap`, `Expert`, `HivehubCloud`, `Transmutation`, `Umicp`, `Tml` — broaden coverage
5. `Lexum`, `VectorizerSync`, `TransmutationLite`, `Assets`, `CompressionPrompt`, `TmlDocs`, `TmlTextmate` — fill out the rest

Each repo gets a `cortex.toml` that overrides defaults (excluded paths, chunking strategy, sensitive patterns).

### 6.4 Ongoing sync after bootstrap

Once a repo is bootstrapped, Cortex tails it:

- A **filesystem watcher** (notify on Linux/Mac, ReadDirectoryChangesW on Windows) per registered repo emits change events.
- A **git hook** (`post-commit`, `post-merge`) batches commit-level updates.
- For repos hosted on GitHub, a thin webhook-receiver service can push PR/issue updates directly into the bootstrap stream.

This means after the initial backfill, the corpus stays current automatically — every new commit, doc change, or merged PR appears in retrieval results within seconds.

### 6.5 Cost & sizing (estimate)

Rough back-of-envelope for the 17 HiveLLM repos:

| Metric                          | Estimate           |
|---------------------------------|--------------------|
| Total source files              | ~30 000            |
| Total commits                   | ~15 000            |
| Code chunks after Tree-sitter   | ~150 000           |
| Doc chunks                      | ~25 000            |
| Embedding storage (768-dim)     | ~600 MB            |
| Nexus nodes/edges               | ~1.2 M nodes / 3 M edges |
| Meilisearch index size          | ~1.5 GB            |
| One-time bootstrap runtime      | 4–8 hours single node, parallelizable |

Numbers are loose — the bootstrap CLI emits a `--dry-run --estimate` mode for real numbers before committing to the full run.

### 6.6 Where this fits in the roadmap

- **Phase 0:** ship `cortex-bootstrap` skeleton with a single repo (Vectorizer) end-to-end.
- **Phase 1:** finish the bootstrap of all 17 repos *before* opening the live capture firehose, so day-1 retrieval is already useful.
- **Phase 2+:** filesystem + git-hook tailing; webhook receiver if/when repos move to GitHub.

---

## 7. Component Boundaries (what we build vs. what we use)

### What Cortex itself contains (this repo)

- `cortex-core/` — event schemas, redactor, the ingestion router (Rust)
- `cortex-workers/` — async processing pipeline (Rust + tokio); calls Haiku/Vectorizer/Nexus/Meilisearch
- `cortex-api/` — HTTP + MCP server exposing query, analysis, governance APIs (Rust + Axum)
- `cortex-classifier/` — Haiku invocation client (CLI + SDK paths), prompt templates, batcher, content-hash cache, budget tracker, static fallback (Rust)
- `cortex-laws/` — laws DSL parser, detector sandbox, enforcement logic
- `cortex-adapters/` — capture adapters for Claude Code, Cursor, etc.
- `cortex-dashboard/` — React + TypeScript SPA
- `cortex-cli/` — operator CLI (`cortex analysis start`, `cortex laws lint`, `cortex doctor`)
- `docs/` — this document and downstream specs

### What Cortex does NOT reimplement

- ❌ A vector index (use **Vectorizer**)
- ❌ A graph DB (use **Nexus**)
- ❌ A pub/sub or stream broker (use **Synap**)
- ❌ A full-text engine (use **Meilisearch** today; migrate to **Lexum** when it's production-ready)
- ❌ A small-model serving stack (v1: call **Haiku via Claude Code CLI**; future: swap to **Expert** if costs/latency demand it)
- ❌ Per-tool rule generation, hooks scaffolding, basic memory store (use **Rulebook**)

If we're tempted to build any of the above, that's a smell — re-check the Hive service first.

---

## 8. Data Flow (end-to-end example)

User runs Claude Code; asks: *"Refactor the HNSW configurator."*

1. **Capture.** Claude Code's `UserPromptSubmit` hook POSTs the prompt to local Cortex daemon → published to `cortex.events.raw`.
2. **Pre-thinking enrichment.** Daemon synchronously calls `POST /api/query` with `intent: pre_change_context`, scope = `{repo: Vectorizer, files: ["src/index/hnsw/*"]}`. Cortex returns: 3 prior Decisions, 2 past Analyses on HNSW recall, 7 similar tool-call sequences, 1 active Law ("LAW-012: HNSW recall benchmarks must run before merge"). The daemon injects this bundle into the model's system prompt.
3. **Tool calls.** Each `PreToolUse`/`PostToolUse` hook fires → events published.
4. **Law check.** `PreToolUse` evaluates blocking detectors — e.g., LAW-007 blocks a `git commit --no-verify`.
5. **Processing.** Workers batch events, classify them via Haiku (CLI or SDK), embed snippets (Vectorizer), write nodes/edges (Nexus), index text (Meilisearch).
6. **Stop hook.** Session summary is generated by the local model and stored as a `Decision` candidate (user can promote to formal Decision).
7. **Dashboard.** All events stream live via Synap pub/sub → SSE to dashboard.
8. **Future session.** Next time anyone (or any model) touches HNSW code, step 2 returns the new context too.

---

## 9. Privacy, Security, Retention

- **Local-first.** All capture, redaction, classification, and indexing run on user infra. No third-party calls during ingestion.
- **Redaction at the edge.** Static patterns applied before the event leaves the originating machine. Haiku classifier may suggest *additional* redactions on payloads it sees, but never sees raw secrets that the static redactor caught.
- **Retention tiers** keyed off `pii_risk`:
  - `low` → indefinite
  - `medium` → 365 days (re-summarized + raw discarded after 90)
  - `high` → 30 days raw, embedding kept indefinitely (one-way)
- **Access control.** API requires JWT or API key; per-collection RBAC delegated to Vectorizer/Nexus.
- **Right to forget.** `cortex forget --session <id>` cascades to all four data services and updates the graph.

---

## 10. Performance Targets

| Path                                | Target P50 | Target P95 |
|-------------------------------------|-----------:|-----------:|
| Event ingest → ack                  |      < 5ms |    < 20 ms |
| Pre-thinking query (cached)         |     < 50ms |   < 150 ms |
| Pre-thinking query (cold)           |    < 250ms |   < 500 ms |
| Classifier (Haiku, batch of 32)     |    < 1.5 s |    < 3.0 s |
| Classifier (cached hit, content_hash)|    < 5 ms |    < 15 ms |
| Embed + persist (per event)         |    < 200ms |   < 500 ms |
| Dashboard SSE end-to-end latency    |    < 200ms |   < 500 ms |

Throughput target v1: **500 events/sec sustained**, **2 000 events/sec burst**, single node.

---

## 11. Phased Roadmap

### Phase 0 — Foundations (2 weeks)
- Lock event schema (§4.3) and law schema (§5.4).
- Stand up local stack: Vectorizer + Nexus + Synap + Meilisearch in `docker-compose.yml`.
- Skeleton `cortex-core` + `cortex-api` + `cortex-bootstrap` (Rust workspace).
- Manual ingestion via CLI, end-to-end smoke test (event → all four backends).
- Single-repo bootstrap proof of concept (Vectorizer): code + docs + git history → retrievable.

### Phase 1 — Bootstrap + Claude Code adapter + basic retrieval (4 weeks)
- **Bootstrap (§6)** of all 17 HiveLLM repos so day-1 retrieval is meaningful.
- Tree-sitter symbol-level chunking for the top 5 languages we use (Rust, TS, Python, Go, JS).
- Hook scripts for Claude Code; daemon receives events.
- Static redactor with pattern catalog v1.
- Hybrid query API (vector + keyword via Meilisearch), no graph yet.
- Pre-thinking injection working in one repo (Vectorizer itself), backed by the bootstrapped corpus.

### Phase 2 — Classifier + graph (4 weeks)
- Tune Haiku classifier prompt against the corpus collected in Phase 1; lock controlled vocabulary (~200 topics).
- Workers populate Nexus relations.
- Graph expansion in query orchestrator.
- Cursor adapter (parity with Claude Code).

### Phase 3 — Governance (3 weeks)
- Laws DSL + detector sandbox.
- Blocking detectors wired into PreToolUse.
- Punishment ladder + trust scores.
- Law dashboard view.

### Phase 4 — Deep Analysis + full Dashboard (4 weeks)
- Analysis workflow engine.
- Decision register with supersession graph.
- All dashboard views complete.
- Codex / Gemini adapters.

### Phase 5 — Hardening + HivehubCloud integration (open)
- Multi-tenant story.
- Distributed deployment guide (Raft via Vectorizer/Nexus HA).
- Trust scores feed cloud router.

---

## 12. Open Questions (decide before Phase 0 ends)

1. **Classifier migration trigger.** What metric flips us from Haiku-via-CLI to a local model (Expert or other)? Candidates: monthly $ ceiling, P95 latency persistently > 3 s, batch failure rate, or privacy-sensitive deployment. Define the threshold before we hit it.
2. **Event bus durability.** Synap streams give us speed; do we also persist raw events to object storage for replay/audit?
3. **Adapter granularity.** Do we capture every keystroke-level edit or only PostToolUse diffs? Affects volume by 10–100x.
4. **Law authoring UX.** YAML/Markdown vs. a small DSL vs. visual editor? Start with Markdown, evolve later.
5. **Cross-repo identity.** When the same function is referenced across repos, do we deduplicate? Probably yes via content-hash + symbol resolution.
6. **Punishment for the *user***, not the model. If a human pushes despite a blocking law, what's the audit trail look like?
7. **Schema evolution.** v1 events will be wrong about something. Migration story for existing embeddings/graph nodes when schema changes.

---

## 13. Glossary

| Term              | Meaning                                                           |
|-------------------|-------------------------------------------------------------------|
| **Pre-thinking**  | Phase before a model produces tokens, where Cortex injects context |
| **Hot path**      | Synchronous queries on the critical path of an AI session         |
| **Detector**      | Small sandboxed program that decides if an event violates a law   |
| **Trust score**   | Per-(model, repo) score driven by violations and decision-fidelity|
| **Bundle**        | The context payload returned by `/api/query` for injection        |
| **Punishment**    | Graduated response to a law violation (annotation → block → down-weight) |

---

## 13.5 Observability — health endpoints (phase8a)

Every long-running Cortex binary exposes a `GET /healthz` returning
a [`SubsystemStatus`](../crates/cortex-health/src/lib.rs) JSON
record (`{name, state, latency_ms, last_error?, version, since,
extras}`). `cortex-api` aggregates them into a single
`GET /v1/health` report whose `overall` is the worst observed
state across the stack — `Down` > `Degraded` > `Ok`.

```
operator → /v1/health on cortex-api (port 17000)
            ├─ self-report (cortex-api uptime, indexed_repos)
            ├─ /healthz on cortex-adapter-claude-code (port 17011)
            ├─ /v1/healthz on cortex-ingestion (port 17010)
            └─ /healthz on each worker (17021..17024)
```

Per-probe budget is 1.5 s. A failed probe marks the row `Down`
with a `last_error` reason but never fails the aggregator call,
so a single dead worker can't take down the whole report. The
operator scripts `scripts/health.sh` / `scripts/health.bat` print
the report and exit `0`/`1`/`2`/`3` for `ok`/`degraded`/`down`/
`unreachable` — wire into CI smoke jobs to catch silent stack
degradation in <2 s.

Closes the 2026-04-28 incident class where every component
looked individually healthy but the stack was silently degraded
(adapter publisher had stalled; the gap took ~2 hours to trace).

## 13.6 Observability — pipeline stage metrics & freshness (phase8b)

`/v1/health` answers "is each component alive?". `/v1/health/freshness`
and `/v1/health/divergence` answer the question phase8a couldn't:
**is data still moving?**

Every pipeline stage now exports a per-kind `last_*_ts_ms` and a
matching `*_total{kind|hook}` counter via `/healthz` extras *and* a
parallel Prometheus-text `/metrics` endpoint mounted on the same
listener. The freshness aggregator on cortex-api fans out across
the stack, parses the per-kind extras, and returns a flat table
keyed `<stage>.<kind>` with `gap_seconds` derived from "now -
last_event_ts". The divergence aggregator pairs adjacent stages
(`adapter.frames_parsed → adapter.envelopes_built →
adapter.envelopes_publish_ok → ingestion.events_archived`) so a
silent drop localises to the offending boundary in seconds.

```
operator → /v1/health/freshness  on cortex-api
operator → /v1/health/divergence on cortex-api
              │
              ├─ /healthz on cortex-adapter (port 17011)
              │     extras: frames_received_total / frames_parsed_total
              │             envelopes_built_total / envelopes_publish_ok_total
              │             last_frame_ts_ms / last_envelope_ts_ms
              ├─ /v1/healthz on cortex-ingestion (port 17010)
              │     extras: events_received_total / events_archived_total
              │             events_rejected_total / last_archive_write_ts_ms
              └─ /healthz on each worker (17021..17024)
                    extras: jobs_processed_total / last_job_ts_ms
```

Severity rules:
- `gap_seconds > 60` → `warn`; `gap_seconds > 300` → `critical`
- `delta_growth_60s > 10` → `warn`; `delta_growth_60s > 50` → `critical`

The 2026-04-28 JSON-truncation incident would have surfaced as a
divergence row of shape
`adapter.frames_parsed → adapter.envelopes_built ≈ 100 delta_growth`
within seconds — instead of the ~2 h grep-the-logs search that
actually found it.

See [`docs/metrics.md`](metrics.md) for the canonical metric names
+ labels each crate exposes.

## 13.7 Observability — version coherence (phase8c)

`/v1/health/freshness` answers "is data still moving?". Version
coherence answers a different but equally common operator question:
**is the running binary actually the one I just built?**

Every Cortex binary embeds at compile time, via the new
[`cortex-build`](../crates/cortex-build/) helper invoked from each
crate's `build.rs`:

- `git_sha` (full + 7-char short)
- `build_ts` (UTC RFC-3339)
- `git_dirty` (`true` if `git status --porcelain` had output at build
  time)
- `profile` (`debug` / `release`)
- `crate_version` (`CARGO_PKG_VERSION` of the calling crate)

The block lands in `/healthz extras.version` on every long-running
binary. Cortex-api's NEW `GET /v1/health/versions` aggregator fans
out, parses the version blocks, computes drift against the workspace
HEAD captured once at boot, and returns:

```json
{
  "head_sha": "<workspace HEAD>",
  "running_binaries": [{ "name": "...", "git_sha": "...", "matches_head": false, ... }],
  "drift": [{ "binary": "...", "running_sha": "...",
              "expected_sha": "...", "behind_by_commits": 3 }],
  "all_in_sync": false
}
```

`scripts/doctor-versions.{bat,sh}` curls the endpoint, prints a
table, and exits non-zero when `all_in_sync == false`. Wire into
operator workflows ("did I forget to restart cortex-adapter after
the last `cargo build`?") and CI smoke jobs.

A separate GitHub Action (`.github/workflows/version-coherence.yml`)
defends the rare path where someone commits a release binary into
the repo: the workflow rejects PRs whose `target/release/<bin>` mtime
is older than the most-recent source mtime in the owning crate.

Closes the 2026-04-28 incident class where the source had the fix
but the running `cortex-api.exe` had been built before the commit —
no way to ask the running daemon "what git SHA were you built from?"
turned a 5-minute fix into a 2-hour mystery.

## 13.8 Observability — config coherence (phase8d)

`cortex-api`'s NEW `GET /v1/health/config` endpoint and the new
`cortex-ops doctor-config` subcommand share a single pure-function
audit that compares **what the config files say** to **what the
running processes are actually using** to **what should be coherent
across surfaces**.

Surfaces audited:
- `.env` — `CORTEX_*_URL` family, plus arbitrary `KEY=VALUE` lines
- `~/.cortex/adapter.toml` — `[adapter] endpoint`, `api_endpoint`
- `cortex-plugin/.mcp.json` — `mcpServers.cortex.env.CORTEX_API_URL`
- `cortex-plugin/hooks/hooks.json` — registered hooks list

Cross-checks:
- `adapter.toml.endpoint` MUST equal `.env CORTEX_INGESTION_URL`
- `adapter.toml.api_endpoint` MUST equal `.env CORTEX_API_URL`
- `.mcp.json CORTEX_API_URL` MUST equal `.env CORTEX_API_URL`
- `hooks.json` MUST register all 7 canonical Claude Code hooks
  (`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`,
  `SubagentStop`, `SessionStart`, `Notification`)
- Every `*_URL` in `.env` MUST parse with an explicit port

Severity rules:
- `severity: critical` — port mismatch between `adapter.toml` and
  `.env` (the 2026-04-28 bug).
- `severity: warn` — missing optional surface (no
  `~/.cortex/adapter.toml`), missing canonical hook(s).
- `severity: ok` — everything aligns.

CLI exit codes (`cortex-ops doctor-config`,
`scripts/doctor-config.{bat,sh}`): `0` ok, `1` warn, `2` critical.
Suitable for CI gates and operator quick-check.

Closes the 2026-04-28 incident's *first* wrong turn: the adapter
was talking to `:15010` while ingestion was bound to `:17010`, the
config file had the right value, but a stale daemon was holding
the old endpoint in memory. The audit catches the disagreement
before it matters; phase8c catches the stale daemon.

## 13.9 Observability — silent-drop detector (phase8e)

phase8b added counters at every stage; phase8c added "is the
running binary the right one?"; phase8d added "are the configs
coherent?". phase8e closes the loop with the *detector*: a
background watcher in `cortex-api` that polls the same divergence
data the `/v1/health/divergence` endpoint surfaces and emits a
`law_violation` envelope whenever a sustained drop transitions
`Ok → Warn` or `Warn → Critical`.

The crucial difference from phase8b: this is a push channel. The
alert envelope lands in the durable archive AND the in-memory
keyword lane, so it shows up in the existing Live Timeline and
Violations dashboard without anyone having to remember to call
the divergence endpoint.

Debouncing rules:
- `Ok → Warn`: 2 consecutive polls observe `delta_growth >
  warn_delta` (default 10). Avoids transient bursts.
- `Warn → Critical`: a single poll exceeding `critical_delta`
  (default 50) suffices.
- Recovery: 5 consecutive polls observe non-growing delta.

Per-pair state persists at `~/.cortex/alerts/<pair>.json` so a
restart does not re-fire alerts the previous run already flagged.
Optional escalation hooks (gated by `SilentDropConfig`):
- `webhook_url` — every transition is POSTed here as JSON.
- `write_to_handoff` — every Critical transition appends a single
  `[silent-drop alert]`-prefixed line to
  `.rulebook/handoff/_pending.md` so the next session sees it.

Closes the 2026-04-28 silent-drop incident class verbatim: the
divergence between `adapter.frames_parsed` and
`adapter.envelopes_built` would have surfaced as a `law_violation`
envelope in the GUI within ~60 seconds of the truncation hitting,
with a message naming both counter values.

## 14. References (within HiveLLM and external)

- Vectorizer — `e:/HiveLLM/Vectorizer` (vector DB, MCP, embeddings)
- Nexus — `e:/HiveLLM/Nexus` (graph DB, Cypher, KNN)
- Synap — `e:/HiveLLM/Synap` (KV, streams, pub/sub)
- Lexum — `e:/HiveLLM/Lexum` (full-text, Tantivy) — *not production-ready yet; replace Meilisearch when ready*
- Meilisearch — https://www.meilisearch.com/ (full-text, used as Lexum stand-in)
- Expert — `e:/HiveLLM/Expert` (Qwen3-0.6B + adapters) — *not used in v1; reserved for future migration*
- Claude Haiku — invoked via Claude Code CLI (`claude -p ... --model claude-haiku-4-5`) or Anthropic SDK directly
- Rulebook — `e:/HiveLLM/Rulebook` (rules, hooks, persistent memory)
- HivehubCloud — `e:/HiveLLM/HivehubCloud` (SaaS that integrates the above)

---

*This document is a draft. Open questions in §11 must be resolved before implementation begins. Subsequent specs (event-schema, law-dsl, query-api, adapter-protocol) will live alongside this file under `docs/`.*

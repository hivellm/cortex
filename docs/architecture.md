# Cortex — Architecture

> **Status:** Original architecture vision (2026-04-17), substantially implemented since; see [`docs/analysis/cortex-platform-2026-07/`](../analysis/cortex-platform-2026-07/) for current state and open items.
> **Owners:** HiveLLM core team
> **Design intent from:** 2026-04-17

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

### 6.0 Tiered storage: raw → consolidation → Parquet archive

Three tiers of derived state sit alongside the raw envelope corpus, each with a different cost / fidelity tradeoff:

| Tier              | Source                                    | Storage                                         | Lifetime                          |
|-------------------|-------------------------------------------|-------------------------------------------------|-----------------------------------|
| **Raw events**    | Live capture + bootstrap                  | Vectorizer hot (`cortex.{turn,tool_call,…}.fp32`) + per-repo Meili indexes + Nexus | 0-90 days hot, 90-365 days warm (PQ), > 365 days cold (binary) |
| **Consolidations** (phase11j) | LLM summarisation of raw windows | Vectorizer `cortex.consolidation.{fp32,pq}` + global `cortex_consolidations` Meili index | Same hot/warm/cold schedule as raw |
| **Parquet archive** | Bootstrap + every live envelope routed through `cortex.events.{raw,bootstrap}` | zstd-NDJSON partitions on local disk under `$CORTEX_ARCHIVE_ROOT` | Indefinite — durable replay source for hot/warm/cold rebuilds |

Consolidations distill many raw events into one summary the agent can read in a single line. The pre-thinking renderer's `## Consolidated context` section (spec 12 §Output) reads them through the same query lanes the raw retrieval uses; nothing in the pipeline knows the difference except the dedicated index / collection (`cortex_consolidations`, `cortex.consolidation.fp32` + `.pq`) and the `kind = consolidation` filter. See [`docs/cortex/consolidation-tuning.md`](cortex/consolidation-tuning.md) for the operator handbook (cost guardrails, fidelity threshold tuning, prompt template iteration).

The Parquet archive is the durable source for everything below it: a tier rebuild walks the matching partition window, streams envelopes back through `cortex.events.bootstrap`, and lets the standard processing layer re-populate Vectorizer + Meili + Nexus. The pruner (phase11j §5, blocked on the upstream Vectorizer SDK gaining `move_to_collection` + `delete_vectors` — tracked by `phase11o_vectorizer_demotion_api`) is the demotion driver that walks active consolidations, resolves `source_event_id` lifetimes, and moves events between hot / warm / cold per the schedule above. Until the SDK ships, the Parquet archive carries the only complete cold-tier representation; the Vectorizer + Meili tiers grow monotonically.

### 6.0a Living-synthesis tier: topic cards on top of consolidations

Phase 11r layers a **living-synthesis** tier on top of consolidations. Where a consolidation is a snapshot of a window (one session, one topic burst, one decision trace), a **topic card** is a slug-keyed prose synthesis the orchestrator **rewrites in place** as new evidence accumulates. One card per `(topic_slug, repo_scope)`; the deterministic id (`topic-{24-hex}` derived from `sha256(slug ⊕ repo_scope)`) means re-emitting the same card lands on the same node and bumps `revision`.

| Layer              | Granularity                          | Mutation model                  | Storage                                                                                       |
|--------------------|--------------------------------------|---------------------------------|-----------------------------------------------------------------------------------------------|
| Raw events         | One envelope per turn / tool call    | Append-only                     | Per-repo Vectorizer / Meili / Nexus                                                           |
| Consolidations     | Many raw events → one summary        | Append-only (new id per window) | `cortex.consolidation.{fp32,pq}` + `cortex_consolidations`                                    |
| **Topic cards**    | Many consolidations → one synthesis  | **Rewritten in place** per rev  | `cortex.topic_card.{fp32,pq}` (recall-tuned `m=48 / ef_search=256`) + `cortex_topic_cards`    |

Three trigger heuristics fire a rewrite (any one is sufficient):

1. **Burst** — `events_since_last_rev ≥ 8` (`TRIGGER_EVENTS_THRESHOLD`).
2. **High-impact proximity** — a new event lands within `0.30` distance and is a Decision / LawViolation / high-impact-outcome event.
3. **Stale + new evidence** — `synthesis_age_d ≥ 14` AND any new evidence cited.

When none fire, the orchestrator emits `Hold { reason }` (`Cooldown` / `LowImpact` / `NotRelevant`). The synthesiser composition reuses the consolidator's `Summariser` trait — no new abstract layer; the orchestrator escalates Haiku → Opus only when (a) `force_deep` is set, (b) ≥ 3 open contradictions exist, or (c) the existing evidence already trips a `decision_supersession` per the contradiction scanner.

Three contradiction detectors run on every rewrite:

- **`DecisionSupersession`** — any pair where one decision's `supersedes` matches another's `decision_id`.
- **`LawViolationMismatch`** — a violation citing a different version than the matching active Law.
- **`OutcomeDivergence`** — two consolidations with overlapping temporal spans + different outcome majorities.

Each emitted contradiction stamps `surfaced_at_rev` and `status = Open`; the producer never blocks a rewrite on contradictions — they are heuristic surface signals the agent reads.

The pre-thinking renderer ([formatter.rs](../crates/cortex-pre-thinking/src/formatter.rs)) gives the topic-card section **top priority** in the section-ordering matrix (laws → topic_cards → consolidations → decisions → similar_turns → past_sessions → snippets) when the card is fresh. Staleness — `confidence < 0.6` OR (`synthesis_age_d > 30` AND `events_since_last_rev > 0`) — flips the order so consolidations render first and stamps a `> stale-topic-card: <reason>` advisory line. Section budget: 1 400 bytes (`section_caps::TOPIC_CARDS_BYTES`).

The MCP tool surface adds five new tools that operate on topic cards: `cortex_topic_get`, `cortex_topic_drill`, `cortex_topic_neighbors`, `cortex_topic_diff`, `cortex_synthesize`. See [`docs/cortex/topic-cards.md`](cortex/topic-cards.md) for the operator runbook (force-rewrite, replay, dry-run cost cap, the MCP tool reference table) and [ADR-006](../.rulebook/decisions/006-topic-card-as-living-synthesis-vs-consolidation-as-snapshot.md) for the choice to layer a separate `Kind::TopicCard` over `Kind::Consolidation` rather than mutating the consolidation kind.

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
| Claude Code conversation archive (`~/.claude/projects/<project>/*.jsonl`) | `Turn` / `ToolCall` / `AgentCall` (phase11i) | Walked + tailed by [`cortex-claude-archive`](../crates/cortex-claude-archive/); each user/assistant pair maps to one `Turn`, each `tool_use` block to one `ToolCall`, each Task tool to one `AgentCall`. Sessions persist across daemon restarts. |

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
- **Claude Code conversation archive** (phase11i §5.2) — [`cortex-claude-archive tail`](../crates/cortex-claude-archive/) runs as a long-running daemon (1 Hz mtime poll, axum `:17030/healthz` endpoint, sysinfo RSS sampler). The compose stack ships it alongside the live workers; cortex-api's `/v1/health/coverage` surfaces the watcher's snapshot under an `archive_watchers` block.

This means after the initial backfill, the corpus stays current automatically — every new commit, doc change, merged PR, AND every newly-appended Claude Code session appears in retrieval results within seconds.

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

1. **Classifier migration trigger.** What metric flips us from Haiku-via-CLI to a local model (Expert or other)? Candidates: monthly $ ceiling, P95 latency persistently > 3 s, batch failure rate, or privacy-sensitive deployment. Define the threshold before we hit it. **Status: still open**
2. **Event bus durability.** Synap streams give us speed; do we also persist raw events to object storage for replay/audit? **Status: not verified in this pass**
3. **Adapter granularity.** Do we capture every keystroke-level edit or only PostToolUse diffs? Affects volume by 10–100x. **Status: resolved to PostToolUse diffs per live config**
4. **Law authoring UX.** YAML/Markdown vs. a small DSL vs. visual editor? Start with Markdown, evolve later. **Status: resolved to Markdown/YAML per §5.4**
5. **Cross-repo identity.** When the same function is referenced across repos, do we deduplicate? Probably yes via content-hash + symbol resolution. **Status: resolved via (repo, path, content_hash) idempotent identity per §6.2**
6. **Punishment for the *user***, not the model. If a human pushes despite a blocking law, what's the audit trail look like? **Status: still open (punishment ladder today targets model, not user)**
7. **Schema evolution.** v1 events will be wrong about something. Migration story for existing embeddings/graph nodes when schema changes. **Status: not verified in this pass**

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
operator scripts `scripts/doctor/health.sh` / `scripts/doctor/health.bat` print
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

`scripts/doctor/doctor-versions.{bat,sh}` curls the endpoint, prints a
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
- `packages/cortex-claude-plugin/.mcp.json` — `mcpServers.cortex.env.CORTEX_API_URL`
- `packages/cortex-claude-plugin/hooks/hooks.json` — registered hooks list

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
`scripts/doctor/doctor-config.{bat,sh}`): `0` ok, `1` warn, `2` critical.
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

## 13.10 Observability — synthetic E2E canary (phase8f)

phase8a–8e watch *real* traffic. During quiet hours (no claude-code
activity) the pipeline could be silently broken for hours and no
divergence would fire because no events are flowing. phase8f closes
that gap with a synthetic canary: every N seconds, fire a known
fake hook frame through the real IPC path and assert it lands in
the archive within a deadline. Failures emit a `law_violation`
envelope via the same alert path phase8e uses.

The canary frame mimics the 2026-04-28 regression vector verbatim:
pretty-printed JSON with newlines between top-level fields and
multi-line `\n`-escaped strings inside `tool_response.stdout`. A
canary that mimics that behavior would have caught the truncation
regression the moment it landed.

Components:
- **Library** — `cortex_api::canary` exports `run_canary_once`
  (round-trip via `send_frame_via_ipc` + `poll_archive_for_marker`),
  `build_canary_frame` (per-hook fixture builder), and
  `run_canary_loop` (the background runner).
- **CLI** — `cortex-ops canary --hook=PostToolUse` invokes the
  same library function and exits 0/1/2 per the outcome bucket.
  `scripts/doctor/canary.{bat,sh}` thin wrappers.
- **Background runner** — opt-in via
  `CORTEX_CANARY_ENABLED=1` env var (off by default to keep cold
  dev quiet). When on, ticks every `CORTEX_CANARY_INTERVAL_SECS`
  (default 300), records every result to
  `~/.cortex/canary-history.jsonl`, and POSTs a `law_violation`
  envelope on failure (`law_id: canary-PostToolUse`, severity
  `critical`).
- **History** — `canary-history.jsonl` is append-only; a `tail -f`
  is the operator's view of recent canary outcomes.

CLI exit codes (`cortex-ops canary`, `scripts/doctor/canary.{bat,sh}`):
`0` round-trip success, `1` transport / connect error, `2`
deadline elapsed without observing the marker.

Closes the *quiet-hours* failure class: a regression like the
2026-04-28 JSON truncation is now detected in 10 seconds (or 5
minutes if running on the default schedule) instead of hours.

## 13.11 Observability — dashboard Health view (phase8g)

phase8a–8f produce rich JSON over `/v1/health/*`, but the user
realistically opens the GUI before the terminal. phase8g surfaces
the entire health system as a first-class dashboard view at
`/health`, with the same visual language as Live Timeline /
Conversations / Decisions.

Layout (top-down):
1. **Overall banner** — green/yellow/red driven by
   `health.overview().overall`.
2. **Subsystems grid** — one card per crate from
   `health.overview().subsystems[]` (state pill, version, latency).
3. **Freshness table** — rows sorted by `gap_seconds` desc, colour-
   coded against the phase8b severity buckets (warn > 60 s,
   critical > 300 s).
4. **Divergence table** — adjacent-stage drops where
   `severity != ok`.
5. **Version drift** — rendered only when `versions.all_in_sync`
   is `false`; lists each binary's running SHA vs workspace HEAD
   plus `behind_by_commits`.
6. **Config audit** — findings from phase8d's audit where
   `severity != ok`.

Real-time pulse: NEW `GET /v1/health/stream` SSE endpoint emits a
combined `HealthSnapshot { overall, freshness, divergence,
truncated }` every 5 seconds plus a `heartbeat` every 15 s. The
snapshot is byte-capped at 64 KiB; oversized payloads halve the
freshness vec until they fit and flip `truncated: true`.

Topbar status pill: every view (Live Timeline, Conversations, …)
shows a tiny pill in the header carrying the overall health label.
Click to jump to `/health`. Polls `/v1/health` every 5 s — the
user can't miss a stack-degraded state while browsing other views.

## 13.12 Observability — CI smoke gate (phase8h)

phase8a–8g detect issues in *production*. phase8h closes the loop:
prevent the issues from reaching production at all by booting the
full Cortex stack inside CI for every PR and running the synthetic
canary plus the doctor checks against it. If the canary times out
or any doctor check flags a critical finding, the workflow fails
the PR.

Today's `cargo test` suite uses in-process fakes
(`MemoryPublisher`, `MemoryKeywordLane`, …). It would not have
caught the 2026-04-28 JSON truncation bug because the bug only
manifested over a real named pipe with a real binary. CI must
boot the binaries.

Components:
- **NEW `.github/workflows/health-smoke.yml`** — runs on every PR
  and push to main, matrix `[ubuntu-latest, windows-latest]`,
  12-minute budget per matrix entry.
- **NEW `scripts/ci/boot-stack.{sh,bat}`** — spawn cortex-ingestion,
  cortex-api, cortex-adapter-claude-code in the background; wait
  for `/v1/health` to report `ok` or `degraded` (60 s timeout);
  pid file at `$CORTEX_PIDS_FILE` for teardown.
- **NEW `scripts/ci/teardown-stack.{sh,bat}`** — read the pid file
  and SIGTERM (then SIGKILL after 5 s) every spawned daemon.
  Idempotent.
- **CORTEX_HOME isolation** — every run gets
  `${{ runner.temp }}/cortex-home-<run_id>-<attempt>` so concurrent
  matrix legs don't collide on a shared `~/.cortex`.

Doctor checks gate the PR (each must exit clean):
1. `scripts/doctor/health.{bat,sh}` (phase8a) — overall ≤ degraded.
2. `scripts/doctor/doctor-versions.{bat,sh}` (phase8c) — no drift between
   running binaries and HEAD (CI just built them, drift means a
   stale cargo cache).
3. `scripts/doctor/doctor-config.{bat,sh}` (phase8d) — no critical
   findings (warns allowed; e.g. missing adapter.toml in a fresh
   CI checkout).
4. `cortex-ops canary --hook=PostToolUse --deadline-secs=15`
   (phase8f) — synthetic frame round-trips through real IPC.

Failure path: when any step fails, the workflow uploads the
`$CORTEX_HOME/logs/*.log` files as a named artifact
(`cortex-logs-<os>-<run_id>-<attempt>`) so the PR author can
download and inspect them without re-running CI locally.

External services (Vectorizer / Nexus / Synap / Meili) are NOT
booted — the cortex-api `Memory*` lane fallbacks already let the
stack boot without them, and the doctor checks treat the missing
services as `degraded`, which is the expected smoke shape.
Live-service integration runs in a separate nightly workflow
once that's worth the runner cost.

NEW `.github/PULL_REQUEST_TEMPLATE.md` carries a soft "Health
checks" section with checkboxes for the four scripts. The
workflow is the enforced gate; the template raises author
awareness.

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

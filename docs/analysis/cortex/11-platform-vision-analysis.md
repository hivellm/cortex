# 11 — Platform Vision: Cortex as General Assistance Tool (2026-06-09)

> **Author:** strategic audit · **Source data:** full codebase review at commit `c8a037e` + archived phase history + specs 01–36 + knowledge base.
> **Scope:** comprehensive analysis of what Cortex has built, what the existing MCP/tools deliver, and the full roadmap to make Cortex a general assistance platform — for both software engineering and general company knowledge (timelines, cases, business context).
> **Audience:** project owner (André), future maintainers, and anyone deciding what to build next.

---

## TL;DR

Cortex is a **capture → classify → retrieve → govern loop** built specifically around AI coding sessions. As of June 2026, the capture and retrieval legs are structurally complete for code contexts. The governance, deep analysis, and cross-tool adapter legs are specified but mostly unbuilt.

To become a **general assistance platform** — useful for company timelines, project cases, business knowledge, and not just code — three large gaps must be closed:

1. **Ingestion breadth**: Today Cortex captures only Claude Code hooks. A general tool needs to ingest documents, meetings, project trackers, decisions, and timelines from any source.
2. **Entity model expansion**: Today's entities are code-centric (Symbol, Artifact, Decision, LawViolation). A general tool needs business entities (Person, Project, Case, Timeline, Meeting, Milestone, Client).
3. **Query and pre-thinking for non-code intent**: Today's retrieval is tuned for code symbol lookup and session context. A general tool needs temporal, organizational, and case-type query intent models.

The good news: the infrastructure (Nexus graph, Vectorizer, Meilisearch, Synap event bus, MCP surface) is already in place and extensible. The work is additive — the code-centric pipeline does not need to be dismantled.

---

## Part 1 — What Is Already Implemented (June 2026)

### 1.1 Core Data Pipeline

The end-to-end pipeline is live and producing data:

```
Claude Code hooks
  → cortex-adapter-claude-code (daemon, port 5555)
  → Synap event bus (raw stream)
  → cortex-classifier-worker (Haiku classification + static fallback)
  → Synap (enriched stream)
  → cortex-embedder-worker (tree-sitter chunking → Vectorizer)
  → cortex-graph-worker (Cypher UNWIND → Nexus)
  → cortex-fulltext-worker (Meilisearch per-repo per-kind collections)
  → cortex-api (/v1/query hybrid RRF + /v1/pre-thinking)
  → cortex-mcp-server (MCP stdio bridge)
  → Claude Code pre-thinking injection
```

Every step is a separate worker, parallel, with durable Synap offset tracking (SQLite fallback). Replay is possible from the Parquet archive.

### 1.2 Event Schema (12 Kinds)

All 12 event kinds are implemented and validated against JSON Schema:

| Kind           | What it captures                                   | Where it goes             |
|----------------|----------------------------------------------------|---------------------------|
| `Turn`         | User ↔ assistant exchange                          | All 4 backends            |
| `ToolCall`     | Tool invocation (Bash, Edit, Read, MCP, etc.)      | All 4 backends            |
| `AgentCall`    | Sub-agent dispatch and result                      | All 4 backends            |
| `Memory`       | Rulebook memory save/update/delete                 | All 4 backends            |
| `Decision`     | ADR-style formalized decision record               | All 4 backends + graph    |
| `Analysis`     | Sonnet-backed cross-event analysis report          | Vectorizer + Meili        |
| `LawViolation` | Governance rule breach event                       | Nexus + Meili             |
| `Artifact`     | File/diff/snippet reference with content hash      | Vectorizer + Nexus        |
| `Knowledge`    | Rulebook knowledge base entry                      | All 4 backends            |
| `Learning`     | Rulebook learning capture                          | All 4 backends            |
| `Consolidation`| Distilled session/topic/decision summary           | Vectorizer + Meili        |
| `TopicCard`    | Living synthesis card with contradiction tracking  | Vectorizer + Meili        |

### 1.3 Backends and Their Roles

| Backend        | Version | Role                                            | Current coverage          |
|----------------|---------|--------------------------------------------------|---------------------------|
| **Synap**      | 0.12    | Event bus + KV cache + durable offsets           | All repos, all streams    |
| **Vectorizer** | 3.3.0   | Dense + BM25 vector search (HNSW)               | 3 repos, ~128k vectors    |
| **Nexus**      | 2.1.0   | Graph DB — Cypher executor, 20+ edge types       | 3 repos, ~3.6k nodes      |
| **Meilisearch**| latest  | Full-text inverted index, faceted filters        | 1 repo, 589 docs (gap)    |

### 1.4 Rust Crate Inventory (14 crates)

| Crate                           | LOC est. | Function                                                     |
|---------------------------------|----------|--------------------------------------------------------------|
| `cortex-core`                   | ~3k      | Event types, envelope, redactor, JSON Schema validator       |
| `cortex-config`                 | ~2k      | ADR-016 typed config, env-var map, TOML + defaults           |
| `cortex-storage`                | ~2k      | Parquet archive, zstd compression, SQLite metadata           |
| `cortex-health`                 | ~1k      | Health check library (freshness, divergence, versions)       |
| `cortex-build`                  | ~0.5k    | Build-time codegen (enums, schemas)                          |
| `cortex-workers`                | ~15k     | Unified binary hosting all 8 workers + CLIs                  |
| `cortex-api`                    | ~8k      | Hybrid query API, dashboard backend, SSE, auth               |
| `cortex-pre-thinking`           | ~3k      | Context bundle assembly for pre-thinking injection           |
| `cortex-mcp-server`             | ~2k      | MCP stdio bridge (JSON-RPC 2.0)                              |
| `cortex-cli`                    | ~4k      | Bootstrap, ops, relevance-eval CLIs                          |
| `cortex-adapter-claude-code`    | ~3k      | Hooks daemon, event capture, pre-thinking proxy              |
| `cortex-eval`                   | ~1k      | Retrieval quality eval harness (not in CI yet)               |

### 1.5 MCP Server and Tool Surface

The MCP server (`cortex-mcp-server`) is the primary interface between Claude Code and the Cortex knowledge base. It speaks JSON-RPC 2.0 over stdio.

**Currently exposed tools:**

| Tool                       | Input                                    | Output                                     | Status  |
|----------------------------|------------------------------------------|--------------------------------------------|---------|
| `cortex_pre_thinking`      | `query`, `scope`, `intent`               | Compact context bundle (decisions, laws, similar turns, snippets) | ✅ Live |
| `cortex_query`             | `query`, `scope`, `intent`, `limit`      | Ranked results (RRF fusion: vector + keyword + graph) | ✅ Live |
| `cortex_decision_get`      | `decision_id`                            | Full decision record + context             | ✅ Live |
| `cortex_topic_get`         | `topic_id`                               | Topic card (summary, contradictions, references) | ✅ Live |
| `cortex_topic_drill`       | `topic_id`, `depth`                      | Expanded neighborhood of a topic          | ✅ Live |
| `cortex_topic_neighbors`   | `topic_id`                               | Related topics                             | ✅ Live |
| `cortex_topic_synthesize`  | `topic_id`                               | Trigger Sonnet synthesis                  | ✅ Live |

**What the MCP server does NOT expose (gaps):**

- Graph traversal / Cypher queries (gated behind `CORTEX_GRAPH_ENABLE_CYPHER=false`)
- Law evaluation / governance pre-check
- Session timeline navigation
- Artifact diff lookup
- Cross-project/org aggregation
- Timeline queries (temporal slicing)

### 1.6 Dashboard (GUI) — 21 Views Shipped

The Electron dashboard (`gui/`) has 21 React views, all wired to the `cortex-api` backend:

- **Live data**: Timeline, Consolidations, Conversations, Handoffs
- **Knowledge**: Decisions, Laws, Memory, Knowledge (entity browser)
- **Code intelligence**: Graph explorer, Connections, BranchExplorer, BitemporalTimeline, EntityHistory
- **Quality**: Coverage (edge-kind heatmap), PreThinkingQuality, Classifications, Tools, Producers
- **Operations**: Health, Retention, Tasks

Most views are functional. Gaps: auth enforcement, large-timeline perf, laws view reads stubs not live engine.

### 1.7 Bootstrap and Ingestion

- `cortex-bootstrap` walks repos on disk, promotes decisions/laws/memories/analyses into Cortex events
- Supports incremental rerun (checkpoint state), multi-repo orchestration (single invocation covers N repos)
- `cortex.toml` per-repo config: chunking strategy, exclude lists, git include/exclude, promotion patterns
- **Bootstrapped repos** (as of June 2026): Cortex, Vectorizer, Nexus — 3 of 17 Hive repos

### 1.8 Consolidator and Sonnet Analysis

- `cortex-consolidator` detects session end via idle-window detection (phase24) and produces `Consolidation` events
- Three consolidation grains: Session (narrative summary), Topic (cross-session theme), DecisionTrace (decision lineage)
- Sonnet-backed session analyzer provides cross-event analysis (landed phase15+)
- Decision synthesis via debate workflow: Sonnet proposes, a second Sonnet critiques, third synthesizes

---

## Part 2 — Validation of Existing Tools

### 2.1 MCP Tool Quality Assessment

**`cortex_pre_thinking`** — ✅ Working, valuable
- Fetches laws, recent decisions, similar past turns, and code snippets relevant to the current query
- Injected via `PreToolUse` hook on every Claude Code turn
- **Gap**: bundle content is ephemeral (not persisted for audit). Spec 12 promises a `query_id` trail — not yet implemented.
- **Gap**: latency not measured in CI; p95 not tracked per bundle.

**`cortex_query`** — ✅ Working, needs quality measurement
- RRF fusion of vector (Vectorizer), keyword (Meilisearch), and graph (Nexus) lanes
- Per-intent recency decay (ADR-003)
- **Gap**: no recall@10 / MRR benchmark. We don't know if it's better than each lane alone.
- **Gap**: Meilisearch fan-out incomplete (1 repo vs 3 in other backends) — keyword lane systematically underpowered.

**`cortex_decision_get`** — ✅ Working, narrow use case
- Good for "show me ADR-003" lookups
- **Gap**: no fuzzy match — caller must know the decision_id. Needs semantic search fallback.

**`cortex_topic_*`** — ⚠️ Functional but shallow
- Topic cards exist but are sparsely populated (classifier assigns topics, but contradiction tracking is early-stage)
- `cortex_topic_synthesize` triggers Sonnet synthesis but this is one-shot, not iteratively refined
- **Gap**: no topic evolution history — can't answer "how has this topic changed over the last month?"

### 2.2 Backend Integration Health

| Backend        | Status             | Known issues                                           |
|----------------|--------------------|---------------------------------------------------------|
| **Synap**      | ✅ Healthy         | Offset recovery tested (phase15h)                       |
| **Vectorizer** | ⚠️ SDK drifts       | `upsert` reports 4-5 failures/batch (vector_count=0); 2 of 6 known drifts unresolved |
| **Nexus**      | ⚠️ Perf blocked     | nexus#12 — sustained-write stall at 100% CPU under semantic-edge load; blocks CALLS/IMPORTS projection |
| **Meilisearch**| ⚠️ Under-indexed   | Only 1 repo indexed vs 3 in other backends; keyword lane asymmetric |

### 2.3 Adapter Coverage

| Tool              | Adapter status                           |
|-------------------|------------------------------------------|
| Claude Code       | ✅ Full (PreToolUse, PostToolUse, Stop, SubagentStop) |
| Cursor            | ❌ Spec 17, not yet implemented          |
| GitHub Copilot    | ❌ Not specced                           |
| OpenCode          | ❌ Placeholder only                      |
| Gemini            | ❌ Spec 17, not yet implemented          |

Only Claude Code sessions are captured. This is a significant coverage gap.

### 2.4 Governance Engine Status

| Component                   | Status                    |
|-----------------------------|---------------------------|
| Law registry (YAML/TOML)    | ⚠️ Partially (AGENTS.md promoted via bootstrap, but no `.cortex/laws/*.yaml` schema) |
| Law evaluator (PreToolUse)  | ❌ Not built              |
| LawViolation write path     | ❌ Not wired live         |
| Trust score materialization | ❌ Dashboard stub only    |
| Enforcement (reject/warn)   | ❌ Not built              |
| Law DSL sandbox             | ❌ Spec 13 draft          |

Dashboard shows law tables but they read from fixtures, not a live engine.

---

## Part 3 — Gap Analysis for General Assistance

The user's vision: Cortex as a **general assistance tool** covering both:
- **A) Software engineering** — code creation, review, improvement, AI-session memory
- **B) Company knowledge** — timelines, cases, business processes, org decisions, team context

The current system covers A partially. B is entirely absent. Here is a structured gap analysis.

### 3.1 Ingestion Layer Gaps

**What exists**: Claude Code hook capture + file-based bootstrap (git repos, `.rulebook/` files).

**What is needed for general company use**:

| Source                    | Gap                                      | Priority |
|---------------------------|------------------------------------------|----------|
| Business documents (PDF, DOCX, Notion, Confluence) | No ingestion adapter              | P1 — required for company KB |
| Meeting transcripts / recordings | No ingestion pipeline          | P2 — high value for decisions |
| Email threads             | No ingestion adapter                     | P2       |
| Slack / Teams channels    | No ingestion adapter                     | P1 — key communication channel |
| Project management (Linear, Jira, Trello) | No ingestion adapter      | P1 — task/case timelines |
| Calendar events / meeting notes | No ingestion adapter             | P2       |
| CRM / client records      | No ingestion adapter                     | P3       |
| Custom API integrations   | No generic adapter framework             | P2       |

**Root cause**: The ingestion model is a hooks-based event capture from AI tools. It needs a second ingestion model: **document import** that accepts arbitrary structured/unstructured content and routes it through the classify → embed → graph → fulltext pipeline.

**What to build**: A `cortex-ingestion-adapter` framework with:
- A generic `ImportRequest { source_type, source_id, raw_content, metadata }` envelope
- Source-specific connectors (PDF extractor, Notion exporter, Slack export parser, Linear webhook)
- Route imported content into the existing pipeline (same Synap stream, same workers)

### 3.2 Entity Model Gaps

**What exists** (code-centric entities):
```
Session, Turn, ToolCall, AgentCall, Artifact, Symbol,
Decision, Memory, Analysis, Law, LawViolation, Model, Repo, Topic
```

**What is needed** (business entities):

| Entity          | Purpose                                                   | Maps to existing? |
|-----------------|-----------------------------------------------------------|-------------------|
| `Person`        | Team member, client, stakeholder                          | No — new node type |
| `Project`       | Business project / initiative (not git repo)              | No — new node type |
| `Case`          | Support ticket, legal case, incident                      | No — new node type |
| `Meeting`       | Scheduled or recorded meeting                             | Partial (Turn can model meeting turn) |
| `Milestone`     | Deadline, release, checkpoint                             | No — new node type |
| `Client`        | External organization                                     | No — new node type |
| `Document`      | Business document (contract, spec, report)                | Partial (Artifact covers code files) |
| `Timeline`      | Ordered sequence of events around a topic                 | Partially (bitemporal spec 33 covers the temporal dimension) |
| `Objective`     | OKR, goal, target                                         | No — new node type |
| `Risk`          | Identified risk item                                      | Partial (LawViolation is governance risk) |

**What to build**: 
- Extend the `Kind` enum (currently 12 variants) with `BusinessEvent`, `DocumentImport`, `MeetingRecord`
- Extend the Nexus graph schema with business entity node types
- New projection rules in `cortex-workers/graph/projection.rs` for business → graph edges

**Key relationships needed**:
```
(Person)-[:PARTICIPATES_IN]->(Meeting)
(Person)-[:OWNS]->(Project)
(Project)-[:HAS_MILESTONE]->(Milestone)
(Case)-[:RELATED_TO]->(Project)
(Document)-[:REFERENCES]->(Decision)
(Meeting)-[:PRODUCED]->(Decision)
(Timeline)-[:CONTAINS]->(Milestone)
(Client)-[:HAS_PROJECT]->(Project)
```

### 3.3 Query and Retrieval Gaps for Business Use

**What exists**: Hybrid RRF query with intent types tuned for code contexts (`code_lookup`, `session_context`, `decision_trace`).

**What is needed**:

| Query intent          | Example                                        | Gap |
|-----------------------|------------------------------------------------|-----|
| Temporal slice        | "What happened in Q1 2026 on Project X?"       | Timeline API (spec 33) partially specced; not in MCP |
| Person-centric        | "What has Alice worked on this month?"         | No person entity; no PARTICIPATED_IN graph edges |
| Case history          | "Show the full timeline of Case #1234"         | No case entity, no timeline query |
| Decision lineage      | "How did we arrive at the current auth strategy?" | Partial (DecisionTrace consolidation), but not MCP-exposed |
| Cross-project summary | "What are all open risks across our projects?" | No cross-project aggregation (spec 34 draft) |
| Meeting outcomes      | "What was decided in last week's architecture meeting?" | No meeting ingestion |
| Company context       | "Who owns the infrastructure roadmap?"         | No person/ownership graph |

**What to build**:
- Expand MCP tool surface with temporal and organizational query types
- Add business-intent classifiers to the hybrid RRF query pipeline
- Implement spec 33 (Timeline API) as a first-class MCP-exposed query
- Implement spec 34 (Cross-project axis) for org-level aggregation

### 3.4 Pre-Thinking Quality for Non-Code Context

**What exists**: Pre-thinking bundles inject decisions, laws, similar turns, and code snippets.

**What is needed** for business contexts:
- Inject relevant meeting decisions when the user is working on a project
- Inject timeline context ("this project is in Q3 sprint, deadline is 2026-08-15")
- Inject team context ("Alice owns this; Bob was the last reviewer")
- Inject case history when discussing a support incident
- Inject relevant company policies (not just code governance laws)

**Root cause**: Pre-thinking bundle assembly (`cortex-pre-thinking`) is hard-coded for the four current retrieval dimensions (decisions, laws, similar turns, snippets). It needs a pluggable retrieval strategy where business contexts can add dimensions.

---

## Part 4 — What to Build (Prioritized Roadmap)

### Phase A — Complete the Code Platform (1–2 months)

These items unblock the existing spec surface and make the code-assistance story complete.

**A1 — Fix Nexus #12 sustained-write stall** [BLOCKER]
- Blocks: semantic-edge projection live (CALLS, IMPORTS, DEFINES, ABOUT edges)
- Without this, the graph topology stays shallow and graph-based code intelligence is limited
- Path: upstream fix in Nexus 2.x; or implement rate-limited batch scheduling in cortex-graph-worker as mitigation

**A2 — Governance engine MVP (specs 13–14)**
- Static law registry (`.cortex/laws/*.yaml`) + evaluator
- PreToolUse law check: reject/warn on `severity: critical`
- LawViolation write path to Nexus + Meilisearch
- Trust score materialization (daily job, per model+repo)
- Dashboard Laws view wired to live engine instead of stubs
- Value: closes the biggest structural gap in the current spec surface

**A3 — Complete Meilisearch fan-out**
- Bootstrap replay for all indexed repos (not just 1 of 3)
- Fixes keyword lane systematic underpowering in hybrid RRF
- Quick win: 2–3 days effort

**A4 — Deep Analysis workflow (spec 15)**
- Sonnet-backed orchestrated debate: multi-turn Sonnet analysis for cross-event synthesis
- Decision synthesis: from debate outcomes to formal Decision events
- Analysis entities promoted to Vectorizer + Meili (already specced in spec 15 observability)
- Dashboard Analysis view wired to live Sonnet-produced analyses

**A5 — Retrieval quality benchmark**
- 50+ labeled query/result pairs covering code lookup, session context, decision trace
- CI job scoring recall@10 + MRR per intent
- Without this, we cannot prove the hybrid RRF is better than single-lane retrieval

**A6 — Additional tool adapters (spec 17)**
- Cursor adapter (highest priority: large user base)
- GitHub Copilot adapter (most common enterprise tool)
- OpenCode / Codex adapters
- Value: "100% of AI interactions captured" becomes true

**A7 — Expose graph queries via MCP**
- Remove `CORTEX_GRAPH_ENABLE_CYPHER=false` gate or add safe Cypher template library
- Expose `cortex_graph_query(cypher_template, params)` as MCP tool with parameterized templates
- Value: unlocks code intelligence queries that vector/keyword can't answer (dependency chains, who-calls-what)

---

### Phase B — General Document Ingestion (2–3 months)

These items extend Cortex from AI-session-only capture to general document and knowledge ingestion.

**B1 — Generic document ingestion adapter**
- `POST /v1/ingest` endpoint accepting `{ source_type, source_id, raw_content, metadata }`
- Routes through the same Synap → classify → embed → graph → fulltext pipeline
- Source type registry: `pdf`, `markdown`, `html`, `plain_text`
- CLI tool: `cortex ingest <file_or_url> --type pdf --project "Cortex" --tags "spec,v2"`

**B2 — Confluence/Notion connector**
- Pull pages from a Confluence space or Notion workspace
- Map page hierarchy to `Document` entities in the graph
- `(:Document)-[:PART_OF]->(Space)` + `(:Document)-[:LINKS_TO]->(Decision)`
- Scheduled sync: connector polls for changes every N minutes

**B3 — Linear/Jira integration**
- Ingest issues/tasks as `Case` entities
- Map assignees to `Person` nodes
- `(:Case)-[:ASSIGNED_TO]->(Person)`, `(:Case)-[:PART_OF]->(Project)`, `(:Case)-[:HAS_MILESTONE]->(Milestone)`
- Status transitions become `BusinessEvent` items on the timeline

**B4 — Slack/Teams integration**
- Export message threads from channels tagged for Cortex ingestion
- Classify messages (decision discussion, question, announcement, action item)
- Extract decisions from conversation threads using Sonnet
- `(:Meeting)-[:PRODUCED]->(Decision)` when a Slack thread reaches a conclusion

**B5 — Entity model expansion (business nodes)**
- Add `Person`, `Project`, `Case`, `Meeting`, `Milestone`, `Client`, `Document` to the Nexus schema
- Extend `cortex-core` Kind enum with `BusinessEvent`, `DocumentImport`, `MeetingRecord`
- New projection rules in graph worker for business entity edges

---

### Phase C — Company Intelligence (3–6 months)

These items build the organizational knowledge layer that makes Cortex genuinely useful as a "company brain."

**C1 — Timeline API (spec 33 implementation)**
- Bitemporal queries: "What was the state of Project X on 2026-03-01?"
- Event timeline for any entity: `GET /v1/timeline/{entity_type}/{entity_id}`
- MCP tool: `cortex_timeline(entity_id, from, to, granularity)`
- Dashboard BitemporalTimeline view already scaffolded — wire it to real data

**C2 — Cross-project aggregation (spec 34 implementation)**
- Org-level aggregate queries: "Open risks across all projects", "Decisions made this quarter", "Who is working on what?"
- `GET /v1/org/summary`, `GET /v1/org/risks`, `GET /v1/org/decisions`
- MCP tool: `cortex_org_query(intent, filters)`

**C3 — Pre-thinking for business context**
- Add retrieval dimensions to bundle assembly: project context, team context, timeline position, relevant policies
- When the user is working on a non-code task, inject relevant business entity context
- Pluggable retrieval strategy: connectors register retrieval dimensions with the bundle assembler

**C4 — Multi-tenant isolation**
- Per-organization namespacing (not just per-repo)
- `org_id` in every event envelope and Nexus node
- Scoped queries: all backends filter by org_id when present
- Auth: JWT with org claim; per-org API keys

**C5 — Company policy governance**
- Extend law registry to cover business policies (not just coding rules)
- Examples: "decisions above $50k require CFO approval", "client data must not leave EU region", "all architecture changes require ADR"
- Law evaluator checks business operations, not just code tool calls
- Trust score extends to business decision quality

**C6 — Person knowledge graph**
- `Person` nodes with expertise areas, project history, decision history
- `(:Person)-[:EXPERT_IN]->(Topic)` derived from session/document analysis
- "Who knows most about the payment system?" answerable from graph
- Org chart representation: `(:Person)-[:REPORTS_TO]->(Person)`

---

### Phase D — Product Polish (ongoing)

**D1 — Dashboard auth and multi-tenant UI**
- Per-org login; scoped views; connection switching

**D2 — Mobile/web client**
- Today Cortex is desktop-only (Electron). A web client extends reach to non-developer users.
- Key for company knowledge use case: non-developers (executives, PMs, clients) need access.

**D3 — Notification and alerting**
- Law violations → Slack notification
- Milestone passed → team notification
- Decision superseded → affected team notified

**D4 — Search quality continuous improvement**
- A/B test RRF vs pure vector vs pure keyword for business vs code queries
- Feedback loop: thumbs up/down on query results → training signal

---

## Part 5 — Strategic Assessment

### 5.1 Cortex's Unique Position

Cortex occupies a position few other tools hold: it captures **the process of thinking and decision-making**, not just the outputs. Most knowledge management tools (Confluence, Notion, GitHub) capture static artifacts. Cortex captures the *dynamic flow* — what questions were asked, what tools were invoked, what decisions emerged, what was contradicted later.

This process memory is more valuable than static artifact storage for two reasons:
1. **It explains why**, not just what. "We chose Nexus because we evaluated three graph DBs and Nexus had the best Cypher support for our query patterns" — this reasoning lives in Cortex sessions, not in any doc.
2. **It detects drift**. When a new decision contradicts an old one, Cortex can surface the conflict before it becomes a production incident.

For general company use, this same principle applies: capturing business decisions, project discussions, and case resolutions — with the reasoning attached — creates an institutional memory that survives team turnover.

### 5.2 Critical Enablers vs. Nice-to-Haves

**Cannot become a general tool without (Phase A + B blockers)**:
- Generic document ingestion (B1) — without this, Cortex only knows about AI coding sessions
- Business entity model (B5) — without Person/Project/Case, org queries return nothing
- Timeline API (C1) — "what happened when" is the most common business query pattern

**High leverage additions (Phase A)**:
- Governance engine (A2) — makes the platform verifiably useful, not just informational
- Retrieval quality benchmark (A5) — lets the team prove the platform is working
- Deep analysis (A4) — Sonnet cross-event synthesis is where Cortex becomes non-substitutable

**Nice to have (Phase C–D)**:
- Multi-tenant auth, mobile client, notification system — these matter for scale and UX, but don't gate the core value

### 5.3 What Makes Cortex Different from Alternatives

| Tool              | What it does well                       | What Cortex adds                          |
|-------------------|-----------------------------------------|-------------------------------------------|
| Notion/Confluence | Static docs + wikis                     | Process memory + contradiction detection  |
| GitHub Issues     | Code task tracking                      | Cross-session reasoning + decision lineage|
| Linear/Jira       | Project management                      | AI session context + decision capture     |
| Obsidian/Logseq   | Personal notes + graph                  | Auto-capture (no manual entry) + multi-agent |
| Langchain/LlamaIndex | RAG over documents                  | Not just retrieval — capture + governance |
| ChatGPT Memory    | Per-user conversation memory            | Organization-level + code intelligence + governance |

Cortex's moat is **auto-capture + cross-session synthesis + governance**. Other tools require manual input. Cortex captures automatically from AI tool hooks.

### 5.4 Risk Register

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| Nexus #12 sustained-write stall not fixed upstream | Medium | High — blocks semantic-edge projection indefinitely | Implement rate-limited projection scheduler as client-side mitigation |
| Vectorizer SDK drifts accumulate | Medium | Medium — reduces embedding coverage | Implement post-upsert verification + escalate to Vectorizer team |
| MCP surface too narrow for business queries | High now | High — limits general use | Phase A7 + B timeline (high priority) |
| No retrieval quality measurement → silent degradation | High | Medium | A5 (recall benchmark) is P1 |
| Governance engine never shipped → platform loses credibility | Medium | High | A2 is blocked only on engineering, not external deps |
| Bootstrap covers 3/17 repos → cognitive coverage is patchy | High now | Medium | Multi-repo orchestrator + automated fan-out |
| No business entity model → company knowledge queries impossible | Certain (currently) | High for general use | Phase B5 is the entry gate |

---

## Part 6 — Implementation Sequence (One-Paragraph Summary)

**Short term (next 4–8 weeks):** Close the data loop — fix Nexus #12 or implement the rate-limiter workaround, complete Meilisearch fan-out, build the governance engine MVP (spec 13–14), wire the Deep Analysis workflow (spec 15), and ship a retrieval quality benchmark. After this, the code-assistance platform is complete and measurably working.

**Medium term (2–4 months):** Build the document ingestion adapter (`POST /v1/ingest`), expand the entity model with business types (Person, Project, Case, Document), implement Timeline API (spec 33), and ship one high-value connector (Linear or Slack). After this, Cortex ingests general company content and can answer "what happened when" queries.

**Long term (4–8 months):** Cross-project aggregation (spec 34), company policy governance (law registry for business rules), person knowledge graph (expertise map), multi-tenant auth, and a web client for non-developer users. After this, Cortex is a genuine company intelligence platform.

---

## Appendix A — MCP Tool Completeness Matrix

For reference: current MCP tools vs. what a complete general-assistance surface would look like.

| Tool                          | Available today | Phase |
|-------------------------------|-----------------|-------|
| `cortex_pre_thinking`         | ✅              | —     |
| `cortex_query`                | ✅              | —     |
| `cortex_decision_get`         | ✅              | —     |
| `cortex_topic_get/drill/neighbors/synthesize` | ✅ | —   |
| `cortex_graph_query`          | ❌              | A7    |
| `cortex_law_evaluate`         | ❌              | A2    |
| `cortex_session_timeline`     | ❌              | A     |
| `cortex_decision_search`      | ❌              | A     |
| `cortex_timeline`             | ❌              | C1    |
| `cortex_org_query`            | ❌              | C2    |
| `cortex_entity_history`       | ❌              | C1    |
| `cortex_person_context`       | ❌              | C6    |
| `cortex_project_summary`      | ❌              | B/C   |
| `cortex_case_history`         | ❌              | B3    |
| `cortex_risk_summary`         | ❌              | C2    |

---

## Appendix B — Spec Implementation Status (June 2026)

| Spec | Name                       | Status    | Notes                                              |
|------|----------------------------|-----------|----------------------------------------------------|
| 01   | Event schema               | 🟢 Done   | All 12 kinds validated                             |
| 02   | Storage layout             | 🟢 Done   | Parquet + zstd + SQLite metadata                   |
| 03   | Local stack                | 🟢 Done   | docker-compose, all backends                       |
| 04   | Cortex core                | 🟢 Done   | Types, redactor, ingestion router                  |
| 05   | Classifier                 | 🟢 Done   | Haiku + static fallback; budget control            |
| 06   | Embedder                   | 🟢 Done   | Tree-sitter chunking; Vectorizer client            |
| 07   | Graph writer               | 🟢 Done   | Nexus Cypher; semantic-edge projection (gated)     |
| 08   | Fulltext indexer           | 🟢 Done   | Meilisearch; fan-out incomplete                    |
| 09   | Bootstrap CLI              | 🟢 Done   | Multi-repo; incremental; promotion patterns        |
| 10   | Claude Code adapter        | 🟢 Done   | 4 hooks; daemon; session capture                   |
| 11   | Query API                  | 🟢 Done   | Hybrid RRF; scope; intent; Synap cache             |
| 12   | Pre-thinking injection     | 🟢 Done   | Bundle assembly; hook wiring                       |
| 13   | Laws DSL + detector        | 🟡 Draft  | Architecture specced; engine not built             |
| 14   | Governance engine          | 🟡 Draft  | Trust score specced; not wired                     |
| 15   | Deep analysis              | 🟡 Draft  | Sonnet precursor landed; full workflow open        |
| 16   | Dashboard                  | 🟡/🟢    | 21 views shipped; auth + law engine open           |
| 17   | Additional adapters        | 🟡 Draft  | Cursor/Codex/Gemini specced; none built            |
| 18   | Claude Code plugin         | 🟢 Done   | MCP bridge; hooks; commands                        |
| 20   | MCP tool surface           | 🟢 Partial| 7 tools; 8+ needed                                |
| 21   | Dashboard SSE              | 🟢 Done   | Live push for timeline + analysis                  |
| 22   | Fine-grained search        | ⚪ Open   | Cypher + Meili coordination                        |
| 24   | Producer trait             | 🟢 Done   | Event identity; idle-window session end            |
| 25   | Event identity             | 🟢 Done   | Replay dedup keys                                  |
| 26   | Cortex config              | 🟢 Done   | ADR-016 typed config                               |
| 27   | Consolidation              | 🟢 Done   | Session/Topic/DecisionTrace grains                 |
| 30   | Bitemporal schema          | ⚪ Draft  | valid_from/valid_to time dimensions                |
| 31   | Temporal classifier        | ⚪ Draft  | Time-scoped classification                         |
| 32   | Branches                   | ⚪ Draft  | Commit-scoped variant trees                        |
| 33   | Timeline API               | ⚪ Draft  | Bitemporal query surface                           |
| 34   | Cross-project axis         | ⚪ Draft  | Org-level aggregation                              |
| 35   | Temporal pre-thinking      | ⚪ Draft  | Time-scoped context injection                      |
| 36   | Temporal observability     | ⚪ Draft  | Freshness/divergence by time dimension             |
| —    | Document ingestion         | ❌ Missing | Not yet specced — needed for Phase B              |
| —    | Business entity model      | ❌ Missing | Not yet specced — needed for Phase B              |
| —    | Company policy governance  | ❌ Missing | Extension of specs 13–14 to business domain        |
| —    | Person knowledge graph     | ❌ Missing | Not yet specced                                   |
| —    | Connector framework        | ❌ Missing | Slack, Linear, Notion, Confluence connectors       |

---

*This analysis supersedes the 2026-04-28 state captured in files 01–10. For the original pipeline state and data quality baseline, see [02-pipeline-state.md](02-pipeline-state.md) and [03-data-quality.md](03-data-quality.md).*

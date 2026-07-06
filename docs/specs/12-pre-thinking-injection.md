# 12 — Pre-thinking injection

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 10, 11

## Goal

Turn the raw bundle returned by the query API (spec 11) into a **compact, well-shaped block of context** that the Claude Code adapter (spec 10) drops into the model's system prompt *before* the model plans its response. The goal is not "maximize context" — it is "give the model exactly the 3–5 things it needs to avoid repeating past mistakes, honor active laws, and reference the right decisions." This spec owns the heuristics, formatting, and budget.

## Scope

**In:**
- Adapter-side module (`cortex-adapters/claude-code/src/pre_thinking.rs`) that wraps `cortex-api /v1/query`.
- Scope-derivation heuristics from the user prompt + `cwd` + recent files.
- Bundle formatter (deterministic Markdown; no model-generated prose).
- Byte-budget enforcement (≤32 KB per `adapter.pre_thinking.max_bundle_kb`).
- Per-section caps (snippets N, decisions N, laws N) with fairness.
- Debug tracing: every bundle assembled carries a `query_id` so we can audit retrieval quality later.

**Out:**
- Query lanes (spec 11).
- Hook wiring (spec 10).
- Evaluation harness / offline scoring (Phase 2 retrieval-quality pass).
- Non-Claude-Code adapters (spec 17 copies this module with adapter-specific tweaks).

## Inputs / Outputs

### Input (from the adapter)

```rust
pub struct PreThinkingInput<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub user_prompt: &'a str,
    pub cwd: &'a Path,
    pub recent_files: &'a [RecentFile],   // from git status, TTL-cached 10 s (spec 10)
    pub budget: PreThinkingBudget,
}

pub struct RecentFile {
    pub path: PathBuf,
    pub status: FileStatus,                // modified | staged | untracked
    pub age_seconds: u64,
}

pub struct PreThinkingBudget {
    pub bundle_bytes: u32,                 // default 32 * 1024
    pub time_ms: u32,                      // default 600 (hook-budget bound)
}
```

### Output (string returned as `additionalContext`)

A deterministic Markdown block. Example:

```markdown
<!-- cortex: pre_change_context · query_id=01HY… · budget=32KB -->

## Active laws in this scope
- **LAW-012** (notable) — HNSW recall benchmarks must run before merge.
- **LAW-007** (critical) — Never pass `--no-verify` to `git commit` without explicit authorization.

## Topic card
[auth-rewrite] (rev 3, confidence 82%, age 5d, +2 ev)
JWT validation now consolidated behind a single middleware so token rotation lands deterministically without the prior 5-minute cache lag. The new flow short-circuits expired tokens at the gateway and refreshes session state through the SessionStore the dashboard reads.

### Evidence (2)
- decision:DEC-0042 (cited@rev=3, w=0.90)
- consolidation:cons-ses-aaa1 (cited@rev=3, w=0.75)

### Open contradictions (1)
- DecisionSupersession: DEC-0042 vs DEC-0001 (surfaced@rev=3)

## Consolidated context (3)
1. session/cons-ses-aaa1 · 2026-02-11 · ✓ · Auth refactor session — JWT cache TTL drop
2. topic/cons-top-bbb2 · 2026-01-04 · ✓ · HNSW ef_search tuning across the 2M-vector benchmark family
3. decision_trace/cons-dec-ccc3 · 2025-09-21 · ⚠ · Trace from DEC-0042 back through the predecessor benchmarks

## Recent decisions you should know about
- ✓ **DEC-0042 (accepted, 2026-03-05)** — Raise HNSW ef_search default to 128.
  Rationale: recall@10 held above 0.92 up to 2M vectors in benchmarks.
- ✗ **DEC-0031 (superseded, 2025-09-21)** — Keep ef_search=64 for low-latency profile.

## Similar past turns
1. ✓ 2026-02-11 — Claude Sonnet refactored `hnsw_search` to accept `ef` per-call.
2. ⚠ 2025-12-03 — Gemini benchmarked ef_search=128 and concluded it was safe up to 2M.

## Relevant snippets (3)
1. `Vectorizer/src/index/hnsw/mod.rs:hnsw_search` — current implementation with configurable `ef`.
2. `Vectorizer/docs/perf/hnsw.md#ef_search-tuning` — section on recall/latency tradeoffs.
3. `Vectorizer/benches/hnsw_recall.rs:bench_ef` — benchmark that drives LAW-012.

<!-- end cortex -->
```

Sections are always in the same order — phase 11r §5.4 reordered the matrix so the topic-card synthesis lane is the **top-priority context band** when fresh: **laws → active operator work (phase13g §4.4) → similar past sessions (phase13g §4.4) → ADR provenance (phase13g §4.4, conditional) → topic_cards → consolidated context → decisions → similar turns → past sessions (fallback only) → snippets → (optional) graph traversal sub-blocks → (optional) graph neighbours catch-all**. Sections with zero entries are omitted entirely (no empty headers). The trailing comment makes it easy to strip / diff in logs.

#### Phase13g §4 — grounding sections (active work + similar sessions + ADR provenance)

Three new sections feed off the phase13g MCP tools and sit between the laws block and the topic-card / consolidations block. Each renderer self-bounds via the section_caps byte budget below; when its input vec is empty the section is omitted entirely so an unwired or empty fetch degrades gracefully.

| Section                        | Byte cap                                | Source tool                  |
|--------------------------------|-----------------------------------------|------------------------------|
| `## Active operator work (N)`  | `section_caps::ACTIVE_WORK_BYTES = 1200`| `cortex_active_work`         |
| `## Similar past sessions (N)` | `section_caps::SIMILAR_SESSIONS_BYTES = 2000` | `cortex_similar_sessions` |
| `## ADR provenance (N)`        | `section_caps::ADR_PROVENANCE_BYTES = 800` | `cortex_decision_chain`   |

Combined the three caps consume 4 000 bytes — about 33 % of the existing 12 KB pre-thinking ceiling. The `section_caps_sum_stays_under_pre_thinking_ceiling` test pins the invariant.

Dispatch policy (phase13g §4.3):

- `cortex_active_work` fires unconditionally on every `pre_change_context` and `change_landing_context` turn.
- `cortex_similar_sessions` fires when `formatter::should_fetch_similar_sessions(user_prompt)` returns true — i.e. when the trimmed user prompt is longer than `SIMILAR_SESSIONS_QUERY_FLOOR_CHARS = 16` characters. Bare tool pings (under the floor) skip the fetch because the resulting embedding produces noise hits.
- `cortex_decision_chain` fires once per unique ULID returned by `formatter::extract_adr_event_ids(user_prompt)` plus the union of ULIDs found in `response.laws_active`, `response.results.decisions`, and `response.results.similar_turns`. The extractor walks 26-char Crockford-safe windows and rejects any window buried in a longer alphabet run, deduplicating in first-seen order.

Each renderer truncates with `… (truncated; see <tool> for the full <unit>)` so the caller knows which MCP tool to invoke if it wants the rest.

Phase11j §4.2 — when the consolidations lane returns ≥ 1 hit, the renderer emits the **Consolidated context** section and **suppresses the legacy Past sessions block** (§4.3 fallback). When the lane returns zero hits, the renderer falls back to the original past-sessions section so cold caches degrade gracefully.

Phase11r §5.4 — when the topic-card lane returns ≥ 1 hit AND the card is **not stale** (see staleness contract below), the topic-card section leads the context band, ahead of consolidations. When the card **is stale**, the order flips: consolidations render first, then the topic card section is downgraded to its old position with a `> stale-topic-card: <reason>` advisory line stamped under the heading. A card is stale when `confidence < 0.6` OR (`synthesis_age_d > 30` AND `events_since_last_rev > 0`).

#### Topic card section (phase11r §5.3)

```text
## Topic card
> stale-topic-card: <reason>            ← only when staleness fires
[<topic_slug>] (rev N, confidence X%, age Yd, +Z ev)
<synthesis preview clipped at section_caps::TOPIC_CARDS_BYTES / 2>

### Evidence (N)
- <kind>:<id> (cited@rev=N[, w=…])

### Open contradictions (N)
- <kind>: <evidence_a> vs <evidence_b> (surfaced@rev=N)
```

Section budget: 1 400 bytes (`section_caps::TOPIC_CARDS_BYTES`). Cap: 1 card (`section_caps::TOPIC_CARDS`) — the renderer only ever surfaces the top-priority card, falling back to the consolidation lane for additional context. Evidence is clipped to top-5 by `cited_at_rev` desc; only contradictions with `status == Open` land in the section. The advisory line is omitted when the card is fresh.

#### Graph traversal sub-blocks (phase11k §6.2)

The graph layer's static-extraction pass (phase11k §1-§5) materialises three high-signal edge classes the renderer surfaces under named sub-blocks rather than the generic `## Graph neighbours` heading:

- `## Connected files (via IMPORTS_FILE)` — tier-2 / tier-3 imports the touched artifact resolves into. Useful for blast-radius questions ("if I edit this file, what else needs a re-test?"). Backed by the `cypher/blast_radius.cypher` template's `IMPORTS_FILE*1..2` walk.
- `## Documented under (via DOCUMENTED_BY)` — Rust intra-doc backlinks the markdown analyzer extracted. Surfaces every `:DocSection` that documents a touched symbol. Backed by `cypher/code_callers.cypher` (one-hop direction reversal applied at the renderer).
- `## Cited from (via CITES)` — ADR / Decision / Analysis chain rooted at a touched node. Useful for design-trace questions ("trace the design behind decision X"). Backed by `cypher/doc_trail.cypher`'s `CITES*1..4` chain.

Worked example (spec → file → symbols → callers chain):

```markdown
## Cited from (via CITES)
- Decision:DEC-0042 -> Spec:docs/specs/07-graph-writer.md (hops=1)
- Spec:docs/specs/07-graph-writer.md -> Artifact:cortex|crates/cortex-workers/src/graph/mapper.rs|sha256:abc (hops=2)

## Connected files (via IMPORTS_FILE)
- Artifact:cortex|crates/cortex-workers/src/graph/mapper.rs|sha256:abc -> Artifact:cortex|crates/cortex-workers/src/graph/identity.rs|sha256:def (hops=1)
- Artifact:cortex|crates/cortex-workers/src/graph/mapper.rs|sha256:abc -> Artifact:cortex|crates/cortex-workers/src/graph/patch.rs|sha256:ghi (hops=1)
```

Edges whose relation does not match one of the three named classes fall through to the catch-all `## Graph neighbours` block so nothing the orchestrator surfaces is silently dropped. Each sub-block honours the `graph_cap` setting independently of the others; a budget squeeze drops the whole graph section before any other content.

#### Outcome glyphs (phase11i §4.2)

Every turn line and decision line carries a single outcome glyph between the row marker and the body so readers can scan the section for green / red flags without parsing each line:

- `✓` — positive outcome. Turns: classifier `outcome = "success"`. Decisions: `status = "accepted"`.
- `✗` — negative outcome. Turns: `outcome ∈ {"error", "failed", "failure"}`. Decisions: `status ∈ {"superseded", "deprecated", "rejected"}`.
- `⚠` — neutral / unknown. Turns: any other outcome (`partial`, `blocked_by_law`, missing tag). Decisions: `proposed`, `draft`, or any unrecognised status.

The neutral glyph is the renderer's default, so a missing-outcome regression upstream still produces a row with exactly one glyph rather than a column-shape skew.

#### Past sessions (phase11i §4.1)

A new section surfaces the top sessions whose centroid embedding is most similar to the current query. Each row is one line:

```text
N. <session_id> — <YYYY-MM-DD> · "<first user prompt clipped to 80 bytes>" · <turn_count> turn(s)
```

Caps: top **3** sessions by centroid similarity, each prompt clipped to **80 bytes** on a UTF-8 boundary. The section is omitted entirely when the upstream surfaces no past sessions, so cold caches degrade silently. Phase11j §4.3 — also suppressed whenever the **Consolidated context** section has ≥ 1 entry; the consolidations are the higher-fidelity replacement and rendering both at once would surface the same context twice.

#### Consolidated context (phase11j §4.2)

Replaces **Past sessions** when the consolidations lane returns ≥ 1 hit. Each row is one line:

```text
N. <grain>/<consolidation_id> · <YYYY-MM-DD> · ✓|✗|⚠ · <title clipped to one line>
```

Where `grain ∈ {session, topic, decision_trace}` mirrors `ConsolidationPayload.grain`, the date is the consolidation's `ts` formatted as `YYYY-MM-DD` (em-dash when the upstream did not supply one), the glyph is the dominant outcome (`✓` success, `✗` error / failure, `⚠` partial / blocked / unknown — same vocabulary the Similar past turns section uses), and the title is the consolidation's pre-clipped one-line title (`title` is constrained to ≤ 80 chars by the spec-11j payload validator, so the renderer just trims trailing whitespace).

Caps: top **3** consolidations by similarity (configurable via `FormatOptions.consolidations_cap`). The section sits between **Similar past turns** and **Past sessions** in the section order. The section is omitted entirely when the upstream returns no consolidations; the **Past sessions** fallback then runs as before.

## Design

### Pipeline

```
user_prompt + cwd + recent_files
        │
        ▼
  scope_derive()  ──────────▶   QueryRequest (spec 11)
        │                                │
        │                                ▼
        │                         cortex-api /v1/query
        │                                │
        │                                ▼
        └──────▶ bundle_format() ◀─── QueryResponse
                        │
                        ▼
           clip_to_budget() + audit()
                        │
                        ▼
                   additionalContext
```

### `scope_derive`

Maps `(user_prompt, cwd, recent_files)` → `scope`:

1. **repo** — basename of the nearest ancestor containing `.git/` (or the `cortex.toml` `cortex.id` override).
2. **files** — union of:
   - `recent_files` (age < 5 min)
   - files mentioned verbatim in the user prompt (shell-glob-like regex; bounded to 16 candidates)
3. **topics** — phase10h: derived from the file extensions in
   `recent_files` + prompt-mentioned paths via
   [`topic_for_path`](../../crates/cortex-pre-thinking/src/scope.rs).
   `.rs` / `.py` / `.go` / `.ts` / etc. → `code`,
   `.md` / `.rst` / `.txt` → `docs`,
   `.toml` / `.yaml` / `.json` / `.ini` → `config`.
   Unknown extensions surface no topic so the orchestrator's
   filter stays permissive. The result is deduplicated and
   lowercased.
4. **since** — `None` in v1; we want all relevant history.

#### Scope inference (phase10h)

The classifier stamps the same canonical topic vocabulary on
every event (see [spec 05 §Topic vocabulary](./05-classifier.md));
inferring `topics` here scopes the orchestrator's lane filter
to the corpus the user is most likely asking about — without the
agent having to spell it out. The inference is best-effort: a
prompt that mentions only `README.md` in a Rust repo derives
`topics: ["docs"]` (from the file mention) on top of
`["code"]` (from a recent `.rs` edit), so the bundle blends
both corpora when the user's question genuinely spans them.

The full scope filter contract is documented in
[spec 11 §Scope filter contract (phase10h)](./11-query-api.md#scope-filter-contract-phase10h).

If `repo` can't be resolved, the adapter issues the query with repo-less scope and accepts coarser results.

### `intent` selection

Rule-based (no ML). The `user_prompt` is classified by cheap case-insensitive substring match. Phase6d expanded the table with the `Explain` intent + richer keyword coverage on the existing four; the selector now also returns the matched keyword (`MatchedIntent { intent, trigger }`) so the audit envelope can record `intent_trigger` for routing telemetry.

| Signal in prompt                                                                                                                  | Intent                | Plan summary |
|-----------------------------------------------------------------------------------------------------------------------------------|-----------------------|--------------|
| `how does`, `what is`, `what's`, `explain`, `show me`, `where is`, `where does`, `find usages`, `find references`, `look up`, `definition of` | `explain` (phase6d)   | vector + keyword on `code`+`docs` (k/limit capped at 8); **no** decisions / laws / similar-turns / graph overlays. Vector-heavy 60/40 split. Closes F-006. |
| `why did we pick`, `why do we use`, `history of`, `why did`, `why do`, `who decided`, `should we`, `why is`                       | `decision_lookup`     | vector + keyword on `decisions` collection, supersession-chain graph leg, decisions overlay only |
| `have we seen`, `did we hit`, `stuck`, `keep failing`, `keeps failing`, `kept failing`, `doesn't work`, `doesnt work`, `isn't working` | `similar_problems`    | vector on `turns` + analysis-decision graph, `similar_turns` overlay |
| `is this allowed`, `am i allowed`, `would this violate`, `can i `, `is it allowed`, `blocked`, `permitted`                        | `law_check`           | keyword on `governance` + law-violation graph, violations overlay |
| `refactor`, `modify`, `rewrite`, `change`, `edit`                                                                                  | `pre_change_context`  | three-lane fan-out + full overlay set (default) |
| (no rule matched)                                                                                                                  | `pre_change_context`  | safe default; `intent_trigger` lands as `null` on the audit envelope |

Order of evaluation matters: `explain` fires *before* `decision_lookup` so a prompt like "explain why we picked X" routes navigationally (the user wants to read code/docs, not consult an ADR). The `pre_change_context` keywords stay last because their verbs are common.

`pre_change_context` is the safe default — it pulls the broadest mix.

#### Audit envelope

Every audit envelope carries:

- `intent` — the resolved intent label (`explain`, `decision_lookup`, …).
- `intent_trigger` — the keyword that fired (`Some(&'static str)` from the rule table) or `null` when the prompt fell through to the default.

Closes [F-006 in `docs/analysis/relevance/01-findings.md`](../analysis/relevance/01-findings.md).

### Query rewriting

Phase6f inserts a single rewrite step **after** intent selection and
**before** the orchestrator's per-lane fan-out. The rewriter sees
the user prompt + selected intent, and produces distinct
`vector_query` / `keyword_query` strings that are stamped onto each
lane request and the audit envelope. Three implementations ship —
deterministic noun-phrase strip (default), Sonnet rewrite (opt-in,
with cache + fallback), passthrough (kill-switch). Selected via
`CORTEX_QUERY_REWRITER`. Full contract in
[spec 11 §Query rewriting](./11-query-api.md#query-rewriting-phase6f).

### Budget-aware section caps

Per section, soft caps:

| Section          | Max entries (default) | Max bytes per entry |
|------------------|-----------------------|---------------------|
| Laws             | 10                    | 256                 |
| Decisions        | 5                     | 512                 |
| Similar turns    | 5                     | 256                 |
| Snippets         | 5                     | 1 024               |
| Graph neighbors  | 0 (off by default)    | 256                 |

After formatting, the total is measured. If it exceeds `budget.bundle_bytes`:

1. Drop graph neighbors (if present).
2. Trim snippets to their `why` + first 3 lines of `text`.
3. Halve the snippets count.
4. Halve the similar-turns count.
5. Truncate decision bodies to 160 chars.
6. As a last resort, drop snippets entirely.

Never drop **laws** — active laws are load-bearing; better to drop everything else than to ship a prompt that silently skips a blocking rule.

### Deterministic formatting

- No templating engine — pure Rust string concatenation with fixed section order.
- Markdown is stable across runs (same input → byte-identical output).
- `query_id` is injected in the leading comment for auditability (spec 11 audit stream correlation).

### Sources (phase10e)

The bundle pulls from these corpora, in addition to the
existing snippet / decision / similar-turn / graph-neighbour
sections:

- **Knowledge** — pattern / anti-pattern entries from
  `cortex.knowledge.fp32` + `cortex_knowledge`. Routed to the
  bundle whenever `intent ∈ {pre_change_context,
  decision_lookup}` so the agent re-reads the canonical
  patterns + anti-patterns before acting on a related change.
- **Learnings** — implementation insights from
  `cortex.learning.fp32` + `cortex_learnings`. Same intent
  routing as knowledge — these were written specifically
  because someone made a mistake worth not repeating, so they
  belong front-and-centre when the agent is about to make a
  change.

Both corpora are populated by the bootstrap walker
([`crates/cortex-cli/src/bootstrap/walker.rs`](../../crates/cortex-cli/src/bootstrap/walker.rs))
recursing into `.rulebook/knowledge/**` and
`.rulebook/learnings/**`. Spec 02
[§Knowledge + Learnings corpus (phase10e)](./02-storage-layout.md#knowledge--learnings-corpus-phase10e)
is the cross-store contract.

### Snippet section layout (phase10b)

Each snippet renders as `<header><body>`. The header carries
identifying context; the body carries the projected source text.

- **Header** — `` `repo/path:symbol` — <why> `` when both a real
  symbol AND a `why` blurb are present. Drops the `:symbol`
  segment when no symbol is present (the orchestrator strips
  event-kind labels before they reach the wire — see
  [spec 11 §phase10b](./11-query-api.md#phase10b--body-capture--pathtext-separation)).
- **Body** — full or slimmed `text`, indented under the header.
  When `Snippet.body_truncated = true` (no body indexed inline),
  the body block is replaced by a `(body not indexed inline)`
  cue stamped onto the header so the agent does NOT see the
  path masquerading as the file contents.

This closes the audit-flagged `path:artifact — \n   path`
rendering. The pre-thinking budget is now spent on actual prose /
code instead of an `ls`-grade directory listing.

### In-session capture (phase10j)

The bundle is read-only by construction — it surfaces what
`cortex-api` already indexed. When an agent wants the next
pre-thinking call to see a fact learned mid-session, it MUST write
the fact through the
[`cortex_capture_memory` MCP tool](18-claude-code-plugin.md#cortex_capture_memory),
which POSTs a canonical `kind=memory|knowledge|learning` envelope to
`/v1/ingest` on `cortex-api`. The proxy validates body size (≤ 8 KiB),
stamps `event_id`, and forwards to `cortex-ingestion`. The next
pre-thinking call can then surface the captured envelope through the
free-search lane just like any other indexed event.

Without this surface the only path back into the live lane is
`rulebook_memory_save`, which writes the on-disk Rulebook store and
NOT the lane that pre-thinking reads — captured knowledge would stay
invisible to the next bundle. The MCP capture tool is the bridge.

### Error handling (fail-open)

| Failure                       | Response                                              |
|-------------------------------|-------------------------------------------------------|
| `scope_derive` fails          | Issue query with `scope = {}`; still useful            |
| `cortex-api` timeout (>600 ms) | Return empty string; session unaffected (spec 10 rule) |
| `cortex-api` 5xx              | Return empty string; log + metric                      |
| 0 results in the response      | Return empty string (not an empty header block)        |
| Any formatter panic            | Return empty string; never crash the daemon            |

### Observability

```
cortex.prethink.calls.total        counter, labels: intent
cortex.prethink.bundle.bytes       histogram
cortex.prethink.sections.count     histogram, labels: section
cortex.prethink.truncation.applied counter, labels: step (1..6)
cortex.prethink.latency_ms         histogram
cortex.prethink.empty_bundle       counter  // 0-result responses
cortex.prethink.timeouts           counter
```

Every call emits a span with `query_id`, `intent`, `scope_hash`, `bundle_bytes`, `sections_included`.

**Operator-visible health surface (phase26e §2/§3).** The pre-thinking
`BundleCache` and the `cortex.prethink.latency_ms` histogram are process-local
to the **adapter daemon** (the long-lived `cortex-adapter-claude` that holds the
`SyncClient`). The cortex-api `/v1/health/pre-thinking` endpoint runs in a
separate process with an unwired source, so it cannot see them. Instead the
daemon `/healthz` exports them in its `extras` (→ cortex-api `/v1/health`
`cortex-adapter` subsystem extras):

- `pre_thinking_cache_hit_total` / `pre_thinking_cache_miss_total` — live
  BundleCache hit/miss counts (a repeated identical query within the 60 s TTL is
  a hit).
- `pre_thinking_latency_ms` `{count, p50, p95, p99}` — the TRUE bundle-assembly
  latency. This is distinct from the GUI dashboard's legacy `pre_thinking_p95_ms`
  series, which is the p95 of generic envelope `duration_ms` and so reflects
  unrelated long-running tool_calls (phase26d gap C). **`phase26f §3.1`** adds
  the honest counterpart: `cortex-api`'s `OverviewBody.pre_thinking_assembly_p95_ms`
  (see [spec 16 — Dashboard](./16-dashboard.md), `/v1/dashboard/overview` series
  block) reads `extras.pre_thinking_latency_ms.p95` from this same `/healthz`
  surface via `gather_subsystem_extras()`, defaulting to `0` when the adapter is
  unreachable or has recorded no pre-thinking calls yet. The GUI's "Pre-thinking
  P95" tile now headlines this field; the legacy `pre_thinking_p95_ms` series is
  kept (additive, no breaking change) but relabeled as envelope tool/agent
  durations, not pre-thinking latency.

## Acceptance criteria

- [ ] Given a user prompt "refactor hnsw_search to take ef per call" in the Vectorizer repo, `scope_derive` produces `repo=Vectorizer, files=[src/index/hnsw/mod.rs]`, `intent=pre_change_context`.
- [ ] Given a 3-KB response with 2 laws, 3 decisions, 4 snippets, the formatter emits a bundle with all four sections in fixed order and size < 4 KB.
- [ ] Budget enforcement: artificial response of 80 KB is clipped to ≤32 KB; clip steps execute in the documented order; laws section is preserved.
- [ ] Empty-result response → empty string returned; counter `prethink.empty_bundle` increments.
- [ ] Timeout: forced 800 ms API latency (budget=600 ms) → empty string, no partial bundle, counter `prethink.timeouts` increments.
- [ ] Intent selection: a prompt containing "why did we pick 128?" maps to `decision_lookup`.
- [ ] Deterministic output: identical inputs produce byte-identical bundles across 1 000 runs.
- [ ] `query_id` is present in the leading comment and matches the Cortex audit stream entry.
- [ ] Truncation: a snippet `text` of 5 KB is trimmed to 1 024 bytes in the bundle; original length is preserved in debug logs.
- [ ] When `recent_files` is empty, the query still issues (repo-scope only) and returns non-empty results on the bootstrap corpus.
- [ ] Laws are **never** dropped: a bundle request with 20 active laws keeps 10 (the cap) and drops snippets/decisions/turns first to fit the budget.
- [ ] Unit test: formatter round-trip for a fixture response is stable byte-for-byte.

## Decisions

1. **Rules, not a model, to pick intent.** A small rule table is fast, debuggable, and predictable. We graduate to a model only if offline eval shows >5% precision gap.
2. **Laws are load-bearing — never trim.** Other sections can shrink or disappear; laws stay.
3. **Fixed section order, no prose.** The model relies on structural cues more than stylistic ones; a stable, scannable layout is better than natural-language "Here are the things I found…".
4. **Empty-result → empty bundle.** Injecting "No relevant context found." would train models to ignore the block. Silence is more honest.
5. **No model-generated summaries at read time.** Summaries already exist (classifier, spec 05). Re-summarizing here would add latency and non-determinism.
6. **Per-section caps, not a global ranker.** Simpler and avoids pathological bundles dominated by one section.
7. **`query_id` in a comment.** Survives any Markdown pass-through and lets us correlate bundle quality with retrieval audit later.

## Open questions

1. **Intent routing via an MCP tool.** Should the model pick its own intent (a dropdown) instead of the adapter guessing? Leaning no (UX cost, latency) but revisit if intent mismatch shows up as the dominant failure mode.
2. **Adaptive budgets.** A 32-KB cap is a hunch. Once we have eval data, tune per intent (e.g., `similar_problems` wants more snippets).

## Fail-open contract + circuit breaker (phase14e)

Every call to [`cortex_pre_thinking::pipeline::run_with_breaker`] guards its upstream `cortex-api` call through a shared [`cortex_pre_thinking::breaker::Breaker`]:

```
Closed → 5 fails / 60s window → Open
Open   → 30s cooldown          → HalfOpen
HalfOpen probe success         → Closed
HalfOpen probe failure         → Open
```

Defaults: `threshold = 5`, `window = 60s`, `cooldown = 30s`. Operator overrides land via `BreakerConfig`.

### Fail-open dispatch

Every fail-open path (timeout, network, internal, or breaker-open short-circuit) emits:

1. A `tracing::warn!` line with structured fields `session_id`, `turn_id`, `intent`, `reason`, `new_state`. Scrape this to alert on outages.
2. A `cortex_pre_thinking_fail_open_total{reason}` metric increment. Reasons: `timeout`, `network`, `unauthorised`, `internal`, `breaker_open`.
3. A bundle sentinel: the empty bundle is replaced with `<!-- cortex: timeout reason=<reason> [query_id=<id>] -->`. The HTML-comment shape survives markdown rendering without being treated as content; downstream models can distinguish "context retrieval failed" from "no context matched" by grepping for the prefix.

The breaker itself emits a structured WARN log on every state transition (`closed → open`, `open → half_open`, `half_open → open`, `half_open → closed`) so the scrape pipeline can alert without polling.

### Health endpoint

`GET /v1/health/pre-thinking` returns:

```json
{
  "breaker_state": "closed",
  "failures_in_window": 0,
  "fail_open_total": { "timeout": 3, "breaker_open": 1 },
  "fail_open_sum": 4
}
```

`breaker_state` is one of `closed` / `open` / `halfopen`. `failures_in_window` is the current breaker-window failure count (resets on window roll-over). `fail_open_total` is the per-reason counter since process boot; `fail_open_sum` is the convenience total.

Default state ships an `UnwiredPreThinkingHealthSource` that returns `closed` + zero counters. Production wires a [`cortex_pre_thinking::health_source::LivePreThinkingHealthSource`] over the same `Arc<Breaker>` + `Arc<Metrics>` the pipeline guards calls through. The live source lives in `cortex-pre-thinking` (not `cortex-api`) because `cortex-pre-thinking` already depends on `cortex-api` — wiring the reverse would create a circular dep.

### Operator playbook

| Symptom | Action |
|---|---|
| `fail_open_sum` climbing + `breaker_state = closed` | upstream `cortex-api` flaky but below threshold; check `fail_open_total.timeout` vs `network` to bucket the cause. |
| `breaker_state = open` | breaker tripped; subsequent calls short-circuit instantly. Cooldown is 30 s. Investigate `cortex-api` health (cores, GC, vectorizer reachability). |
| `breaker_state = halfopen` | breaker probing recovery; the next call decides. |
| Bundle starts with `<!-- cortex: timeout reason=… -->` | confirms the model saw a fail-open. Grep transcripts for the prefix to count silent context losses. |

Live operator probe: `curl -s http://127.0.0.1:17000/v1/health/pre-thinking | jq`.

## Feedback + per-intent budget (phase14f)

The pipeline now collects two kinds of feedback so bundle quality becomes measurable and tunable.

### Explicit feedback

`POST /v1/pre-thinking/feedback`

```json
{
  "query_id": "01HXQUERY00000000000000A",
  "intent": "explain",
  "helpful": true,
  "files_cited": ["src/lib.rs", "docs/specs/12.md"],
  "rating": 5,
  "free_text": "clear and useful",
  "implicit_score": 0.83
}
```

- `query_id` is required + non-empty; the audit cross-check lands in a follow-up.
- `helpful` is required.
- `rating` ∈ [1, 5] when present.
- `implicit_score` ∈ [0.0, 1.0] when present (operator can pre-compute via `cortex_pre_thinking::implicit_feedback::detect_citation`).
- Idempotent: re-posting the same `query_id` overwrites the prior row (`upserted: false` in the response). Persists to `pre_thinking_feedback` SQLite table.

### Implicit feedback

`cortex_pre_thinking::implicit_feedback::detect_citation(reply, bundle_files)` returns a Jaccard overlap score in `[0.0, 1.0]` between (a) the file paths the bundle surfaced and (b) the file-shaped tokens in the model's first ~100-token reply window. The async detector stamps the score in `pre_thinking_feedback.implicit_score`; the explicit feedback row may carry an operator-supplied value to seed before the detector runs.

### Per-intent budget

`PreThinkingConfig.budget_per_intent` is a per-intent KiB cap. Defaults (F-005 spec table):

| Intent              | KiB |
|---------------------|-----|
| `pre_change_context`| 32  |
| `similar_problems`  | 32  |
| `explain`           | 24  |
| `decision_lookup`   | 24  |
| `coverage`          | 16  |
| `free_search`       | 16  |
| `law_check`         | 12  |

`PreThinkingConfig::budget_bytes_for(intent_label)` returns the per-intent entry × 1 KiB, falling back to `bundle_kb` when the intent is absent. Operator overrides via TOML + env (existing `cortex_config` pattern).

### Quality dashboard

`GET /v1/health/pre-thinking` carries additional fields:

```json
{
  "breaker_state": "closed",
  "failures_in_window": 0,
  "fail_open_total": { "timeout": 0 },
  "fail_open_sum": 0,
  "bundle_bytes_per_intent": {
    "explain": { "count": 10, "p50": 12000, "p95": 22000, "p99": 24000 }
  },
  "helpful_rate_per_intent": {
    "explain": { "helpful": 7, "unhelpful": 3, "rate": 0.7 }
  }
}
```

GUI view `Pre-Thinking Quality` (`gui/src/views/PreThinkingQuality.tsx`) renders the breaker banner + per-intent bundle-bytes table (p50/p95/p99) + per-intent helpful-rate table.

## Intent routing + rewriter cascade (phase14g)

### Intent rule ordering

`DEFAULT_RULES` is now grouped into three tiers, evaluated top-down with first-match-wins (F-002 fix):

1. **High-specificity compounds** — multi-token decision / debug / policy / change phrases (`why did we pick`, `decided to pick`, `chose to`, `we picked`, `rationale for`, `history behind`, `who decided`, `have we seen`, `regression on`, `fails intermittently`, `is this allowed`, `would this violate`, `policy says`, `rules forbid`, `violates law`, `going to refactor`, `about to change`).
2. **Explain compounds + medium tier** — navigational compounds (`how does`, `what is`, `show me`, `where is`, `find usages`, `definition of`) AND 2-token decision phrases (`why did`, `why do`, `why is`, `should we`, `can i `).
3. **Single-word fallbacks** — bare `explain` / `blocked` / `permitted` / `stuck` / `refactor` / `modify` / `rewrite` / `change` / `edit`. These only fire when the prompt carries no compound signal.

**Regression contract** (phase14g §1.4): `"explain why did we pick hnsw"` routes to `decision_lookup` (was `explain`). 5 fixture prompts per intent ship as tests in `intent_select::tests::*_fixtures_route_correctly`.

### Mismatch metric

`Metrics::intent_mismatch_total{(from, to)}` counter. Feedback recorder bumps it when a row marks `helpful = false` AND the corrected intent differs from the routed intent. `intent_mismatch_snapshot()` returns `[(from, to, count)]` sorted by count desc. Surfaced on `/v1/health/pre-thinking.intent_mismatch_top` and via `cortex-ops intent-stats [--since <window>] [--api-url URL] [--json]`.

### Query rewriter cascade

`cortex_pre_thinking::rewriter::cascade(query, intent, sonnet?, cache, metrics, config)` runs:

1. **Cache lookup** — SHA256(query + intent) → `RewriteCache` (TTL 24 h, cap 10 000 entries; oldest evicted past cap). Hit → return tagged `sonnet_cache_hit`.
2. **Sonnet** — invoke supplied [`SonnetRewriter`] under `CascadeConfig::sonnet_timeout` (default 800 ms). Success → cache + return `sonnet_hit`. `SonnetError::Timeout` → fall through tagged `sonnet_timeout`. `SonnetError::Other(...)` → fall through tagged `sonnet_error`. WARN logged with reason + intent.
3. **Deterministic fallback** — `deterministic_rewrite(query)` (lowercase + whitespace collapse). Tags `deterministic_fallback`. When no Sonnet backend is wired, every call uses this path.

Telemetry: every dispatch bumps `Metrics::rewriter_path_total{path}` so `/v1/health/pre-thinking.rewriter_path_total` + `cortex-ops intent-stats` render per-path cascade counts.

### Operator playbook additions

| Symptom | Action |
|---|---|
| `intent_mismatch_top` shows high `explain → decision_lookup` | look for compound decision phrase missing from `DEFAULT_RULES` — add a top-tier rule that pre-empts the bare `explain`. |
| `rewriter_path_total.sonnet_timeout` rising | upstream Sonnet flaky; check `SONNET_TIMEOUT` (default 800 ms) and rate-limit headroom. |
| `rewriter_path_total.deterministic_fallback` ≈ 100 % | no Sonnet backend wired — by design when `CORTEX_PRE_THINKING_REWRITER=deterministic` (env default until operator enables cascade). |

## Canary (phase15f)

### Default change

`CanaryConfig.enabled` defaults to `true` from phase15f onward (was `false`). Existing operator `cortex.toml` files that do not include `[canary]` or include it without `enabled` will automatically pick up the new default on next restart; no migration is needed.

### Pre-thinking health canary loop

A second background loop (`run_pre_thinking_health_canary_loop`) ticks every `canary.pre_thinking_health_secs` seconds (default `60`). On each tick it:

1. Calls `GET /v1/health/pre-thinking` on the running cortex-api.
2. Records the result in the `canary_runs` SQLite table (`cortex-storage::metadata`): `ts` (RFC-3339), `status` (`"ok"` or `"error"`), `latency_ms`, `error_message`.
3. On **two consecutive failures** emits a structured WARN that the Prometheus scrape pipeline picks up:
   ```
   tracing::warn!(target: "canary", consecutive_failures, last_error, "canary alarm")
   ```
4. Resets the consecutive counter on the first successful tick.
5. Trims rows older than 24 h each tick.

### Configuration

```toml
[canary]
enabled                   = true   # master switch (now on by default)
interval_secs             = 300    # IPC round-trip canary interval
deadline_secs             = 10     # IPC round-trip deadline
pre_thinking_health_secs  = 60     # pre-thinking health canary interval
```

Environment override: `CORTEX_CANARY_PRE_THINKING_HEALTH_SECS`.

### Operator playbook

| Symptom | Action |
|---|---|
| `"canary alarm"` WARN in log with `consecutive_failures >= 2` | pre-thinking health endpoint is returning errors; run `curl /v1/health/pre-thinking` to inspect breaker state. |
| No `canary_runs` rows in metadata DB | canary loop not started; check that `canary.enabled = true` in config and the loop is wired in `cortex-api`. |

## References

- Architecture §5.3 (query → context bundle), §8 (end-to-end example step 2).
- Spec 01 — Event schema (nothing direct; used via the query response).
- Spec 10 — Claude Code adapter (embeds this module; owns the hook budget).
- Spec 11 — Query API (response schema is this spec's input).
- Spec 13 — Laws DSL (laws content comes from here).
- Spec 14 — Governance engine (trust score could reweight caps later; not in v1).
- Spec 17 — Additional adapters (will copy this module with small surface differences).

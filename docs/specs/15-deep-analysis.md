# 15 — Deep Analysis workflow

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 11, 13

## Goal

Structured, auditable debate: the user (or an agent) opens an **Analysis** around a hard question. Cortex assembles the relevant prior context, orchestrates a panel of 2–5 AI agents, captures each round as first-class `Turn`s linked to the Analysis, then produces a **Decision** record that is indexed and citable by all future `similar_problems` queries. This is how "stuck" topics become institutional memory instead of recurring pain.

## Scope

**In:**
- `cortex-analysis` crate + subcommands under `cortex analysis ...`.
- Data model: `Analysis` node + linked `Turn`s + final `Decision`.
- Orchestration engine: per-round prompt assembly, parallel agent invocation, timeout handling.
- Context-pack builder: pulls prior Decisions, similar Analyses, linked Turns, and active Laws (via spec 11).
- Judge pipeline: rule-based or LLM-judge options; writes the final `Decision` with provenance.
- Cost guardrails (token/round caps).
- Resume + re-open semantics.
- REST endpoints for dashboard integration.

**Out:**
- Agent implementations (we use provider SDKs; no model owned by Cortex).
- Live-streaming UI (dashboard — spec 16).
- Cross-project analysis federation (future).
- Human-only debates (manual analyses can be typed directly via the dashboard; orchestration optional).

## Inputs / Outputs

### CLI

```
cortex analysis start "<question>"
  [--scope repo=X,files=...,topics=...]
  [--panel claude-opus,claude-sonnet,gemini-pro,gpt-5]
  [--rounds 3]
  [--judge auto|human|model:<name>]
  [--budget-usd 5.0]
  [--async]                     # return analysis_id and detach

cortex analysis show <analysis_id>        # pretty-print transcript + decision
cortex analysis list [--status open|resolved|archived]
cortex analysis resume <analysis_id>      # continue after a pause
cortex analysis close <analysis_id>       # force-close → judge runs
cortex analysis cancel <analysis_id>
```

### HTTP endpoints

```
POST /v1/analysis                        # create
GET  /v1/analysis/{id}
GET  /v1/analysis                        # list
POST /v1/analysis/{id}/round             # trigger next round (async mode)
POST /v1/analysis/{id}/close             # force-judge
DELETE /v1/analysis/{id}                  # cancel (soft; node remains with status=cancelled)
GET  /v1/analysis/{id}/stream            # SSE — live events for dashboard
```

### Analysis node (graph)

```jsonc
{
  "id": "01HYA...", "question": "Why does HNSW recall drop above 1M vectors?",
  "scope": { "repo": "Vectorizer", "topics": ["hnsw", "recall"] },
  "panel": ["claude-opus", "claude-sonnet", "gemini-pro"],
  "rounds_planned": 3, "rounds_completed": 2,
  "status": "in_progress",                 // open | in_progress | resolved | cancelled | timed_out
  "judge": { "mode": "model", "id": "claude-opus" },
  "opened_at": 1713369600000,
  "closed_at": null,
  "decision_id": null,
  "budget_usd": 5.0, "spent_usd": 1.32
}
```

Edges (spec 07 handles writes):

- `Analysis -[:DEBATED_IN]- Turn` (many)
- `Analysis -[:RESOLVES_TO]-> Decision` (one, once judged)

### Round turn shape

Each agent invocation per round produces a `Turn:analysis_round` event:

```jsonc
{
  "turn_id": "01HYB...",
  "analysis_id": "01HYA...",
  "round": 2, "role": "panelist",
  "model": "claude-sonnet",
  "prompt_sha": "sha256:...",              // for reproducibility
  "response": "...",                        // full response (redacted)
  "citations": ["DEC-0042", "Analysis:01HX3..."],  // model-declared
  "tokens_in": 4321, "tokens_out": 1204,
  "cost_usd": 0.04,
  "latency_ms": 3820
}
```

## Design

### Lifecycle

```
 start
   │
   ▼
 context_pack ← spec 11 `/v1/query intent=similar_problems`
   │
   ▼
 round_1: parallel panelist invocation with context_pack
   │
   ▼
 round_2: each panelist sees round_1 responses as additional context
   │         (deduplicated; citations preserved)
   ▼
 round_3: final position statements
   │
   ▼
 judge: rule-based or LLM
   │
   ▼
 Decision created; Analysis.status = resolved
```

Rounds are sequential; within a round, panelists run in parallel.

### Context pack

```rust
struct ContextPack {
    prior_decisions: Vec<Decision>,          // top-5 by relevance
    similar_analyses: Vec<AnalysisSummary>,  // top-3 resolved analyses
    active_laws: Vec<Law>,                   // in scope
    snippets: Vec<Snippet>,                  // top-10 code/doc
    budget_bytes: u32,                       // default 128 KB
}
```

Assembled by calling spec 11 with `intent=similar_problems` + scope. The context pack is **frozen** at Analysis start — every round sees the same base context, plus prior-round responses. Freezing is what makes the debate reproducible.

### Prompt template per round

```
You are panelist {model} in a structured analysis. Respond in ≤800 tokens.

## Question
{analysis.question}

## Scope
{analysis.scope}

## Prior context (unchanging across rounds)
{context_pack}

## Previous rounds
{round_1_responses_concat}
{round_2_responses_concat}
...

## Your job this round
{round_instructions}           // varies per round; see below

## Output format
Markdown with these sections:
- **Position** (1–3 sentences, your current best answer)
- **Reasoning** (evidence-linked; cite Decision IDs, Law IDs, file paths)
- **Confidence** (low | medium | high)
- **Open disagreements** (where you differ from prior panelists)
```

Round instructions:

| Round | Instruction                                                                          |
|:-----:|--------------------------------------------------------------------------------------|
| 1     | Present your initial position with reasoning. Do not address other panelists yet.     |
| 2     | Address each panelist's reasoning. Where you agree, say so. Where you disagree, explain. |
| 3     | Final position. State the answer you'd stand behind if asked to commit.                |
| N     | (custom — user can define their own final-round prompt)                                 |

### Judge

Two modes:

1. **`auto` (rule-based).** Picks the answer with highest **weighted citation count** across the transcript (ties broken by lowest `rounds_to_converge`). Writes a short Decision body: question + winning position + reasoning summary + transcript link. Fast, cheap, deterministic.
2. **`model` (LLM judge).** Asks a judge model to read the transcript and produce a structured Decision. Prompt is fixed; output parsed against a JSON schema; failure falls back to `auto`. Judge model is separate from panelists (default: the largest available model).

**`human` mode**: emits the transcript + a decision-draft scaffold; no Decision is created until the user posts one via the API. Useful when the analysis is hot and the user wants final say.

### Concurrency

- Panelists within a round invoked with `tokio::join_all`; per-agent timeout = `round_timeout_ms` (default 60 000).
- Rounds are strictly sequential — prevents cascading-retry issues and keeps transcripts comprehensible.
- Under `--async` mode, the CLI returns immediately; the analysis proceeds in the background (lifecycle hosted by the `cortex-analysis` daemon inside `cortex-api`).

### Cost guardrails

- `--budget-usd` is a hard cap. Before each round, the engine estimates `Σ(tokens_in × price_in + tokens_out_est × price_out)` and **skips** panelists that would push the total over budget.
- On budget exhaustion mid-round, the remaining planned rounds are truncated; judge runs on what we have.
- Per-model prices live in `cortex-analysis/pricing.toml`, sourced from provider public pricing. Stale prices surface as a warning.

### Redaction

The question itself passes through the static redactor before context is assembled (the question can contain pasted secrets — happens in practice). Responses are redacted before they enter the next round's prompt (prevents secrets from multiplying across agents).

### Failure modes

| Failure                                        | Handling                                                                 |
|------------------------------------------------|--------------------------------------------------------------------------|
| Panelist unreachable (transient)               | Retry once; if still failing, mark panelist as `absent` for the round     |
| Panelist timeout                               | Mark absent; round proceeds with remaining panelists                      |
| All panelists absent in a round                | Pause analysis (`status=paused`); surface alert                           |
| Judge fails (model mode) → parse error         | Fall back to `auto` judge; record `judge.fallback = true`                 |
| Budget exhausted                               | Truncate remaining rounds; close analysis with note; Decision if possible |
| Context pack assembly fails (spec 11 down)      | Analysis paused; retry on resume                                          |
| Cancellation mid-round                         | Partial round preserved; status `cancelled`; no Decision                 |

### Observability

```
cortex.analysis.started.total         counter, labels: scope_repo
cortex.analysis.rounds.total          counter, labels: status
cortex.analysis.round.latency_ms      histogram
cortex.analysis.panelists.absent      counter, labels: model
cortex.analysis.judge.latency_ms      histogram, labels: mode
cortex.analysis.judge.fallback        counter
cortex.analysis.cost.usd              histogram, labels: model
cortex.analysis.decisions.created     counter
```

## Acceptance criteria

- [ ] `cortex analysis start "…"` with 3 panelists and 3 rounds produces 9 `Turn` events, one `Analysis` node, one `Decision` node; `Analysis -[:RESOLVES_TO]-> Decision` edge exists.
- [ ] Context pack is identical across rounds (sha comparison); round-2 prompt includes round-1 transcript; round-3 includes both.
- [ ] Auto-judge on a golden transcript picks the expected winning position (golden set of 20 scored analyses).
- [ ] Model-judge on the same golden set parses successfully ≥90%; fallback to auto on parse errors.
- [ ] Human mode: no Decision is created until `POST /v1/analysis/{id}/decision` is hit; Analysis.status remains `awaiting_decision`.
- [ ] Budget cap: `--budget-usd 0.10` truncates rounds when estimate would exceed; final message records `truncated_for_budget=true`.
- [ ] Panelist absence: killing one panelist mid-round produces the other two's responses and proceeds; round marked `partial`.
- [ ] All-panelists-absent: analysis pauses, can be resumed with `cortex analysis resume`.
- [ ] Redaction: a synthetic token in the question is replaced with `[REDACTED]` in the context pack and in every panelist prompt.
- [ ] Citations: when a panelist response cites `DEC-0042`, the citation appears in `citations[]` on the Turn event.
- [ ] SSE stream: `GET /v1/analysis/{id}/stream` emits one event per round start / panelist response / judge decision.
- [ ] Async mode: `--async` returns `analysis_id` immediately; background progress verified via `cortex analysis show`.
- [ ] Cancellation: `cortex analysis cancel <id>` during round 2 ends with status `cancelled`, partial turns preserved, no Decision.
- [ ] Re-indexing: a resolved Analysis is retrievable via `/v1/query intent=similar_problems` within 30 s of close.
- [ ] Telemetry counters non-zero after a synthetic analysis.

## Decisions

1. **Context pack is frozen.** Variable context across rounds would make transcripts un-reproducible and audit impossible.
2. **Rounds are sequential, panelists parallel.** Best tradeoff between wall-clock and coherence. Parallel rounds would produce commentary-on-nothing.
3. **Judge is pluggable.** Rule-based default is cheap and deterministic; LLM judge is an opt-in tier for high-stakes analyses.
4. **Redact responses before next round.** Agents leak. The host is the only trust boundary we control.
5. **Hard budget cap, graceful degradation.** Truncating is better than surprise bills. A partial analysis with a scrappy Decision still beats no Decision.
6. **Analyses are first-class retrievable.** Spec 11's `similar_problems` intent treats resolved Analyses as a first-order citation target.
7. **No human-in-the-loop requirement.** You can run fully autonomous analyses when the stakes are low. Human judge is an escape hatch, not a default.

## Open questions

1. **Adversarial panelist seating.** Do we deliberately seed a "devil's advocate"? Shows up in research as a quality win; deferring to post-launch evaluation.
2. **Cross-analysis supersession.** If a new Analysis contradicts a resolved one, do we auto-supersede the old Decision? Leaning no — the judge should *explicitly* cite and supersede, not the engine.
3. **Pricing drift.** `pricing.toml` will go stale. Auto-fetch from provider pricing APIs? Low priority; manual quarterly sync is fine for v1.

## References

- Architecture §5.5 (Deep Analysis), §8 (context loop).
- Spec 01 — Event schema (`turn.analysis_round`, new kind).
- Spec 04 — Cortex Core (redactor).
- Spec 07 — Graph writer (`Analysis`, `DEBATED_IN`, `RESOLVES_TO`).
- Spec 11 — Query API (`intent=similar_problems`, context pack source).
- Spec 13 — Laws DSL (laws inclusion in context).
- Spec 14 — Governance engine (trust deltas from analysis outcomes — future).
- Spec 16 — Dashboard (Analysis library view, live-stream UI).

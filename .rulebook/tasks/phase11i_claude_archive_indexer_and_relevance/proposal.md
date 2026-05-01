# Proposal: phase11i_claude_archive_indexer_and_relevance

Source: [`docs/analysis/organize/`](../../../docs/analysis/organize/)

## Why

The agent runs blind. Every new Claude Code session starts with
`MEMORY.md` (8 lines) plus the Cortex pre-thinking bundle (5 generic
snippets), and ignores 4.5 GB of conversation history sitting on
disk under `C:\Users\Bolado\.claude\projects\` (9 835 JSONL session
transcripts across 31 project directories). 798 of those sessions
are this very repo (Cortex itself); 9 037 cover the rest of the
HiveLLM universe (Vectorizer, Nexus, Synap, Tml, UzEngine, …) plus
adjacent projects.

Every decision, every bug fix, every "we tried that and it didn't
work" lives in those JSONL files. When the agent starts a session,
none of it surfaces. The user has been losing institutional
knowledge at every `/clear`.

Two independent gaps:

1. **Ingestion gap.** `cortex-bootstrap` walks git repos. It can't
   walk a flat directory of JSONL session transcripts. There's no
   crate today that consumes `~/.claude/projects/`.
2. **Relevance gap.** Even with the corpus indexed, the query API
   has no recency decay, no model attribution, no session-cohesion
   boost, no outcome signal. RRF fusion treats every historical
   match equally. As the corpus grows from `~10K` docs to `~2.4M`,
   recall floods the bundle with noise.

This phase closes both. It depends on `phase11h_cortex_query_recall_recovery`
landing first (backend coverage `ok`, daemon at HEAD, ADRs + laws
ingested) — running 11i with 11h still outstanding would write the
new corpus into a half-bootstrapped backend.

## What Changes

### §1 — `cortex-claude-archive` crate

New sibling crate at `crates/cortex-claude-archive/`. JSONL walker
+ Envelope mapper + emitter. Two sinks: Synap (`cortex.events.bootstrap`)
or zstd-NDJSON parquet under `<CORTEX_ARCHIVE_ROOT>` so the existing
[`crates/cortex-api/src/archive_loader.rs`](../../../crates/cortex-api/src/archive_loader.rs)
seeds the keyword lane at `cortex-api` boot for free.

JSONL records → canonical `Envelope`:
- `user` + `assistant` paired by `parentUuid` → `Kind::Turn`
- `assistant.tool_use` blocks + matching `attachment.tool_result`
  → `Kind::ToolCall`
- `Agent` tool invocations → `Kind::AgentCall`
- sidecars (`history.jsonl`, `todos/`, `plans/`) → `Kind::Memory` or
  `Kind::Artifact`
- `~/.codex/` parallel corpus → same kinds with `tool: "openai-codex"`

CLI: `cortex-claude-archive {bootstrap|tail|estimate}` with
`--root`, `--projects-only`, `--sidecars`, `--codex`,
`--sink {synap|archive}`, `--resume`. Checkpoint every 5 s.

**No new `Kind` variants.** The schema absorbs the corpus as-is.

### §2 — Classifier + family wiring

Three string adds in
[`crates/cortex-workers/src/classifier/kinds.rs:19`](../../../crates/cortex-workers/src/classifier/kinds.rs#L19):
`"turn.claude-code"`, `"tool_call.claude-code"`,
`"agent_call.claude-code"`. Topic rule in
[`crates/cortex-classifier/src/statics.rs`](../../../crates/cortex-classifier/src/statics.rs):
every envelope with `tool == "claude-code"` adds `topics.push("claude-code")`.

### §3 — Relevance axes (5 + config)

Five orthogonal axes, each behind a flag in
`crates/cortex-api/config/relevance.toml`:

| Axis | Schema field | Filter / boost |
|---|---|---|
| Temporal recency | `occurred_at` | `Scope.recency_decay`; `exp(-λ·days)` per intent |
| Project scope | `context.repo` | `Scope.cross_repo_boost` (default 0) |
| Author + model | `model`, `tool` | `Scope.models`, `Scope.tools`; alias table |
| Session cohesion | `session_id` | `Scope.session_id` ×2.0; `Scope.session_cohort` ×1.5 |
| Outcome signal | `Turn.outcome` (derived) | `Scope.outcomes` / `exclude_outcomes`; success ×1.2, error ×0.5, blocked_by_law ×0.3 |

Settings v1 → v2: add `model`, `tool`, `session_id`, `outcome` to
Meili `filterableAttributes`. Vectorizer payloads carry the same
fields. Reload via `cortex-bootstrap --apply-settings-only`.

### §4 — Pre-thinking surfaces + measurement

Two new sections in
[`crates/cortex-pre-thinking/src/render.rs`](../../../crates/cortex-pre-thinking/src/render.rs):
`Past sessions` (top-3 by centroid similarity) and outcome glyphs
(`✓` / `✗` / `⚠`) on every turn / decision line. Stays under the
32 KiB clipper budget enforced by
[`crates/cortex-api/src/budget.rs`](../../../crates/cortex-api/src/budget.rs)
(phase 11c).

30-question hand-curated gold set under
`crates/cortex-api/tests/fixtures/relevance-gold.json`. CI runs
`relevance_eval_it` gated by `CORTEX_RELEVANCE_IT=1`, computes
`mrr@10` and `ndcg@10`, fails when `mrr@10 < 0.75`.

### §5 — Watcher daemon + ops

`cortex-claude-archive tail` becomes a managed docker-compose
service: read-only bind mount of `~/.claude/projects/`, restart on
failure, depends on `synap` + `cortex-ingestion`. Health endpoint
`:17030/healthz` + extension to `/v1/health/coverage`. Hard cap
≤ 512 MiB RSS.

### §6 — Tail (mandatory)

CHANGELOG, `docs/architecture.md` §6, `docs/specs/16-dashboard.md`
Memory-view section, full IT suite green.

## Impact

- **Affected specs:** `01` (event schema — no kind change, but new
  filterable fields), `02` (storage layout — new global index
  `cortex_claude_archive_turns` if §3.4 lands), `08` (Meili settings
  v1→v2), `11` (query API — new Scope fields), `12` (pre-thinking —
  new sections), `16` (dashboard — Memory view), `17` (multi-AI
  adapter family — Codex parallel ingestion).
- **Affected code:**
  - **New:** `crates/cortex-claude-archive/`,
    `crates/cortex-cli/src/bin/cortex-claude-archive.rs`,
    `crates/cortex-api/config/relevance.toml`,
    `crates/cortex-api/tests/fixtures/relevance-gold.json`,
    `docs/cortex/relevance-tuning.md`.
  - **Modified:** `crates/cortex-core/src/types.rs` (Scope fields),
    `crates/cortex-core/src/redact.rs` (new patterns),
    `crates/cortex-workers/src/classifier/kinds.rs`,
    `crates/cortex-workers/src/fulltext/settings.rs` (v2),
    `crates/cortex-classifier/src/statics.rs` (Turn outcome
    derivation),
    `crates/cortex-api/src/fusion.rs` (multipliers),
    `crates/cortex-api/src/strategies.rs` (new lanes),
    `crates/cortex-pre-thinking/src/render.rs`,
    `docker-compose.yml`.
- **Breaking:** NO. New crate is additive; new Scope fields are
  optional with safe defaults; settings v2 is auto-applied via
  `cortex-bootstrap --apply-settings-only`; pre-thinking sections
  fit in the existing budget.
- **Storage cost:** ~2.4 M Meili docs (~1.5 GB index), ~6 M
  Vectorizer chunks (PQ-only on warm tier, FP32 hot tier capped at
  the most-recent 90 days; ~6 GB total), ~5 M Nexus nodes (~3 GB).
  Sized to fit the dev machine.
- **User benefit:** Cortex becomes the agent's persistent memory.
  Every past session — every decision, bug, failed approach — is
  retrievable, decayed by recency, scoped by project, weighted by
  outcome. The pre-thinking bundle on every turn includes "Past
  sessions" surfacing the most relevant historical context for the
  current question.

## Source

[`docs/analysis/organize/`](../../../docs/analysis/organize/) — full
inventory, pipeline trace, gap analysis, relevance framework, and
build sequence. Read README.md first; the five companion files
provide the evidence the proposal compresses.

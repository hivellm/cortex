# Proposal: phase11k_governance_lane_projection

Source: [`docs/cortex/governance-ingestion.md`](../../../docs/cortex/governance-ingestion.md)

## Why

`phase11h_cortex_query_recall_recovery` §3.1 + §3.3 successfully
ingest ADRs and behavioral laws into `cortex-{slug}-decisions` and
`cortex-{slug}-governance` (14 + 389 documents in cortex's own
bootstrap). But §3.2 + §3.4 acceptance — `decision_lookup` returning
`results.decisions[]` and `law_check` returning `results.violations[]`
— fail in three independent ways:

1. **Writer-side top-level projection missing.**
   `crates/cortex-workers/src/fulltext/document.rs::Document` has no
   `decision_id`, `decision_title`, `decision_status`, `law_id`,
   `severity`, `tier`, or `turn_id` fields. The bootstrap envelope's
   payload is serialised into `body` as a JSON string and the rest is
   inaccessible to the spec-11 lane projection contract. The
   orchestrator's `derive_decisions` reads
   `extras["decision_id"]` — the meili lane projection in
   `crates/cortex-api/src/meili_lane.rs` flattens top-level Meili doc
   fields into `extras_raw`, but since the worker never writes those
   fields, `extras["decision_id"]` is always missing and
   `derive_decisions` returns empty. Same shape for laws.

2. **Global Meili indexes queried but never written.**
   `cortex-api/src/strategies.rs` routes `decision_lookup` to the
   global `cortex_decisions` index and `law_check` to `cortex_laws`.
   Neither index exists on the running Meili (the workers only write
   per-repo `cortex-{slug}-decisions` / `cortex-{slug}-governance`).
   The query 400s on the missing index, the orchestrator silently
   drops the lane, and global searches across all repos for an ADR
   are impossible.

3. **`AGENTS.override.md` is classified as Memory, not Law.**
   `LAW-CORTEX-*` declarations live in `AGENTS.override.md`, which
   `[cortex.memories].import_files` picks up as `Kind::Memory`. The
   law promote patterns (`.rulebook/laws/*.yaml`,
   `.claude/rules/*.md`) do not match it. So `LAW-CORTEX-001` never
   reaches the governance lane in the first place — even when the
   read-side projection lands, `law_check "task sequence cherry pick"`
   would still miss it.

This task closes all three gaps. It depends on `phase11h` having
landed (daemon at HEAD, coverage ok) — that's the foundation.

## What Changes

### §1 — Top-level Meili projection for governance kinds

Extend `crates/cortex-workers/src/fulltext/document.rs`'s `Document`
struct with optional fields stamped only when the source kind
warrants them:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub decision_id: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub decision_title: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub decision_status: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub decision_supersedes: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub law_id: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub law_severity: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub law_tier: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub turn_id: Option<String>,
```

`crates/cortex-workers/src/fulltext/builders.rs` reads the payload
JSON for `Kind::Decision` / `Kind::LawViolation` / `Kind::Turn` and
sets the matching fields. Settings v1 → v2 adds the new keys to
`filterableAttributes` + `searchableAttributes` where appropriate.

### §2 — Global decisions/laws indexes

Either extend `index_for_event` to ALSO write each Decision /
LawViolation envelope to the global `cortex_decisions` / `cortex_laws`
index, or update `cortex-api/src/strategies.rs` to drop the global
lane and rely on a per-repo fan-out. Decision: write to global —
keeps cross-repo `decision_lookup` (asking "have we ever decided X?"
without specifying the repo) working.

### §3 — `LAW-CORTEX-*` extraction from override files

Two options, ship both:
- Extend `[cortex.laws].promote_patterns` in `cortex.toml` to include
  `AGENTS.override.md` (and any future override files).
- Add a `[cortex.laws].extract_pattern` regex (`^LAW-[A-Z0-9-]+`)
  that the bootstrap walker uses to scan files classified as Memory
  AND emit a sibling `Kind::LawViolation` envelope per matched
  declaration.

### §4 — Auto-republish on file change

`cortex-claude-archive` (phase11i §5) ships a watcher daemon. Extend
its scope to also watch `.rulebook/decisions/`, `.rulebook/laws/`,
`.claude/rules/`, `AGENTS.override.md`. On change, re-publish the
relevant envelope to `cortex.events.bootstrap`. Idempotent via
`content_hash` dedupe at the worker.

### §5 — Acceptance ITs

- `decision_lookup_it.rs` — seeds an ADR via bootstrap, fires
  `decision_lookup`, asserts `results.decisions[]` non-empty with
  `decision_id` matching the ADR file path's slug.
- `law_check_it.rs` — seeds a LAW-CORTEX-* via the new
  extraction path, fires `law_check`, asserts `results.violations[]`
  contains the law id with severity + rationale.
- `governance_global_index_it.rs` — fires `decision_lookup` with no
  `scope.repo`, asserts hits from at least 2 different repos in the
  global lane.
- `governance_watcher_it.rs` — modifies a fixture ADR file, asserts
  the change reaches the index within 2 s.

### §6 — Tail (mandatory)

Update CHANGELOG, `docs/cortex/governance-ingestion.md` (flip status
to 🟢), `docs/specs/16-dashboard.md` Memory + Decisions views.
Quality pipeline green.

## Impact

- **Affected specs:** `01` (event schema — no kind change), `08`
  (Meili settings v1→v2 plus new fields), `11` (query API
  strategies — global lanes), `16` (dashboard).
- **Affected code:**
  - `crates/cortex-workers/src/fulltext/document.rs` — Document fields
  - `crates/cortex-workers/src/fulltext/builders.rs` — payload → field
  - `crates/cortex-workers/src/fulltext/settings.rs` — v2 schema
  - `crates/cortex-workers/src/fulltext/routing.rs` — global index fan-out
  - `crates/cortex-api/src/strategies.rs` — strategy lane wiring
  - `crates/cortex-api/src/meili_lane.rs` — projection assertions
  - `cortex.toml` — promote pattern extension
  - `crates/cortex-cli/src/bootstrap/walker.rs` — extract pattern path
  - `crates/cortex-claude-archive/src/watcher.rs` — governance scope
- **Breaking:** NO. Settings v2 is auto-applied; existing per-repo
  index reads continue to work; new global indexes are opt-in via
  the strategy update; promote pattern extension is purely additive.
- **User benefit:** `decision_lookup` and `law_check` finally return
  populated `results.decisions[]` / `results.violations[]` overlays
  with searchable IDs + statuses + severities. Cross-repo decision
  lookups answer "where in the codebase did we ever decide X?"
  without forcing the caller to enumerate repos. `LAW-CORTEX-*`
  declarations become enforceable through the law lane.

## Source

[`docs/cortex/governance-ingestion.md`](../../../docs/cortex/governance-ingestion.md)
documents the current contract + the four open follow-ups this task
closes.

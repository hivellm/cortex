# Proposal: phase2_rulebook_artifact_indexer

## Why

The `/v1/query` orchestrator returns `decisions=0`, `laws_active=0`, `violations=0` for every prompt — confirmed against the live daemon on 2026-04-27 with 11 distinct queries. Yet the project carries real institutional content on disk:

- `.rulebook/decisions/*.md` — 2 ADRs (`001-bypass-vectorizer-sdk-…`, `002-classifier-worker-lives-…`)
- `.rulebook/learnings/*.md` — 3 entries (most recent: the very anti-pattern captured by the previous task)
- `.rulebook/knowledge/anti-patterns/*.md` — 6 entries (including `cypher-unwind-write-…`, `don-t-ship-a-bespoke-http-client-…`)
- `.rulebook/specs/*.md` — `RULEBOOK.md`, `GIT.md`, `QUALITY_ENFORCEMENT.md`, `AGENT_AUTOMATION.md`, `MULTI_AGENT.md`, `TIER1_PROHIBITIONS.md`, `TOKEN_OPTIMIZATION.md` (the canonical "laws" of this project)

None of this reaches the model when pre-thinking fires. The information already exists in machine-readable form (each artifact ships a sibling `.metadata.json`). The gap is a missing indexer that emits these files into the cortex retrieval surfaces under the correct kinds (`decision`, `law`, `pattern`, `anti_pattern`, `learning`).

This task closes that gap. It is the highest-ROI follow-up to `phase1_adapter_pre_thinking_contract_fix` because it does not require any live lane upgrade — the orchestrator already builds `DecisionRef` / `LawRef` / `Snippet` overlays from whatever the lanes emit; it just receives nothing today.

## What Changes

- New crate (or new module under `cortex-bootstrap`) `cortex-rulebook-indexer` that:
  - Walks `.rulebook/decisions/`, `.rulebook/learnings/`, `.rulebook/knowledge/{patterns,anti-patterns}/`, `.rulebook/specs/` in the repo root.
  - Reads each artifact + sibling `.metadata.json` and produces canonical envelopes:
    - decisions → `Kind::Decision` (or `Kind::ArtifactDecision` if a new variant is needed) carrying id/title/status/ts/links/body
    - learnings → `Kind::Learning`
    - patterns / anti-patterns → `Kind::Pattern` with a `pattern_kind` discriminator
    - specs → `Kind::Law` (one envelope per `### Requirement` / `LAW-NNN` block, or one per spec doc with the laws extracted into `payload.laws`)
  - Emits the envelopes through the canonical publisher path so they flow into the same archive + lanes as turn / tool_call.
- Wire the indexer into `cortex-bootstrap` so a fresh stack picks them up at boot, plus a watch mode (or hourly re-scan) so post-boot edits land.
- The orchestrator's `decisions` strategy (`crates/cortex-api/src/strategies.rs`) and `laws_active` overlay must consume the new kinds — surface them under `results.decisions` / `laws_active` / `results.snippets` (for patterns / learnings) without further downstream churn.
- The `MemoryKeywordLane` test-double can keep being used for now (live lanes ship in sibling tasks) — what matters here is that the data exists in the lane.

## Impact

- Affected specs: spec-04 (cortex-core kinds), spec-09 (bootstrap CLI), spec-11 (query API result population)
- Affected code:
  - new: `crates/cortex-rulebook-indexer/` (or `crates/cortex-bootstrap/src/rulebook.rs`)
  - `crates/cortex-core/src/events.rs` (likely new `Kind` variants)
  - `crates/cortex-bootstrap/src/main.rs` (wire the walker)
  - `crates/cortex-api/src/strategies.rs` (consume new kinds in decisions / laws / similar overlays)
  - tests in each
- Breaking change: NO (additive — new envelope kinds, new walker)
- User benefit: pre-thinking bundles surface real ADRs, anti-patterns, learnings, and laws on every relevant prompt — the original promise of the system.

## Source

This task derives from the 2026-04-27 audit captured in the learning `adapter-sync-paths-must-use-the-cortex-pre-thinking-pipeline-not-a-bespoke-http-client` and the live probe summary that recorded `decisions=0 laws=0` for 11 queries.

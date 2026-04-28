# 07 — Quality, tests, captured knowledge

## Test footprint

| Metric                                | Count                                                |
|---------------------------------------|------------------------------------------------------|
| Test files (under `crates/*/tests/`)  | 58                                                   |
| Test functions (`#[test]` + `#[tokio::test]`) | 291                                          |
| Rust LOC across all crates            | ~44k                                                 |
| Tests / kLOC                          | ~6.6                                                 |

This is reasonable density for a Rust workspace, but the AGENTS.md target is **≥95% coverage** — there is no coverage report in the repo to verify against the bar. No `tarpaulin.toml`, no `lcov` artifact in CI files.

**Action:** wire `cargo tarpaulin` (or `cargo llvm-cov`) into `Makefile` and produce a per-crate coverage report. Required by the project's own quality gate.

## Test categories observed

- **Unit tests** colocated with source modules (most crates).
- **Integration tests** under `crates/*/tests/` — env-gated via `CORTEX_IT=1` per the [recorded pattern](../../../.rulebook/knowledge/patterns/env-gated-integration-tests-via-cortex-it-1-early-return.md). This means CI runs unit tests by default; integration tests against the docker-compose stack are opt-in. That matches the user's preference for **real tests, not mocks** ([memory: feedback_real_tests](../../../../C:/Users/Bolado/.claude/projects/e--HiveLLM-Cortex/memory/feedback_real_tests.md)) — but only if `CORTEX_IT=1` is actually flipped in CI / pre-commit.

**Action:** confirm `CORTEX_IT=1` is set in CI for the integration tier (gate before merging non-trivial PRs).

## Captured patterns ([.rulebook/knowledge/patterns/](../../../.rulebook/knowledge/patterns/))

| Pattern                                                                                                                                                                                          | Why it matters                                                                                          |
|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| [Dedup by metadata key when server assigns primary IDs](../../../.rulebook/knowledge/patterns/dedup-by-metadata-key-when-server-assigns-primary-ids.md)                                          | Vectorizer assigns its own UUIDs; client must dedup on `chunk_content_hash` instead of trusting server ID. |
| [Env-gated integration tests via `CORTEX_IT=1`](../../../.rulebook/knowledge/patterns/env-gated-integration-tests-via-cortex-it-1-early-return.md)                                               | Lets integration tests live next to unit tests without slowing every `cargo test`.                       |
| [Live external service lane with in-memory fallback at daemon boot](../../../.rulebook/knowledge/patterns/live-external-service-lane-with-in-memory-fallback-at-daemon-boot.md)                  | Daemon does not crash if Vectorizer/Nexus/Meili is down; lanes degrade gracefully.                       |
| [Synap publisher should auto-create rooms on first not-found](../../../.rulebook/knowledge/patterns/synap-publisher-should-auto-create-rooms-on-first-not-found.md)                              | Removes the chicken-and-egg "stream doesn't exist yet" boot problem.                                     |

## Captured anti-patterns ([.rulebook/knowledge/anti-patterns/](../../../.rulebook/knowledge/anti-patterns/))

| Anti-pattern                                                                                                                                                                                                                            | The trap                                                                                              |
|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------|
| [Cypher UNWIND-write and param-write substitution silently drop in Nexus 1.15](../../../.rulebook/knowledge/anti-patterns/cypher-unwind-write-and-param-write-substitution-silently-drop-in-nexus-1-15.md)                              | Driver returns 200, no rows land. Fix: per-row Cypher + post-write read-back.                          |
| [Don't bake tooling-only fields into JSON sent verbatim downstream](../../../.rulebook/knowledge/anti-patterns/don-t-bake-tooling-only-fields-into-json-payloads-sent-verbatim-to-a-strict-downstream.md)                                | Meili rejects unknown fields on settings PATCH. Fix: strip at client boundary.                          |
| [Don't ship a bespoke HTTP client when an in-tree pipeline crate already drives that endpoint](../../../.rulebook/knowledge/anti-patterns/don-t-ship-a-bespoke-http-client-when-an-in-tree-pipeline-crate-already-drives-that-endpoint.md) | Adapter had its own HTTP path; pipeline crate already covered it. Fix: collapse to one path.            |
| [Vectorizer SDK 3.0.3 follow-up — 2 of 6 drifts resolved, 3-6 still server-side](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md)                     | Tracking. Insert + login resolved; get_vector still bypassed.                                          |
| [Vectorizer SDK 3.0.3 round 8 — server-assigned UUIDs accepted, drift 4 neutralised](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-round-8-follow-up-server-assigned-uuids-accepted-drift-4-neutralised.md)            | Continuation of the same epic.                                                                          |
| [Vectorizer SDK 3.0 drifts from hivehub/vectorizer 3.0.0-dev image](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-drifts-from-hivehub-vectorizer-3-0-0-dev-image.md)                                                    | Original epic — the Cargo.toml SDK pin and the Docker image went out of sync.                          |

## Captured learnings ([.rulebook/learnings/](../../../.rulebook/learnings/))

Five learnings, dated 2026-04-22 → 2026-04-27:

1. **tree-sitter 0.22 grammars cc version conflict** — bumped all to 0.23.
2. **End-to-end Cortex bootstrap on the Cortex repo: pipeline gaps surfaced** (the canonical post-mortem of the 2026-04-27 audit).
3. **Adapter sync paths must use the cortex-pre-thinking pipeline, not a bespoke HTTP client** (anti-pattern source).
4. **MCP server tool descriptors must match spec contract — names without dots, schema fields in camelCase**.
5. **Per-project collection isolation: slug `repo` into every collection/index name**.

These are exactly the kind of artefacts the architecture prescribes — institutional memory that future sessions consult before re-tripping the same wires.

## Decisions ([.rulebook/decisions/](../../../.rulebook/decisions/))

Two ADRs:

- **ADR 001 — Bypass vectorizer-sdk for /insert and /get_vector** — partially **superseded** (insert now uses SDK 3.0.3); get_vector still bypassed. Lifecycle is exemplary: never deleted, just superseded.
- **ADR 002 — Classifier worker lives in a separate crate to avoid the classifier→embedder→classifier cycle** — proposed status; the actual implementation matches the decision (see commit history around `phase1_classifier_worker`).

ADR coverage is **thin**. The architecture has many more decisions worth capturing as ADRs (the Sonnet-vs-Haiku split for classify-vs-analyze, the per-row Cypher response to Nexus drift, the Meilisearch-as-stand-in-for-Lexum choice, etc.). Most of them currently live as inline comments in code or as remembered context in `.rulebook/memory/`.

**Action:** backfill ADRs for the 5-7 most load-bearing implicit decisions. Cheap, high-leverage.

## Code-quality signals worth flagging

1. **Tier-1 prohibitions are enforced by hooks**, not by review alone — see the `.claude/rules/*.md` and the rulebook PreToolUse hook that denies background `Agent` calls without a Team. Good defense-in-depth.
2. **Deferred-items protocol is explicit** ("If you must defer an item before archiving, you MUST create a follow-up rulebook task" — [AGENTS.md](../../../AGENTS.md)). The 12 pending tasks suggest the protocol is actually followed.
3. **No half-implemented placeholders observed** in the spec-marked-🟢 crates. The `--max-tokens` CLI bug is the kind of breakage that gets caught and committed within hours (`c41dab0`), not left to rot.

The codebase health is **good** at the implementation level. The visible problems are at the *integration* level — drifts and asymmetric coverage — not at the code-quality level.

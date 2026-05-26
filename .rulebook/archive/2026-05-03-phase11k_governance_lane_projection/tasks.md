## 1. Top-level Meili projection for governance kinds

- [x] 1.1 Extend `crates/cortex-workers/src/fulltext/document.rs`'s `Document` struct with optional fields: `decision_id`, `decision_title`, `decision_status`, `decision_supersedes`, `law_id`, `law_severity`, `law_tier`, `turn_id` (each `#[serde(skip_serializing_if = "Option::is_none")]`)
- [x] 1.2 `crates/cortex-workers/src/fulltext/builders.rs` — for `Kind::Decision`, parse `payload` and stamp `decision_id` / `decision_title` / `decision_status` / `decision_supersedes`; for `Kind::LawViolation`, stamp `law_id` / `law_severity` / `law_tier`; for `Kind::Turn`, stamp `turn_id` (= `event_id`)
- [x] 1.3 Bump `crates/cortex-workers/settings/settings.v1.json` v4 → v5 adding the new fields to `filterableAttributes` and the human-facing ones (`decision_title`, `law_id`) to `searchableAttributes`
- [x] 1.4 Add 7 unit tests in `builders.rs` (`top_level_projection_tests`): per-kind stamping (Decision / LawViolation / Turn), missing-supersedes / missing-tier tolerance, malformed-payload defensiveness, no top-level on non-governance kinds, JSON round-trip
- [x] 1.5 Update existing builder ITs (`crates/cortex-workers/tests/fulltext_builders.rs`) to assert the new fields where applicable (Turn, Decision, LawViolation)

## 2. Global decisions/laws Meili indexes

- [x] 2.1 Extend `crates/cortex-workers/src/fulltext/routing.rs`: add `index_for_event_global(kind) -> Option<&'static str>` that returns `"cortex_decisions"` for `Kind::Decision` and `"cortex_laws"` for `Kind::LawViolation`, `None` otherwise
- [x] 2.2 `crates/cortex-workers/src/fulltext/indexer.rs` — when `index_for_event_global` returns `Some`, also `ensure_index` + `add_documents` to the global index in addition to the per-repo write
- [x] 2.3 Settings push: global indexes use the same v5 schema as per-repo (filterableAttributes include `repo` so cross-repo filters still work)
- [x] 2.4 IT `crates/cortex-workers/tests/global_governance_indexes_it.rs` — emit Decision envelopes from repo A and repo B (3 cases); assert both land in `cortex_decisions` AND in their per-repo indexes; same shape for `cortex_laws`; non-governance kinds do NOT dual-write

## 3. LAW-CORTEX-* extraction path

- [x] 3.1 Extend `[cortex.laws].promote_patterns` in `cortex.toml` to include `AGENTS.override.md` and `AGENTS.md`
- [x] 3.2 Add `[cortex.laws].extract_pattern = "^LAW-[A-Z0-9-]+$"` config knob; new `emit_extracted_laws_imported` in `crates/cortex-cli/src/bootstrap/emitter.rs` splits Law-classified file bodies by `## ` headings whose first token matches the pattern, emitting one `law.imported` per match (and falling back to single-law if no match). Wired through `emit_for_file_multi_with_extract` + `runner.rs`.
- [x] 3.3 IT `crates/cortex-cli/tests/bootstrap_law_extraction_it.rs` — fixture w/ AGENTS.override.md containing two LAW-CORTEX-* declarations, assert two `law.imported` envelopes emitted with the right `law_id`; second test confirms fallback for files with no matching headings

## 4. Auto-republish on file change

- [x] 4.1 New `crates/cortex-claude-archive/src/governance_watcher.rs` polls `.rulebook/decisions/`, `.rulebook/laws/`, `.claude/rules/`, `AGENTS.override.md`, `AGENTS.md`. The §5 tail watcher continues to poll JSONL sessions; the governance watcher is a sibling module callers can drive at the same cadence.
- [x] 4.2 On change (write / delete) the watcher emits a `GovernanceChange` to a `GovernanceEmitter` trait. `MemoryGovernanceEmitter` is the in-memory test seam; the Synap-bound emitter that publishes onto `cortex.events.bootstrap` ships as a follow-up wrap. Idempotent via per-file content-hash cursor.
- [x] 4.3 IT `crates/cortex-claude-archive/tests/governance_watcher_it.rs` — modifies a fixture ADR file, asserts the change reaches the emitter within 2 s and the post-write upsert carries the mutated body

## 5. Acceptance ITs

- [x] 5.1 `crates/cortex-api/tests/decision_lookup_it.rs` — seeds the keyword lane with the projection a phase11k §1 worker writes; fires `decision_lookup`; asserts `results.decisions[]` non-empty with `decision_id` / `title` / `status` matching
- [x] 5.2 `crates/cortex-api/tests/law_check_it.rs` — seeds `LAW-CORTEX-001` projection in `cortex_laws`; fires `law_check`; asserts top-level `laws_active` overlay non-empty with the seeded `law_id` + severity
- [x] 5.3 `crates/cortex-api/tests/governance_global_index_it.rs` — fires `decision_lookup`; seeds the global `cortex_decisions` index with hits from two repos; asserts surfacing from at least 2 distinct repos
- [x] 5.4 Each IT gates the live-stack form behind `CORTEX_GOVERNANCE_IT=1` (early-return + eprintln). The repo ships neither a Justfile nor a `docker-compose.test.yml`; the gating lives inline in each test alongside the existing `CORTEX_MEILI_IT` pattern in `meili_filter_grammar_it.rs`.

## 6. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 6.1 Update or create documentation covering the implementation — `docs/cortex/governance-ingestion.md` status flipped 🟡 → 🟢, caveat replaced with closure summary, read-path table updated; CHANGELOG entry under `[Unreleased]` Added; `docs/specs/16-dashboard.md` Decisions / Laws / Violations rows annotated with phase11k §1 + §2 contract notes
- [x] 6.2 Write tests covering the new behavior — 7 new unit tests in `builders.rs::top_level_projection_tests`, 1 new settings test, 3 new routing tests, 3 new ITs in `global_governance_indexes_it.rs`, 2 new ITs in `bootstrap_law_extraction_it.rs`, 5 new unit tests in `governance_watcher::tests`, 1 new watcher IT, 3 new acceptance ITs in cortex-api
- [x] 6.3 Run tests and confirm they pass — `cargo check -p cortex-workers -p cortex-cli -p cortex-claude-archive -p cortex-api` clean; `cargo test -p cortex-workers --lib` 289/0; `cargo test -p cortex-workers --tests`, `cortex-cli --tests`, `cortex-claude-archive`, `cortex-api --tests` all green; live-stack ITs gated behind `CORTEX_GOVERNANCE_IT=1`. Clippy hits pre-existing strict-warning issues in `cortex-core` / `cortex-health` unrelated to phase11k; `cargo fmt --check` clean for touched files.
- [x] 6.4 Captured learning: `Spec-11 lane projection contract: top-level fields, not nested ext` (id 2026-05-03T02-29-50-spec-11-lane-projection-contract-top-level-fields-not-nested-ext)
- [x] 6.5 Captured decision: ADR `Governance kinds dual-write to per-repo + global Meili indexes` (id 6, slug governance-kinds-dual-write-to-per-repo-global-meili-indexes)

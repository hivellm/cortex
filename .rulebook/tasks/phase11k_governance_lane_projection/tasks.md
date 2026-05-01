## 1. Top-level Meili projection for governance kinds

- [ ] 1.1 Extend `crates/cortex-workers/src/fulltext/document.rs`'s `Document` struct with optional fields: `decision_id`, `decision_title`, `decision_status`, `decision_supersedes`, `law_id`, `law_severity`, `law_tier`, `turn_id` (each `#[serde(skip_serializing_if = "Option::is_none")]`)
- [ ] 1.2 `crates/cortex-workers/src/fulltext/builders.rs` — for `Kind::Decision`, parse `payload` and stamp `decision_id` / `decision_title` / `decision_status` / `decision_supersedes`; for `Kind::LawViolation`, stamp `law_id` / `law_severity` / `law_tier`; for `Kind::Turn`, stamp `turn_id` (= `event_id`)
- [ ] 1.3 Bump `crates/cortex-workers/settings/settings.v1.json` → `settings.v2.json` adding the new fields to `filterableAttributes` and the human-facing ones (`decision_title`) to `searchableAttributes`
- [ ] 1.4 Add 6 unit tests in `builders.rs`: one per (Kind, field) combination covering field stamping, missing-payload tolerance, and JSON round-trip
- [ ] 1.5 Update existing builder ITs (`crates/cortex-workers/tests/fulltext_builders.rs`) to assert the new fields where applicable

## 2. Global decisions/laws Meili indexes

- [ ] 2.1 Extend `crates/cortex-workers/src/fulltext/routing.rs`: add `index_for_event_global(prefix, kind) -> Option<&'static str>` that returns `"cortex_decisions"` for `Kind::Decision` and `"cortex_laws"` for `Kind::LawViolation`, `None` otherwise
- [ ] 2.2 `crates/cortex-workers/src/fulltext/indexer.rs` — when `index_for_event_global` returns `Some`, also `ensure_index` + `add_documents` to the global index in addition to the per-repo write
- [ ] 2.3 Settings push: global indexes use the same v2 schema as per-repo (filterableAttributes include `repo` so cross-repo filters still work)
- [ ] 2.4 IT `crates/cortex-workers/tests/global_governance_indexes_it.rs` — emit one Decision envelope from repo A and one from repo B, assert both land in `cortex_decisions` AND in their per-repo indexes

## 3. LAW-CORTEX-* extraction path

- [ ] 3.1 Extend `[cortex.laws].promote_patterns` in `cortex.toml` to include `AGENTS.override.md` and `AGENTS.md` (so future LAW-* declarations in either file land as `Kind::LawViolation`)
- [ ] 3.2 Add `[cortex.laws].extract_pattern = "^LAW-[A-Z0-9-]+$"` config knob; `crates/cortex-cli/src/bootstrap/walker.rs::classify_path` checks files that would otherwise be `Memory` and, when an extract pattern matches a heading or code-fence label inside, also emits a sibling Law envelope per match
- [ ] 3.3 IT `crates/cortex-cli/tests/bootstrap_law_extraction_it.rs` — fixture w/ AGENTS.override.md containing two LAW-CORTEX-* declarations, assert two `Kind::LawViolation` envelopes emitted with the right `law_id` payload

## 4. Auto-republish on file change

- [ ] 4.1 Extend `cortex-claude-archive`'s watcher (phase11i §5) to also watch `.rulebook/decisions/`, `.rulebook/laws/`, `.claude/rules/`, `AGENTS.override.md`, `AGENTS.md`
- [ ] 4.2 On file change (rename / write / delete) emit the corresponding envelope to `cortex.events.bootstrap`; rely on `content_hash` dedupe at the worker so re-publishing the same file is a no-op
- [ ] 4.3 IT `cortex-claude-archive/tests/governance_watcher_it.rs` — modify a fixture ADR file, assert the change reaches `cortex-cortex-decisions` index within 2 s

## 5. Acceptance ITs

- [ ] 5.1 `crates/cortex-api/tests/decision_lookup_it.rs` — seed an ADR via bootstrap; fire `decision_lookup`; assert `results.decisions[]` non-empty with `decision_id` matching the ADR file slug
- [ ] 5.2 `crates/cortex-api/tests/law_check_it.rs` — seed `LAW-CORTEX-001` via the new extraction path; fire `law_check "task sequence cherry pick"`; assert `results.violations[]` contains the law id + severity + rationale excerpt
- [ ] 5.3 `crates/cortex-api/tests/governance_global_index_it.rs` — fire `decision_lookup` with no `scope.repo`; assert hits come from at least 2 different repos via the global lane
- [ ] 5.4 Wire all three into `Justfile` / `docker-compose.test.yml` gated by `CORTEX_GOVERNANCE_IT=1`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 6.1 Update or create documentation covering the implementation — flip `docs/cortex/governance-ingestion.md` status to 🟢; CHANGELOG entry; update `docs/specs/16-dashboard.md` Decisions + Laws views to call out the new top-level filterable fields
- [ ] 6.2 Write tests covering the new behavior — every IT named in §1-§5 lands; coverage ≥ 95 % on `crates/cortex-workers/src/fulltext/`
- [ ] 6.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`, IT suite gated by `CORTEX_GOVERNANCE_IT=1`
- [ ] 6.4 Capture learnings: `rulebook_learn_capture` for the lane projection contract pattern (writer-side stamps top-level fields, reader-side relies on flat extras_raw; midway transforms via meili filterableAttributes are off-limits)
- [ ] 6.5 Capture decision: `rulebook_decision_create` for the per-repo + global dual-write strategy (preserves cross-repo lookup without mandating repo enumeration on every caller)

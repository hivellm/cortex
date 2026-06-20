## 0. Investigation (2026-06-20) — read before deciding §1.1
- The current emit+build path is CORRECT: `emit_spec_laws_imported` produces a clean `LawPayload`-shaped json (`law_id,title,severity,detector,body,section_index,source_path`) that `build_doc`'s `Kind::Law` arm parses into clean `title`/`body`. So the 240 malformed `cortex_laws` docs (`title==id`, body=stringified JSON) are STALE from an old emit path — re-emit through the current path fixes content.
- **Contract conflict across read paths:**
  - `cortex_laws.settings.v1.json` schema + `law_check` (strategies.rs:379) + `law_violations` handler comments → `cortex_laws` = global law DEFINITIONS; violations live per-repo `cortex-<slug>-governance`.
  - BUT `routing::index_for_event_global` dual-writes `Kind::LawViolation` (NOT `Kind::Law`) to `cortex_laws`, and `events_by_kind` (events_by_kind.rs:82) maps BOTH `law` and `violation` queries to `cortex_laws` (its only GLOBAL violation source — violations are otherwise per-repo only).
  - Live `cortex_laws` currently holds 240 DEFINITION docs, 0 violations.
- So definitions don't route to the index `law_check` reads, and there is no clean global home for violations — the tension `events_by_kind` papered over by pointing `violation` at `cortex_laws`.

## 1. Settle the contract + routing
- [x] 1.1 DECIDED (user, 2026-06-20): **`cortex_laws` = definitions-only.** Violations stay per-repo `governance`; `events_by_kind` violation queries read per-repo governance (require a repo), not the global laws index.
- [x] 1.2 DONE: `routing::index_for_event_global` now `Kind::Law => Some(cortex_laws)`, `Kind::LawViolation => None` (violations no longer dual-write globally); `events_by_kind` "violation" → `None` global (per-repo governance only). Unit tests updated/added (`law_violations_stay_per_repo_governance_not_global_cortex_laws`, `law_definitions_land_in_global_cortex_laws`, events_by_kind tests). Workspace gates green.

## 2. Fix the emit/build path
- [x] 2.1 DONE: confirmed the current emit+build path is already correct — `emit_spec_laws_imported`/`emit_extracted_laws_imported` produce clean `LawPayload` json that `build_doc`'s `Kind::Law` arm parses to clean title/body. The 240 malformed docs are stale from an old path; no builder fix needed.
- [x] 2.2 DONE: `laws-reindex` test asserts a built law doc has `title != id`, clean (non-JSON-object) body, and a stable `bootstrap-` id.

## 3. Reindex + verify (BLOCKED on multi-source coverage — do NOT run the live prune yet)
- [ ] 3.1 ⏸ blocked: the live `cortex_laws` 240 docs span THREE sources — `.claude/rules` (45), **`docs/specs` (191)**, `AGENTS*.md` (4). The new `cortex-ops laws-reindex` currently re-emits ONLY `.claude/rules` (40 docs) and prunes ALL legacy → running it live would DELETE the 191 spec + 4 AGENTS laws. Extend it to re-emit every law source (`.claude/rules` via `emit_spec_laws_imported` + `docs/specs` via `emit_extracted_laws_imported` + AGENTS) so the prune-all-legacy is safe, OR scope the prune to only re-emitted source paths. The routing fix (§1.2) already makes the LIVE write path correct going forward; this item is the one-time historical-data repair.
- [ ] 3.2 After §3.1: run live, verify `cortex_laws` shows real law title+body, 0 `title==id`, all `bootstrap-`-keyed; `law_check` returns definitions; dashboard law counts stay correct.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation. DONE for the landed routing fix: CHANGELOG `Fixed` entry (governance routing → `cortex_laws` definitions-only). Spec 08 §3.1-coverage note pending the live repair.
- [x] 4.2 Write tests covering the new behavior. DONE: routing (`law_definitions_land_in_global_cortex_laws`, `law_violations_stay_per_repo_governance_not_global_cortex_laws`), events_by_kind violation→None, laws-reindex builder test.
- [x] 4.3 Run tests and confirm they pass. DONE: `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --test-threads=1` green (194 suites, 0 failures). NOTE: §3 (live historical repair) intentionally NOT run — would lose 195 non-`.claude/rules` laws until the command covers all sources.

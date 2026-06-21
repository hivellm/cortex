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

## 3. Repair + verify
- [x] 3.1 DONE via an in-place repair that sidesteps the risky multi-source re-walk: the malformed `body` is the STRINGIFIED original law payload, so the real content is recoverable from each doc. New `cortex-ops laws-repair` unwraps `body` → reconstructs an `EnrichedEvent` (Kind::Law) → runs the production `build_doc` → clean `Document` (derived title, prose body, stable `bootstrap-` id) → upsert + delete old id. Works for ALL 240 across ALL 3 sources (`.claude/rules`/`docs/specs`/AGENTS) with no source dir + no data-loss risk. `--dry-run`/`--json`; 2 unit tests. (`laws-reindex` from §1/§2 remains for the forward write path + a source-scoped guard.)
- [x] 3.2 DONE (live): `laws-repair` repaired 240→240 — `cortex_laws` now all `bootstrap-`-keyed, **0** `title==id`, **0** stringified-JSON bodies, real prose bodies + `law_id: title` titles; `doctor-content-addressable --index cortex_laws` → ok exit 0; keyword search `q="task sequence"` matches FOLLOW-TASK-SEQUENCE by body. No data loss (240→240).

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation. DONE: CHANGELOG `Fixed` entry updated (governance routing → definitions-only + the `laws-repair` in-place repair, 240→240 verified); spec 08 §Identity repair-tooling section documents `laws-repair`, `laws-reindex` guard, and the governance routing contract.
- [x] 4.2 Write tests covering the new behavior. DONE: routing (`law_definitions_land_in_global_cortex_laws`, `law_violations_stay_per_repo_governance_not_global_cortex_laws`), events_by_kind violation→None, laws-reindex builder tests, laws-repair unwrap+rebuild tests (`rebuild_law_doc_unwraps_payload_to_clean_doc`, `rebuild_is_deterministic_for_same_identity`).
- [x] 4.3 Run tests and confirm they pass. DONE: `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --test-threads=1` green; live `cortex_laws` repair verified (doctor green, search resolves laws).

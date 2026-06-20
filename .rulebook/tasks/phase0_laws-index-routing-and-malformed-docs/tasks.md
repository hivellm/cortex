## 0. Investigation (2026-06-20) — read before deciding §1.1
- The current emit+build path is CORRECT: `emit_spec_laws_imported` produces a clean `LawPayload`-shaped json (`law_id,title,severity,detector,body,section_index,source_path`) that `build_doc`'s `Kind::Law` arm parses into clean `title`/`body`. So the 240 malformed `cortex_laws` docs (`title==id`, body=stringified JSON) are STALE from an old emit path — re-emit through the current path fixes content.
- **Contract conflict across read paths:**
  - `cortex_laws.settings.v1.json` schema + `law_check` (strategies.rs:379) + `law_violations` handler comments → `cortex_laws` = global law DEFINITIONS; violations live per-repo `cortex-<slug>-governance`.
  - BUT `routing::index_for_event_global` dual-writes `Kind::LawViolation` (NOT `Kind::Law`) to `cortex_laws`, and `events_by_kind` (events_by_kind.rs:82) maps BOTH `law` and `violation` queries to `cortex_laws` (its only GLOBAL violation source — violations are otherwise per-repo only).
  - Live `cortex_laws` currently holds 240 DEFINITION docs, 0 violations.
- So definitions don't route to the index `law_check` reads, and there is no clean global home for violations — the tension `events_by_kind` papered over by pointing `violation` at `cortex_laws`.

## 1. Settle the contract + routing
- [ ] 1.1 DECIDE (data-model, user-owned): cortex_laws = definitions-only (violations stay per-repo `governance`; `events_by_kind` violation queries require a repo / a new global violations index) — vs — cortex_laws stays mixed (definitions + violations) and `law_check` filters by kind. Then set routing: add `Kind::Law => cortex_laws`, and keep/drop `Kind::LawViolation => cortex_laws` per the decision.
- [ ] 1.2 Add `Kind::Law` to `routing::index_for_event_global` so law definitions dual-write to `cortex_laws` (mirroring `Kind::Decision`); unit-test the routing

## 2. Fix the emit/build path
- [ ] 2.1 Trace how the 240 malformed docs got `title==id` + stringified-JSON `body`; confirm `emit_{law,spec_laws,extracted_laws}_imported` produce a payload the `build_doc` `Kind::Law` arm parses to clean title/body; fix the bypassing path
- [ ] 2.2 Builder/emit unit test: a law doc has `title != id` and `body` is clean prose (not a stringified JSON object)

## 3. Reindex + verify
- [ ] 3.1 Re-emit law definitions from `.claude/rules` (+ spec-extracted) through the builder with the stable `bootstrap-` key; prune the 240 malformed legacy docs (reuse `decisions-reindex`/`meili-rekey` patterns or a law-specific command)
- [ ] 3.2 Live: `cortex_laws` shows real law title+body, 0 `title==id`, all `bootstrap-`-keyed; `law_check` returns definitions; dashboard law/violation counts stay correct

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (spec 08 governance routing + CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (routing + builder + reindex)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)

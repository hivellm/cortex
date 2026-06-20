## 1. Settle the contract + routing
- [ ] 1.1 Confirm `cortex_laws` (global) is the law-DEFINITION index `law_check` reads; decide whether `LawViolation` stays in `cortex_laws` or moves to a dedicated violations index (so definitions + violations don't collide)
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

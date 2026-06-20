## 1. Diagnose lane coverage
- [x] 1.1 Confirm which Meili index families `free_search` fans out to — was ONLY `repo_scoped(req,"code")` for both vector + keyword; `misc` absent (strategies.rs free_search)
- [x] 1.2 Confirm captured memories index to per-repo `cortex-<repo>-misc` (no global `cortex_memories`); verified the zeta-7731 doc in `cortex-cortex-misc`

## 2. Fix the fan-out
- [x] 2.1 free_search now fans out across `["code","docs","misc"]` for the keyword lane (misc = memory/knowledge/learning)
- [x] 2.2 Same family list applied to the dense (vector) lane; missing per-repo collections return zero hits gracefully (repo_scoped contract)
- [x] 2.3 Live round-trip verified: captured zeta-7731 marker, redeployed cortex-api, `cortex_query free_search` now returns the marker (snippets:5, FOUND:true)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation. DONE: spec 11 §Intent→retrieval-strategy table updated (`free_search` → `cortex-{slug}-{code,docs,misc}` on both lanes) + a note on the `misc`/captured-memory dependency; spec 20 capture_memory requirement notes the `free_search` misc fan-out it depends on; CHANGELOG already carries the fix (phase28 entry, item 2).
- [x] 3.2 Write tests covering the new behavior. DONE: `free_search_fans_out_to_misc_family_for_captured_memories` (strategies.rs) asserts the fused plan selects the `misc` family on BOTH the vector and keyword lanes — the lane-selection contract that makes a captured memory reachable; complemented by the live round-trip in §2.3 (zeta-7731 marker → FOUND).
- [x] 3.3 Run tests and confirm they pass. DONE: docs-only change; `cargo check -p cortex-api` clean + `free_search_*` strategies test green (workspace already verified green this session).

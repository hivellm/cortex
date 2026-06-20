## 1. Diagnose lane coverage
- [x] 1.1 Confirm which Meili index families `free_search` fans out to — was ONLY `repo_scoped(req,"code")` for both vector + keyword; `misc` absent (strategies.rs free_search)
- [x] 1.2 Confirm captured memories index to per-repo `cortex-<repo>-misc` (no global `cortex_memories`); verified the zeta-7731 doc in `cortex-cortex-misc`

## 2. Fix the fan-out
- [x] 2.1 free_search now fans out across `["code","docs","misc"]` for the keyword lane (misc = memory/knowledge/learning)
- [x] 2.2 Same family list applied to the dense (vector) lane; missing per-repo collections return zero hits gracefully (repo_scoped contract)
- [x] 2.3 Live round-trip verified: captured zeta-7731 marker, redeployed cortex-api, `cortex_query free_search` now returns the marker (snippets:5, FOUND:true)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update spec 11 (lane fan-out) + spec 20 (capture_memory contract) + CHANGELOG
- [ ] 3.2 Test: a captured memory is returned by the fused query path (IT against the lane selection)
- [ ] 3.3 Run `cargo check` + `clippy -D warnings` + `cargo test --workspace`

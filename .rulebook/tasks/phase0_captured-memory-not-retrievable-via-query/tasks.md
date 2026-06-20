## 1. Diagnose lane coverage
- [ ] 1.1 Confirm which Meili index families `free_search` (and each intent) fans out to in `crates/cortex-api/src/search/strategies.rs`; verify the `misc` family (memory/knowledge/learning) is absent
- [ ] 1.2 Confirm where captured memories are indexed (per-repo `cortex-<repo>-misc`; no global `cortex_memories`)

## 2. Fix the fan-out
- [ ] 2.1 Add the memory/knowledge/learning (`misc`) family to the keyword lane fan-out for the relevant intents
- [ ] 2.2 Ensure the dense lane covers the memory collection if memories are embedded
- [ ] 2.3 Live round-trip: capture a marker via cortex_capture_memory, then cortex_query free_search returns it

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update spec 11 (lane fan-out) + spec 20 (capture_memory contract) + CHANGELOG
- [ ] 3.2 Test: a captured memory is returned by the fused query path (IT against the lane selection)
- [ ] 3.3 Run `cargo check` + `clippy -D warnings` + `cargo test --workspace`

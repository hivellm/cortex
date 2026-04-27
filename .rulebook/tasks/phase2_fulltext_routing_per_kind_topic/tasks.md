## 1. Locate today's routing
- [ ] 1.1 Find the call site in `cortex-fulltext-worker` that picks the Meili index for an envelope
- [ ] 1.2 Add a unit test that fails today: feed an `artifact` envelope with `topics=["code"]` and assert the chosen index is `cortex-code` (today it returns `cortex-docs`)

## 2. Routing module
- [ ] 2.1 New `crates/cortex-fulltext/src/routing.rs` owning `route_to_index(env, classifier) -> &'static str`
- [ ] 2.2 Encode the matrix from the proposal: kind=decision/law/law_violation/turn/agent_call branches first; then artifact + topics; default `cortex-misc`
- [ ] 2.3 Code-vs-doc tie-break uses the path extension allowlist (`.rs`, `.ts`, `.py`, `.go`, `.js`, `.tsx`, `.jsx`)
- [ ] 2.4 Unit tests cover every branch + the tie-break

## 3. Wire the router into the worker
- [ ] 3.1 Replace the hardcoded index lookup with a `route_to_index()` call
- [ ] 3.2 Add `cortex_fulltext_routed_total{index}` counter incremented per routed event
- [ ] 3.3 Worker creates indexes lazily on first hit per index name; existing ensure_index logic stays

## 4. Backfill + verification
- [ ] 4.1 Drop existing Meili indexes (`cortex-{code,decisions,docs,governance,misc,turns}`)
- [ ] 4.2 Re-run `cortex-bootstrap` against the 17 Hive repos
- [ ] 4.3 Assert all 6 indexes have non-zero `numberOfDocuments` after drain (cortex-misc may stay small but must be > 0)
- [ ] 4.4 Spot-check: a `Vectorizer/benches/*.rs` envelope lands in `cortex-code`, a `docs/decisions/0042.md` lands in `cortex-decisions`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — codify the routing matrix in spec-08
- [ ] 5.2 Write tests covering the new behavior — routing unit tests + integration test seeding mixed events and asserting per-index distribution
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-fulltext`, `cargo clippy -p cortex-fulltext --all-targets -- -D warnings`

## 1. Scope derivation
- [ ] 1.1 Add a regression test in `cortex-pre-thinking::scope::tests` that drives `derive` with a `cwd` inside a git repo lacking `cortex.toml` and asserts `scope.repos` is non-empty
- [ ] 1.2 Fix `derive` to walk ancestors for `.git`, take the parent dir basename as the repo id when `cortex.toml` is absent, and emit it in `scope.repos`
- [ ] 1.3 When `cortex.toml` IS present, prefer its `cortex.id` over the basename; tests cover both branches

## 2. Filter propagation through lanes
- [ ] 2.1 `cortex-api/src/strategies.rs` — every strategy that builds a `KeywordRequest` / `VectorRequest` / `GraphRequest` MUST pass `req.scope.repos` into the lane's `filter`
- [ ] 2.2 Document the filter format each lane expects (Meili `filter`, Vectorizer payload filter, Cypher param)
- [ ] 2.3 Empty `scope.repos` means "no repo filter" (global), not "filter to nothing"

## 3. Response echo
- [ ] 3.1 `QueryResponse.scope_resolved.repos` populates with the canonicalised id used in the filter, never the literal request-side input
- [ ] 3.2 Tests around `cortex-api/src/service.rs` confirm echo

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (extend spec-12 scope derivation section)
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass

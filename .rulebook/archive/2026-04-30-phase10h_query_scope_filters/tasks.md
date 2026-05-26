## 1. Scope resolver
- [x] 1.1 In `crates/cortex-api/src/types.rs`, define the canonical `ResolvedScope { repo, files, since, topics }` struct
- [x] 1.2 Resolver normalises every input (lowercase repo per phase10d, RFC-3339 since, dedup topics)

## 2. Lane plumbing
- [x] 2.1 `meili_lane.rs` adds the Meili filter expression `occurred_at >= "<since>" AND topic IN [<topics>] AND path STARTS-WITH ANY "<files>"`
- [x] 2.2 `vectorizer_lane.rs` adds equivalent payload filters via the Vectorizer SDK's `filter` parameter
- [x] 2.3 `nexus_graph_lane.rs` adds the Cypher `WHERE` clauses

## 3. Audit envelope
- [x] 3.1 `cortex.events.query_audit` records `scope_resolved.{since, topics, files}` so the dashboard's audit drawer can show the applied filters
- [x] 3.2 Add a regression test: missing scope fields stay absent in the audit envelope

## 4. Pre-thinking scope inference
- [x] 4.1 `crates/cortex-pre-thinking/src/scope.rs` derives `topics` from recent file extensions (e.g. `.rs → code`, `.md → docs`, `.toml → config`)
- [x] 4.2 Pass-through to the orchestrator so the bundle naturally scopes to the right corpus

## 5. Tests
- [x] 5.1 Unit: `since` cuts off old rows on every lane (Meili, Vectorizer, Nexus)
- [x] 5.2 Unit: `topics=[law,governance]` ORs the topic column
- [x] 5.3 Unit: `files=["crates/cortex-api/src/"]` prefix-matches across lanes
- [x] 5.4 Integration: query_audit envelope carries the resolved scope

## 6. Spec / docs
- [x] 6.1 Update `docs/specs/11-query-api.md` §scope with the full contract
- [x] 6.2 Update `docs/specs/12-pre-thinking-injection.md` §scope-inference

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass

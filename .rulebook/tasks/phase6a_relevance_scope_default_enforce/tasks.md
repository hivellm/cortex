## 1. Server-side resolver
- [ ] 1.1 Add `pub fn resolve_scope(req: &mut QueryRequest, headers: &HeaderMap) -> Result<ScopeResolution, ScopeError>` in `crates/cortex-api/src/service.rs`
- [ ] 1.2 Resolution order: explicit `request.scope.repo` → `x-cortex-repo` header → `x-cortex-cwd` header (slugified via `cortex_storage::names::slug_for_repo`) → reject
- [ ] 1.3 Define `enum ScopeResolution { Explicit, Header, Cwd, Rejected }` and surface it through `ServiceOutcome` so the audit envelope can record which lane resolved the scope
- [ ] 1.4 Add `enum ScopeError { Missing }` returning `ServiceOutcome::EmptyScope` (new variant) → handler maps to `422 { "reason": "scope_repo_required" }`
- [ ] 1.5 Honour `CORTEX_ALLOW_UNKNOWN_SCOPE=1` env: when set, log `tracing::warn!(caller, "scope_unresolved_fallback")` and pass through with `scope_resolution = "rejected_legacy"`; otherwise return 422

## 2. Audit field
- [ ] 2.1 Extend `cortex-api::audit::AuditEnvelope` with `scope_resolution: String` (one of the four variants)
- [ ] 2.2 Stamp it from the `service.rs` outcome before publishing to `cortex.events.query_audit`
- [ ] 2.3 Update the in-tree audit fixture in `crates/cortex-api/tests/http.rs` to assert the new field is present on a successful response

## 3. MCP server caller
- [ ] 3.1 In `crates/cortex-mcp-server/src/tools.rs`, the `cortex_query` tool already receives `cwd` via the MCP request context — add `x-cortex-cwd` to the outbound HTTP headers when calling `/v1/query`
- [ ] 3.2 When the tool's input includes an explicit `scope.repo`, pass it through unchanged; when not, rely on the cwd header
- [ ] 3.3 Add an integration test in `crates/cortex-mcp-server/tests/` (or extend the existing one) asserting the header is present on the outbound HTTP call

## 4. Dashboard / GUI caller
- [ ] 4.1 In `gui/src/lib/api.ts`, the existing `query()` helper accepts a body — extend it to accept an optional `repo` and emit `x-cortex-repo` when set
- [ ] 4.2 Wire the active sidebar `filters.repo[0]` (when exactly one repo is active) into the helper from `gui/src/views/` callers — no change when `filters.repo` is empty / multi-valued (the user is browsing globally; a 422 is the right error)
- [ ] 4.3 Surface a friendly toast / inline error in the GUI when the dashboard receives `422 scope_repo_required` so the user understands they need to pick a repo

## 5. Spec docs
- [ ] 5.1 In `docs/specs/11-query-api.md`, add `422 scope_repo_required` to the error table and document the resolution order
- [ ] 5.2 In `docs/specs/18-mcp-server.md`, document the `x-cortex-cwd` / `x-cortex-repo` header contract that the MCP server honours
- [ ] 5.3 Note the `CORTEX_ALLOW_UNKNOWN_SCOPE=1` escape hatch + its planned removal at the harness gate (phase6e)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation — extend `docs/specs/11-query-api.md` and `docs/specs/18-mcp-server.md` per §5; cross-link from `docs/analysis/relevance/01-findings.md` §F-003
- [ ] 6.2 Write tests covering the new behavior — unit tests in `service.rs` for each resolution lane and the rejection path; integration test in `crates/cortex-api/tests/http.rs` asserting `422` body shape and audit envelope `scope_resolution` field; MCP-side integration test asserting outbound `x-cortex-cwd` header
- [ ] 6.3 Run tests and confirm they pass — `cargo clippy -p cortex-api -p cortex-mcp-server --all-targets -- -D warnings`, `cargo test -p cortex-api -p cortex-mcp-server`, `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test`

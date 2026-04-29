## 1. Server-side resolver
- [x] 1.1 Add `pub fn resolve_scope(req: &mut QueryRequest, headers: &HeaderMap) -> Result<ScopeResolution, ScopeError>` in `crates/cortex-api/src/service.rs`
- [x] 1.2 Resolution order: explicit `request.scope.repo` → `x-cortex-repo` header → `x-cortex-cwd` header (slugified via `cortex_storage::names::slug_for_repo`) → reject
- [x] 1.3 Define `enum ScopeResolution { Explicit, Header, Cwd, Rejected }` and surface it through `ServiceOutcome` so the audit envelope can record which lane resolved the scope
- [x] 1.4 Add `enum ScopeError { Missing }` returning `ServiceOutcome::EmptyScope` (new variant) → handler maps to `422 { "reason": "scope_repo_required" }`
- [x] 1.5 Honour `CORTEX_ALLOW_UNKNOWN_SCOPE=1` env: when set, log `tracing::warn!(caller, "scope_unresolved_fallback")` and pass through with `scope_resolution = "rejected_legacy"`; otherwise return 422

## 2. Audit field
- [x] 2.1 Extend `cortex-api::audit::AuditEnvelope` with `scope_resolution: String` (one of the four variants)
- [x] 2.2 Stamp it from the `service.rs` outcome before publishing to `cortex.events.query_audit`
- [x] 2.3 Update the in-tree audit fixture in `crates/cortex-api/tests/http.rs` to assert the new field is present on a successful response

## 3. MCP server caller
- [x] 3.1 In `crates/cortex-mcp-server/src/tools.rs`, the `cortex_query` tool already receives `cwd` via the MCP request context — add `x-cortex-cwd` to the outbound HTTP headers when calling `/v1/query`
- [x] 3.2 When the tool's input includes an explicit `scope.repo`, pass it through unchanged; when not, rely on the cwd header
- [x] 3.3 Add an integration test in `crates/cortex-mcp-server/tests/` (or extend the existing one) asserting the header is present on the outbound HTTP call

## 4. Dashboard / GUI caller
- [x] 4.1 In `gui/src/lib/api.ts`, the existing `query()` helper accepts a body — extend it to accept an optional `repo` and emit `x-cortex-repo` when set
- [x] 4.2 Wire the active sidebar `filters.repo[0]` (when exactly one repo is active) into the helper from `gui/src/views/` callers — implemented as a new `gui/src/views/Search.tsx` (the first GUI consumer of `postQuery`); registered under sidebar id `search`; passes `opts.repo` only when `filters.repo` has exactly one entry, undefined for empty / multi-valued (the user is browsing globally; the daemon's 422 is the right error). Vitest cases pin the contract.
- [x] 4.3 Surface a friendly toast / inline error in the GUI when the dashboard receives `422 scope_repo_required` so the user understands they need to pick a repo. Inline alert in `Search.tsx` ("Scope required: Pick exactly one repository in the sidebar so the relevance lane can route to a real collection. …"); covered by the `renders the inline 'Scope required' alert` vitest case in `Search.test.tsx`.

## 5. Spec docs
- [x] 5.1 In `docs/specs/11-query-api.md`, add `422 scope_repo_required` to the error table and document the resolution order
- [x] 5.2 In `docs/specs/18-mcp-server.md`, document the `x-cortex-cwd` / `x-cortex-repo` header contract that the MCP server honours
- [x] 5.3 Note the `CORTEX_ALLOW_UNKNOWN_SCOPE=1` escape hatch + its planned removal at the harness gate (phase6e)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation — extend `docs/specs/11-query-api.md` and `docs/specs/18-mcp-server.md` per §5; cross-link from `docs/analysis/relevance/01-findings.md` §F-003
- [x] 6.2 Write tests covering the new behavior — unit tests in `service.rs` for each resolution lane and the rejection path; integration test in `crates/cortex-api/tests/http.rs` asserting `422` body shape and audit envelope `scope_resolution` field; MCP-side integration test asserting outbound `x-cortex-cwd` header
- [x] 6.3 Run tests and confirm they pass — `cargo test -p cortex-api -p cortex-mcp-server --lib --tests` (118 + 6 lib + 40 + 1 phase6a integration green), `pnpm exec tsc --noEmit` clean, `pnpm test` 10/10 green (5 Timeline + 5 Search; the 5 Search cases pin §4.2 single-repo / empty / multi forwarding plus §4.3 422 + generic-error banner branches). NOTE: `cargo clippy --all-targets -- -D warnings` flags pre-existing `type_complexity` lints in `dashboard.rs` that are out of scope for phase6a; all phase6a-touched files pass clippy individually.

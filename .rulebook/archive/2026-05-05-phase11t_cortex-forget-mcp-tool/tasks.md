## 1. cortex-api `/v1/admin/forget` endpoint

The kickoff audit re-checked the architecture: `cortex-mcp-server` is an MCP-protocol shim that proxies to `cortex-api` over HTTP — it has no direct handle on Vectorizer / Meili / Nexus. The hard-purge cascade lives in `cortex-api` (which already owns those clients) behind a new admin endpoint; the MCP tool POSTs to it.

- [x] 1.1 `crates/cortex-api/src/admin_forget.rs` ships `ForgetRequest { event_id, confirmation_token, dry_run }`, `ForgetResponse`, plus `resolve_target_collections(kind)`. `Kind::Consolidation | None` ⇒ `[cortex.consolidation.fp32, .pq, cortex.cold.binary]`; `Turn` / `ToolCall` / `Artifact` ⇒ per-kind `{fp32, pq}` + cold; other kinds ⇒ `{cortex.consolidation.fp32, cortex.cold.binary}`. 4 unit tests cover the branches.
- [x] 1.2 `LiveNexusPurger` in `admin_forget.rs`. `delete_node_by_event_id(event_id)` runs `MATCH (n { event_id: $id }) DETACH DELETE n` via the shared `nexus_sdk::NexusClient`; zero-row delete is `Ok(())`.
- [x] 1.3 `LiveArchivePurger` walks the parquet tree (string-level pre-filter + envelope decode confirmation), rewrites each partition through `*.parquet.tmp` + atomic rename. 2 unit tests: hit rewrites partition without the row, miss is a no-op.
- [x] 1.4 axum handler `POST /v1/admin/forget` mounted in `crates/cortex-api/src/http.rs`. Reads kind via `scan_envelope_by_event_id`; missing envelope falls through to the consolidation triple. Dry-run short-circuits with the projection. Real run forwards to `cortex_workers::pruner::purge::forget`. Bad-token returns HTTP 400 + structured reason. 3 unit tests: dry-run no-op, missing-token 400, happy-path full cascade.

## 2. MCP tool wrapper

- [x] 2.1 `ForgetTool` lives at `crates/cortex-mcp-server/src/tools.rs`. `name() -> "cortex_forget"` (no `.` per MCP spec). `descriptor()` returns JSON-schema with `{event_id, confirmation_token, dry_run}` and an explicit `irreversible` advisory in the description string.
- [x] 2.2 `call(ctx, args)` POSTs to `<api_url>/v1/admin/forget`; 200 round-trips as `ToolResult::ok`; 400 maps to `ToolError::invalid_input` carrying the canonical mismatch message; other non-success surfaces as a soft-error so the MCP host renders it without aborting the session.
- [x] 2.3 Registered in `ToolRegistry::default_set()` (registry size 6 → 7). `tools/list` test + transport_stdio round-trip test bumped to 7.

## 3. Tests

- [x] 3.1 `forget_tool_rejects_missing_confirmation_token` (cortex-mcp-server lib test, wiremock-backed). Confirms the cortex-api 400 → MCP `ToolError::invalid_input` mapping.
- [x] 3.2 `forget_tool_dry_run_round_trips_projection` (cortex-mcp-server lib test, wiremock-backed). Confirms the dry-run shape surfaces unchanged.
- [x] 3.3 Live IT at `crates/cortex-mcp-server/tests/forget_it.rs` gated on `CORTEX_FORGET_IT=1`. Asserts dry-run round-trip + missing-token rejection against the live `cortex-api`. Default `cargo test` returns early via the gate; running with `CORTEX_FORGET_IT=1` requires a live stack at `CORTEX_API_URL`.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 4.1 Update or create documentation covering the implementation — `docs/specs/20-mcp-tool-surface.md` registry table gained the `cortex_forget` row (write/irreversible). CHANGELOG `[Unreleased] § Added` entry covers §1-§3 in one paragraph above the phase11p block.
- [x] 4.2 Write tests covering the new behavior — 9 admin_forget unit tests + 2 MCP tool wiremock tests + 1 gated IT. Plus the registry/transport_stdio counters bumped to 7.
- [x] 4.3 Run tests and confirm they pass — `cargo check --workspace` clean. `cargo test -p cortex-api --lib admin_forget` → 9/9. `cargo test -p cortex-mcp-server --lib` → 46/46 (incl. registry/transport_stdio expansion). `cargo test -p cortex-mcp-server --test forget_it` → 1/1 (gate-disabled path returns Ok); `CORTEX_FORGET_IT=1` is the live-stack gate.

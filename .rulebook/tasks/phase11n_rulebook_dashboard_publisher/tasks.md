## 1. Coordination — @hivellm/rulebook (external)

- [ ] 1.1 Open an upstream issue / PR in `hivellm/rulebook` requesting the Synap publisher hook described in this task's proposal §1–§4.
- [ ] 1.2 Pin the released `@hivellm/rulebook` version that contains the publisher in this repo's `package.json` / `.mcp.json`.

## 2. Cortex-side wiring

- [ ] 2.1 Resolve Synap connection details (URL, auth) for the dashboard stream consumer; reuse the existing `CORTEX_API_SYNAP_URL` env var.
- [ ] 2.2 In `crates/cortex-api/src/main.rs`, spawn a Synap pull loop alongside the file-system watcher; feed `DashboardEventBus` via the existing `dashboard_consumer.rs::ingest_event` path. Gate behind `CORTEX_DASHBOARD_SYNAP_CONSUME=1` (default `1`).
- [ ] 2.3 Connection-loss handling: exponential back-off, tracing on every reconnect attempt, never panic.
- [ ] 2.4 Smoke integration test: the rulebook MCP publishes a `memory.appended` event; the cortex-api SSE client receives it within 1 s.

## 3. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 3.1 Update or create documentation covering the implementation — flip `docs/specs/21-dashboard-push.md` status from Draft → Implemented; document the cortex-side env var(s) in `crates/cortex-api/README.md`.
- [ ] 3.2 Write tests covering the new behavior — Synap-side smoke (mock or real) + a regression test that the watcher path still fires when the consumer is disabled.
- [ ] 3.3 Run tests and confirm they pass — `cargo check -p cortex-api`, `cargo clippy --workspace -- -D warnings`, `cargo test -p cortex-api`. All green before archive.

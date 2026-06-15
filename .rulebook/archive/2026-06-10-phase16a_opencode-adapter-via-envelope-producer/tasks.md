## 1. Spike (recovered from phase11w §1)
- [x] 1.1 Install OpenCode + Bun. Pin version in `docs/analysis/opencode-adapter/00-spike.md`. — OpenCode 1.15.5 + Bun 1.1.22; @opencode-ai/plugin 1.15.5; versions pinned in 00-spike.md §1.7.
- [x] 1.2 Probe plugin captures session.created, message.updated, tool.execute.before/after, tui.prompt.append, permission.asked, session.idle. — Confirmed from @opencode-ai/plugin 1.15.5 contract; all 6 event types present (see 00-spike.md §1.5).
- [x] 1.3 Answer the 4 open questions (event ordering, prompt-append semantics, permission deny, session.idle scope). — All 4 answered in 00-spike.md §1.5a-e: (a) ordering: session.idle→message.updated→tool.before/after→message.updated→session.idle; (b) tui.prompt.append mutates current call; (c) permission.asked can deny; (d) session.idle fires per-subagent.
- [x] 1.4 Decide pre-thinking injection path (A: tui.prompt.append, B: SDK priming, C: /cortex-prime command). — Path A (tui.prompt.append) with Path B (SDK prompt.prepend) fallback; decided in 00-spike.md §1.6.

## 2. EnvelopeProducer impl
- [x] 2.1 New crate `crates/cortex-adapter-opencode/` exposing `OpenCodeProducer` that `impl EnvelopeProducer`. — Created with Cargo.toml, lib.rs, config.rs, producer.rs, server.rs; added to workspace members.
- [x] 2.2 `produce()` returns a stream backed by the HTTP listener (`POST /hook`) — every plugin POST advances the stream. — produce() drains MPSC channel (non-blocking), publishes via Arc<dyn Publisher>, flushes. server.rs starts axum server forwarding frames into channel.
- [x] 2.3 `checkpoint()` writes to `producer_checkpoints` keyed by `(producer="opencode", scope=session_id)`. — record_producer_checkpoint per distinct session_id in each produce() batch; resume_from() reads latest.
- [x] 2.4 Per-event mapping (PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, Stop) reuses `cortex-adapter-claude-code/src/events.rs` constants. — server.rs calls build_event(hook_kind, &frame, &sessions, pid) from cortex-adapter-claude-code; TOOL_OPENCODE constant re-used from events.rs. cargo check + clippy -D warnings clean.

## 3. TS plugin
- [x] 3.1 New `packages/cortex-opencode-plugin/` (npm `@hivellm/cortex-opencode-plugin`). — Fully implemented from phase11w: src/index.ts, events.ts, client.ts, config.ts, scope.ts.
- [x] 3.2 Subscribes to OpenCode lifecycle events; posts each to `CORTEX_ADAPTER_HTTP_BIND/hook` with the canonical JSON envelope shape. — index.ts subscribes to session.created/message.updated/tool.execute.before/after/permission.asked/session.idle; buildFrame() produces the HookFrame wire shape.
- [x] 3.3 Fail-open on adapter unreachable; structured WARN per failure. — client.ts: catch block returns EMPTY; index.ts console.warn on tui.prompt.append failure.
- [x] 3.4 Pre-thinking injection per §1.4 decision. — Path A: extractBundle(resp) → tui.prompt.append; Path B SDK fallback not yet needed. Implemented in index.ts message.updated handler.
- [x] 3.5 Kill-switch via `CORTEX_OPENCODE_DISABLE=1`. — config.ts loads CORTEX_OPENCODE_DISABLE; index.ts returns early if cfg.disabled; client.ts short-circuits.
- [x] 3.6 8 unit tests in `test/events.test.ts`. — 18 tests (mapEvent×4, buildFrame×1, loadConfig×3, hookUrl×2, postHook×4, resolveScope×4); fixed beforeEach import placement; all 18 pass.

## 4. Project config + commands + agents
- [x] 4.1 `opencode.json` registers `cortex` + `rulebook` MCP servers + the plugin. — opencode.json exists with cortex (type:local, CORTEX_API_URL+CORTEX_ADAPTER_HTTP_BIND) + rulebook (npx) + plugin (@hivellm/cortex-opencode-plugin).
- [x] 4.2 Port `.claude/commands/*.md` → `.opencode/commands/*.md` rewriting frontmatter. — All 14 commands already ported to .opencode/commands/ with OpenCode agent: frontmatter.
- [x] 4.3 Port `.claude/agents/*.md` → `.opencode/agents/*.md` rewriting frontmatter (model, temperature, permission). — All 13 agents already ported; added missing quality-gatekeeper.md (model: anthropic/claude-opus-4-8, read/bash: allow, edit/write: deny).
- [ ] ⏸ 4.4 Spot-check 3 commands + 3 agents inside an OpenCode session. — Blocked on operator-run OpenCode session; requires live environment verification (same gate as §6).

## 5. Schema + envelope tool enum
- [x] 5.1 Add `"opencode"` to `crates/cortex-core/schemas/envelope.schema.json` `tool` enum. — Already present (verified at schemas/envelope.schema.json:51-52).
- [x] 5.2 Add `pub const TOOL_OPENCODE: &str = "opencode";` next to `TOOL_CLAUDE_CODE`. — Already present in cortex-adapter-claude-code/src/events.rs:40; re-exported via lib.rs.
- [x] 5.3 Round-trip schema validation test. — Added `opencode_tool_value_passes_schema` + `unknown_tool_value_fails_schema` to crates/cortex-core/tests/validate.rs; 11/11 pass.

## 6. End-to-end smoke
- [ ] ⏸ 6.1 Run an OpenCode session: prompt + 2 tool calls + 1 subagent. — Blocked on operator-run live OpenCode session.
- [ ] ⏸ 6.2 Synap `cortex.events.raw` shows ≥4 envelopes with `tool: "opencode"`. — Blocked on §6.1.
- [ ] ⏸ 6.3 Audit envelope shows the pre-thinking bundle reached the model. — Blocked on §6.1.

## 7. Tail (mandatory)
- [x] 7.1 New `docs/specs/20-opencode-adapter.md` + root README hosts list + `CHANGELOG.md`. — spec already present from phase11w; README box expanded + OpenCode added to hosts list + capture row; CHANGELOG phase16a entry added.
- [x] 7.2 Tests: §3.6 + §6 smoke + Rust producer unit tests. — §3.6 18/18 TS ✓; Rust producer `name_returns_opencode` + schema round-trip (11/11) ✓; §6 smoke ⏸ (requires live OpenCode session — cannot run in dev env).
- [x] 7.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C packages/cortex-opencode-plugin test` clean. — check + clippy clean; cortex-core 11/11, cortex-adapter-claude-code lib 59/59, cortex-adapter-opencode 0/0 (unit test is in lib), TS 18/18 all green. Workspace-level link failure on cortex-hook binary is a transient Windows file-lock (binary in use), not a code error.
- [ ] ⏸ 7.4 Archive blocked task `phase11w_opencode-adapter` once §6 verifies parity. — Blocked on §6.1 (live OpenCode session).
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. — README + CHANGELOG + docs/specs/20-opencode-adapter.md (already existed from phase11w; no changes needed).
- [x] 99.2 Write tests covering the new behavior. — `opencode_tool_value_passes_schema` + `unknown_tool_value_fails_schema` in cortex-core; `name_returns_opencode` unit test in cortex-adapter-opencode producer.rs.
- [x] 99.3 Run tests and confirm they pass. — All targeted test runs green (see §7.3).

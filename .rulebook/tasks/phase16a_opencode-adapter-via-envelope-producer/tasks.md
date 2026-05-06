## 1. Spike (recovered from phase11w §1)
- [ ] 1.1 Install OpenCode + Bun. Pin version in `docs/analysis/opencode-adapter/00-spike.md`.
- [ ] 1.2 Probe plugin captures session.created, message.updated, tool.execute.before/after, tui.prompt.append, permission.asked, session.idle.
- [ ] 1.3 Answer the 4 open questions (event ordering, prompt-append semantics, permission deny, session.idle scope).
- [ ] 1.4 Decide pre-thinking injection path (A: tui.prompt.append, B: SDK priming, C: /cortex-prime command).

## 2. EnvelopeProducer impl
- [ ] 2.1 New crate `crates/cortex-adapter-opencode/` exposing `OpenCodeProducer` that `impl EnvelopeProducer`.
- [ ] 2.2 `produce()` returns a stream backed by the HTTP listener (`POST /hook`) — every plugin POST advances the stream.
- [ ] 2.3 `checkpoint()` writes to `producer_checkpoints` keyed by `(producer="opencode", scope=session_id)`.
- [ ] 2.4 Per-event mapping (PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, Stop) reuses `cortex-adapter-claude-code/src/events.rs` constants.

## 3. TS plugin
- [ ] 3.1 New `packages/cortex-opencode-plugin/` (npm `@hivellm/cortex-opencode-plugin`).
- [ ] 3.2 Subscribes to OpenCode lifecycle events; posts each to `CORTEX_ADAPTER_HTTP_BIND/hook` with the canonical JSON envelope shape.
- [ ] 3.3 Fail-open on adapter unreachable; structured WARN per failure.
- [ ] 3.4 Pre-thinking injection per §1.4 decision.
- [ ] 3.5 Kill-switch via `CORTEX_OPENCODE_DISABLE=1`.
- [ ] 3.6 8 unit tests in `test/events.test.ts`.

## 4. Project config + commands + agents
- [ ] 4.1 `opencode.json` registers `cortex` + `rulebook` MCP servers + the plugin.
- [ ] 4.2 Port `.claude/commands/*.md` → `.opencode/commands/*.md` rewriting frontmatter.
- [ ] 4.3 Port `.claude/agents/*.md` → `.opencode/agents/*.md` rewriting frontmatter (model, temperature, permission).
- [ ] 4.4 Spot-check 3 commands + 3 agents inside an OpenCode session.

## 5. Schema + envelope tool enum
- [ ] 5.1 Add `"opencode"` to `crates/cortex-core/schemas/envelope.schema.json` `tool` enum.
- [ ] 5.2 Add `pub const TOOL_OPENCODE: &str = "opencode";` next to `TOOL_CLAUDE_CODE`.
- [ ] 5.3 Round-trip schema validation test.

## 6. End-to-end smoke
- [ ] 6.1 Run an OpenCode session: prompt + 2 tool calls + 1 subagent.
- [ ] 6.2 Synap `cortex.events.raw` shows ≥4 envelopes with `tool: "opencode"`.
- [ ] 6.3 Audit envelope shows the pre-thinking bundle reached the model.

## 7. Tail (mandatory)
- [ ] 7.1 New `docs/specs/20-opencode-adapter.md` + root README hosts list + `CHANGELOG.md`.
- [ ] 7.2 Tests: §3.6 + §6 smoke + Rust producer unit tests.
- [ ] 7.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C packages/cortex-opencode-plugin test` clean.
- [ ] 7.4 Archive blocked task `phase11w_opencode-adapter` once §6 verifies parity.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

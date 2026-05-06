## 1. Schema + producer scaffold
- [ ] 1.1 Add `"cursor"` to envelope `tool` enum + `pub const TOOL_CURSOR`.
- [ ] 1.2 New crate `crates/cortex-adapter-cursor/` with `CursorProducer` that `impl EnvelopeProducer`.
- [ ] 1.3 HTTP listener bind defaults to `127.0.0.1:17005` (distinct from OpenCode's 17004).

## 2. Cursor integration
- [ ] 2.1 Document the integration path in `docs/specs/17-additional-adapters.md` § Cursor.
- [ ] 2.2 Add `.cursor/rules/cortex-capture.mdc` rule that wraps tool invocations with a hook POST.
- [ ] 2.3 Wrapper script `scripts/cursor-hook.sh` (+ `.ps1`) translates Cursor's tool-call payloads into the canonical hook JSON.

## 3. Commands + agents port
- [ ] 3.1 Port `.claude/commands/*.md` → `.cursor/commands/*.md` (Cursor uses MDC-style frontmatter; rewrite accordingly).
- [ ] 3.2 Cursor's agent surface is more limited; document which `.claude/agents/` map to Cursor "modes" and which fall outside the host's surface.

## 4. End-to-end smoke
- [ ] 4.1 Run a Cursor session: prompt + 2 tool calls.
- [ ] 4.2 Synap `cortex.events.raw` shows ≥3 envelopes with `tool: "cursor"`.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/17-additional-adapters.md` + root README + `CHANGELOG.md`.
- [ ] 5.2 Tests: producer unit tests + §4 smoke.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

## 1. Schema + producer
- [ ] 1.1 Add `"codex"` to envelope `tool` enum + `pub const TOOL_CODEX`.
- [ ] 1.2 New crate `crates/cortex-adapter-codex/` with `CodexProducer` that `impl EnvelopeProducer`. HTTP bind default `127.0.0.1:17006`.

## 2. Wrapper CLI
- [ ] 2.1 New `scripts/cortex-codex.{sh,ps1}` wrapping the `codex` binary.
- [ ] 2.2 Streams stdout/stderr to the producer's hook endpoint as Turn / ToolCall envelopes.
- [ ] 2.3 Handles process termination: SessionEnd envelope on exit.

## 3. End-to-end smoke
- [ ] 3.1 Run a Codex session via the wrapper: prompt + 2 tool calls.
- [ ] 3.2 Synap shows ≥3 envelopes with `tool: "codex"`.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/17-additional-adapters.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: producer unit + §3 smoke.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

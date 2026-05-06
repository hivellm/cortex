## 1. Schema + producer
- [ ] 1.1 Add `"gemini"` to envelope `tool` enum + `pub const TOOL_GEMINI`.
- [ ] 1.2 New crate `crates/cortex-adapter-gemini/` with `GeminiProducer` that `impl EnvelopeProducer`. HTTP bind default `127.0.0.1:17007`.

## 2. Wrapper CLI
- [ ] 2.1 New `scripts/cortex-gemini.{sh,ps1}` wrapping the `gemini` binary.
- [ ] 2.2 Streams stdout/stderr to the producer's hook endpoint.
- [ ] 2.3 SessionEnd envelope on process exit.

## 3. End-to-end smoke
- [ ] 3.1 Run a Gemini session via the wrapper.
- [ ] 3.2 Synap shows ≥3 envelopes with `tool: "gemini"`.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/17-additional-adapters.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: producer unit + §3 smoke.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

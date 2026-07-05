## 1. OpenCode adapter — confirm and close the validation gap
- [ ] 1.1 Review `crates/cortex-adapter-opencode/` + `packages/cortex-opencode-plugin/` against ADR-017's design; confirm the `EnvelopeProducer` usage is still correct after phase16a
- [ ] 1.2 Fix any drift found in 1.1 (no rewrite expected — phase16a shipped this code-complete)
- [ ] 1.3 Confirm the live OpenCode session (item 3) is back in scope — phase16b closed it WON'T-DO on 2026-06-22 by explicit operator decision to deprioritize OpenCode

## 2. Cursor adapter — build per spec 17
- [ ] 2.1 Extract or confirm `cortex-adapters/common/` (IPC, publisher, session correlation, redaction) exists for a Cursor crate to build on
- [ ] 2.2 New `crates/cortex-adapter-cursor/` implementing `EnvelopeProducer`, file-watcher based (Cursor has no `PreToolUse`-equivalent hook)
- [ ] 2.3 Edit inference via workspace filesystem watch, tagged `edit_inferred`
- [ ] 2.4 Pre-thinking injection via a rewritten `_cortex_context.md`; governance observational-only (no blocking laws)

## 3. Live verification
- [ ] 3.1 Run a real OpenCode session; confirm events reach Synap with `tool: "opencode"`, get classified/embedded/indexed, and are retrievable via `cortex_query`/`cortex_pre_thinking`
- [ ] 3.2 Run a real Cursor session; confirm the same end-to-end path
- [ ] 3.3 Close phase16a §4.4/§6.1-6.3 and phase16b's re-opened validation once 3.1 passes; revisit whether `phase11w_opencode-adapter` can now archive

## 4. Docs and goal status
- [ ] 4.1 Update `docs/architecture.md`'s capability table and Goal G1's status once §3 verifies both adapters
- [ ] 4.2 Do not overclaim "100%" — note Codex/Gemini as a separate follow-up, out of scope here

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass

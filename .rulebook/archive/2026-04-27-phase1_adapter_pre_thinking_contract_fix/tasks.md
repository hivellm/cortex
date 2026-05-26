## 1. Wire pre-thinking pipeline into adapter
- [x] 1.1 Add `cortex-api` and `cortex-pre-thinking` to `crates/cortex-adapter-claude-code/Cargo.toml`
- [x] 1.2 Replace `SyncClient::pre_thinking` to drive `cortex_pre_thinking::pipeline::run` via a `QueryFn` impl that POSTs a real `QueryRequest`
- [x] 1.3 Drop the obsolete `PreThinkingRequest` struct; rely on `cortex_api::QueryRequest` for the wire shape
- [x] 1.4 `cargo check -p cortex-adapter-claude-code` clean

## 2. Fix hook response contract
- [x] 2.1 Reshape `HookResponse` in `dispatcher.rs` to emit `hookSpecificOutput.additionalContext` (string) and `permissionDecision` / `permissionDecisionReason` (camelCase)
- [x] 2.2 Carry the markdown `String` from pipeline output through dispatcher to the response
- [x] 2.3 `cargo check -p cortex-adapter-claude-code` clean

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation
- [x] 3.2 Write tests covering the new behavior
- [x] 3.3 Run tests and confirm they pass

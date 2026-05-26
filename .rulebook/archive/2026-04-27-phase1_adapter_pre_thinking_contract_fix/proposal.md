# Proposal: phase1_adapter_pre_thinking_contract_fix

## Why

The `cortex-adapter-claude-code` daemon's `UserPromptSubmit` sync path is dead-on-arrival. It POSTs a request shape that the cortex-api `/v1/query` handler rejects with HTTP 422 on every call, then silently returns an empty `additionalContext` to Claude Code. Reproduced manually against the running daemon (PID 93544) with a representative prompt: API returns `Failed to deserialize the JSON body into the target type: missing field 'intent'`.

Three concrete bugs:

1. **Wrong request shape** — `crates/cortex-adapter-claude-code/src/sync_paths.rs:75-86` declares `PreThinkingRequest { prompt, session_id, cwd, max_bundle_bytes }` and POSTs it to `/v1/query?intent=pre_change_context`. But `crates/cortex-api/src/http.rs:86-90` consumes `Json<QueryRequest>` and ignores the URL query string; `crates/cortex-api/src/types.rs:78-98` requires `intent` and `query` (not `prompt`) in the JSON body.

2. **Pre-thinking pipeline not wired** — `crates/cortex-pre-thinking` already implements scope derivation, intent selection, query, format, and budget clipping (per spec-12), producing a markdown bundle ready for `additionalContext`. The adapter doesn't depend on this crate at all (`Cargo.toml` lines 17-34 only pull `cortex-core`); it has its own malformed HTTP client instead.

3. **Wrong hook response shape** — `crates/cortex-adapter-claude-code/src/dispatcher.rs:20-31` serializes `additional_context` (snake_case, JSON object). Claude Code's `UserPromptSubmit` hook contract expects `hookSpecificOutput.additionalContext` as a string. Even if (1) and (2) were fixed, Claude Code would silently drop the payload.

Net effect today: every `UserPromptSubmit` results in a `{}` response and zero context enrichment ever reaches the model. The MemoryKeywordLane is happily seeded from the archive every 30s, the API responds, the orchestrator works — but the bridge from Claude Code to that data is broken.

## What Changes

- Add `cortex-api` and `cortex-pre-thinking` as dependencies of `cortex-adapter-claude-code`.
- Replace `SyncClient::pre_thinking` with a wrapper around `cortex_pre_thinking::pipeline::run`, providing a `QueryFn` impl that POSTs a properly-shaped `QueryRequest` (intent in body, `query` field) and parses `QueryResponse`.
- Change `HookResponse` to serialize as `{ "hookSpecificOutput": { "hookEventName": "UserPromptSubmit", "additionalContext": "<markdown>" } }` for prompt-submit, with `permissionDecision` + `permissionDecisionReason` for tool deny — both camelCase to match the Claude Code hook contract.
- The adapter's pre-thinking output becomes a markdown `String` (already produced by the pipeline's `format_bundle` + `clip_to_budget`), not a JSON object.
- Update tests in `sync_paths.rs` and `dispatcher.rs` to lock the new shapes.

Out of scope (split into a follow-up task): live `VectorLane` / `GraphLane` impls in `cortex-api` (currently `MemoryVectorLane` / `MemoryGraphLane` test-doubles in `crates/cortex-api/src/main.rs:40-42`). The keyword lane's archive seeding gives the bulk of the value once the contract is fixed.

## Impact

- Affected specs: spec-10 (Claude Code adapter sync paths), spec-12 (pre-thinking pipeline integration)
- Affected code:
  - `crates/cortex-adapter-claude-code/Cargo.toml`
  - `crates/cortex-adapter-claude-code/src/sync_paths.rs`
  - `crates/cortex-adapter-claude-code/src/dispatcher.rs`
  - `crates/cortex-adapter-claude-code/src/ipc.rs` (response serialization touch-points only if needed)
- Breaking change: NO (internal contract; no external API surface changes — cortex-api `/v1/query` shape stays untouched)
- User benefit: pre-thinking enrichment actually reaches the model. Active laws, prior decisions, similar past turns, and matching snippets get injected into `additionalContext` on every `UserPromptSubmit`.

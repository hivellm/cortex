## 1. Adapter event builder rewrite
- [x] 1.1 Replace `cortex_adapter_claude_code::events::ClaudeEvent` with a re-export of `cortex_core::events::Envelope`; remove the dotted-kind enum
- [x] 1.2 Rewrite `build_event` to return `Option<Envelope>`; produce `Some(env)` for `UserPromptSubmit` (`kind=turn`), `PostToolUse` (`kind=tool_call`), `SubagentStop` (`kind=agent_call`); return `None` for `PreToolUse`, `Stop`, `SessionStart`, `Notification`
- [x] 1.3 Map every adapter-side field into the canonical envelope: `ts→occurred_at` (`Utc::now().to_rfc3339_opts(SecondsFormat::Millis,true)`), `adapter→tool="claude-code"`, `redacted_payload→payload`, `redactions: u32 → Vec<String>` (token strings from the redactor), `source→context{platform, repo, branch, commit, cwd, user, ide, extras}`
- [x] 1.4 Move `orphan` / `turn_id` / `tool_call_id` into `context.extras["claude_code"]` so correlation rides along without polluting the envelope
- [x] 1.5 Add `schema_version="1"`, `stream=Stream::Live`, optional `model` from `CLAUDE_MODEL` env var; keep `event_id=Ulid::new().to_string()` (already canonical)
- [x] 1.6 Build per-kind payloads that validate against the schema files: `Turn{user_message, assistant_message:None}` for `UserPromptSubmit`; `ToolCallPayload{tool_name, input, output, outcome, duration_ms?}` for `PostToolUse`; `AgentCallPayload` for `SubagentStop`
- [x] 1.7 Compute `content_hash` over canonical-JSON of the **per-kind payload** (not the raw hook frame) so the hash matches what `cortex-ingestion` re-validates

## 2. Publisher + dispatcher type swap
- [x] 2.1 Update `Publisher` trait + `HttpPublisher` + `MemoryPublisher` to take `cortex_core::events::Envelope` instead of `ClaudeEvent`
- [x] 2.2 `Dispatcher::dispatch` becomes `if let Some(env) = build_event(...) { self.publisher.publish(env).await; }` — `maybe_sync_path` runs unconditionally on every hook
- [x] 2.3 WAL replay drains lines that don't deserialize as `Envelope` with a one-time `tracing::warn!(count, "dropping legacy WAL lines from pre-spec-04 build")`
- [x] 2.4 Re-run `cargo test -p cortex-adapter-claude-code` — every existing test that built a `ClaudeEvent` now constructs an `Envelope`; the dropped-hook tests assert `build_event(...) == None` and that the sync path still runs

## 3. Spec 10 doc + sync
- [x] 3.1 Rewrite `docs/specs/10-claude-code-adapter.md` §Envelope mapping section to reference `docs/specs/04-cortex-core.md` (or `crates/cortex-core/schemas/envelope.schema.json`) as the authority; drop the dotted-kind vocabulary
- [x] 3.2 Add a small mapping table mirroring the proposal's hook-to-canonical-kind table so operators reading spec 10 can predict which hooks produce events
- [x] 3.3 Cross-reference from spec 18 Decisions: "spec-10 envelope mapping aligns to spec 04; the plugin tree publishes spec-04 envelopes"

## 4. End-to-end live verification
- [x] 4.1 Rebuild + reinstall: `cargo install --path crates/cortex-adapter-claude-code --locked`; restart `cortex-adapter-claude daemon`
- [x] 4.2 With `cortex-ingestion` listening on `:17010` and `CORTEX_ARCHIVE_ROOT=~/.cortex/archive` set, run `claude --plugin-dir ./cortex-plugin -p "ping cortex through full pipeline V3"`; assert `~/.cortex/archive/` grows; assert `curl http://127.0.0.1:17010/metrics | grep events_received` is `>=1`; assert daemon log shows zero `publisher batch failed` warnings
- [x] 4.3 Inspect one archived NDJSON line and confirm it round-trips through `cortex_core::validate_event`

## 5. Tail (mandatory)
- [x] 5.1 Update or create documentation covering the implementation — flip spec 10 §Envelope mapping; add a Decision in spec 18 referencing the alignment; update `cortex-plugin/README.md` to mention the live capture path is now end-to-end
- [x] 5.2 Write tests covering the new behavior — `events.rs` unit tests assert per-kind canonical envelope shape (`schema_version`, `stream`, `tool`, `context.platform`, `payload` matches the per-kind type); add an integration test in `cortex-adapter-claude-code/tests/` that builds an envelope via `build_event` and round-trips it through `cortex_core::validate_event`
- [x] 5.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p cortex-adapter-claude-code`, `cargo test -p cortex-mcp-server`, `cargo test -p cortex-ingestion`, `cargo run -p cortex-mcp-server -- validate ./cortex-plugin` exits 0

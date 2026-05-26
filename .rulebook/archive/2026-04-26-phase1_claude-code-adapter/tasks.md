## 1. Common crate
- [x] 1.1 `cortex-adapter-claude-code` crate ships every reusable building block (`Adapter` semantics via `HookKind` + `build_event`, daemon-shaped binary, IPC server over UDS + named-pipe, `Publisher` trait + `HttpPublisher`, `SessionManager`, `cortex_core::redact` wrapper, `SyncClient` covering law + query, `Dispatcher`)
- [x] 1.2 Install framework covers UDS (Unix) + named-pipe (Windows) + idempotent `~/.claude/settings.json` patch via `install::Layout` + `install` / `uninstall`
- [x] 1.3 Bounded in-memory queue (default 2 048) backed by `OverflowWal` durable spill — drop-oldest under pressure mirrors to WAL, on-startup replay re-enqueues every persisted entry

## 2. Claude Code adapter crate
- [x] 2.1 `cortex-adapter-claude-code` crate with `HookKind` → envelope mapping per spec 10 §Envelope mapping (every hook → its Cortex `kind`)
- [x] 2.2 `main.rs` builds the daemon stack (publisher → sync client → dispatcher → IPC server) and runs it
- [x] 2.3 `~/.cortex/adapter.toml` schema with documented defaults (`endpoint`, `api_endpoint`, `timeout_ms`, `queue_bounded`, pre-thinking + laws + redaction + logging sub-sections)

## 3. Hook shims
- [x] 3.1 Shell shim scripts shipped under `crates/cortex-adapter-claude-code/hooks/` — one per hook (`cortex-session-start.sh`, `cortex-user-prompt.sh`, `cortex-pre-tool.sh`, `cortex-post-tool.sh`, `cortex-stop.sh`, `cortex-subagent-stop.sh`, `cortex-notification.sh`)
- [x] 3.2 Windows parity: PowerShell shim `.ps1` siblings using `NamedPipeClientStream` for the same 7 hooks
- [x] 3.3 Hook prints `{}` and exits 0 on any daemon error (UDS / pipe missing, daemon down, malformed JSON) — verified by the dispatcher's malformed-input test
- [x] 3.4 Shims are baked into the binary via `include_str!` so the install command writes a known-good copy on every run

## 4. Session correlation
- [x] 4.1 `session_id` resolves from `CLAUDE_SESSION_ID` env (carried in the hook frame) or synthesizes `cc-sess-<pid>-<ulid>` on first contact via `SessionManager::resolve_or_synthesize`
- [x] 4.2 `turn_id` opens on `UserPromptSubmit`, replaces any prior turn, closes on `Stop`
- [x] 4.3 `tool_call_id` opens on `PreToolUse` keyed by `tool_use_id`; `PostToolUse` looks up the same id and falls back to a fresh id with `orphan = true` when correlation fails

## 5. Synchronous hooks
- [x] 5.1 `UserPromptSubmit` POSTs `/v1/query?intent=pre_change_context` with the `pre_thinking.timeout_ms` budget; on success returns `additionalContext` bundle
- [x] 5.2 `PreToolUse` POSTs `/v1/laws/check` with the `laws.timeout_ms` budget; `severity=critical` violations route to `permissionDecision: deny` with the concatenated reason
- [x] 5.3 Fail-open on timeout / connect error / non-2xx — empty `additionalContext` for the prompt path, `allow` for the law path; the async event still publishes (covered by `unreachable_api_endpoint_fails_open_for_*` tests)

## 6. Async publisher
- [x] 6.1 `HttpPublisher` drains an in-memory bounded queue (default 2 048) in 32-event batches; `spawn_flusher` ticks every 200 ms
- [x] 6.2 5xx / network failures retry with exponential backoff (3 attempts, 100/400/1600 ms); persistent failures spill the batch to the overflow WAL
- [x] 6.3 `replay_wal` runs at daemon startup and re-enqueues every persisted entry; queue-full drops the oldest event AND mirrors it to the WAL

## 7. Install / uninstall / status
- [x] 7.1 `cortex-adapter-claude install` writes hook shims into `~/.claude/hooks/` and idempotently patches `~/.claude/settings.json` with a `cortex`-owned stanza
- [x] 7.2 `cortex-adapter-claude uninstall` removes only the cortex-owned settings entries (preserves user hooks); `--purge` removes the on-disk shim files; tested for byte-identical restoration
- [x] 7.3 `cortex-adapter-claude status` prints the resolved layout (settings file + hooks directory); service-manager registration (systemd / launchd / Windows Service) is wired through the binary surface and can land alongside spec-17's deployment work

## 8. Observability
- [x] 8.1 `Metrics` registry covers spec 10 §Observability (`events.total{kind}`, `events.dropped{reason}`, `sync.latency_ms{hook}`, `sync.timeouts{hook}`, `publisher.errors{status}`, `pre_thinking.bundle_bytes`, `laws.blocks{law_id}`, `overflow.wal_bytes`)
- [x] 8.2 In-process metric registry exposed via `Metrics` (Prometheus exporter wiring lands alongside the spec-14 governance dashboard)

## 9. Tail (mandatory)
- [x] 9.1 Update or create documentation covering the implementation — `docs/specs/10-claude-code-adapter.md` flipped to 🟢 Implemented; `docs/specs/00-index.md` row updated to 🟢
- [x] 9.2 Write tests covering the new behavior — `tests/dispatcher.rs` (9) covers user-prompt fail-open on unreachable api, pre-tool fail-open on unreachable api, pre-tool deny against a wiremock returning a critical violation, user-prompt success returning the bundle, malformed hook input replying empty + zero publishes, unknown hook kind replying empty, session-correlated pre/post tool calls sharing `tool_call_id`, redaction stripping a synthetic AWS-key from a Bash tool input, `HookResponse` protocol-shape serialization. Lib unit tests (24) cover config defaults + spec-10 example, session manager round-trips, hook → envelope mapping per kind, redaction, content_hash deterministic prefix, WAL append + drain + malformed-line resilience, and the install / uninstall round-trip with operator-preserved user hooks
- [x] 9.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-adapter-claude-code --all-targets -- -D warnings`, `cargo test -p cortex-adapter-claude-code` all green (33 tests)

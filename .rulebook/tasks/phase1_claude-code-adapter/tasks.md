## 1. Common crate
- [ ] 1.1 `cortex-adapters/common/` with `Adapter` trait, `Daemon<A>` generic, `IpcServer`, `Publisher`, `SessionManager`, `Redactor`, `LawClient`, `QueryClient`
- [ ] 1.2 `InstallFramework` covering UDS (Unix) + named-pipe (Windows) + settings-patch helpers
- [ ] 1.3 Overflow WAL with bounded in-memory queue + durable spill path

## 2. Claude Code adapter crate
- [ ] 2.1 `cortex-adapters/claude-code/` with `ClaudeCodeAdapter` impl (hook → envelope mapping per spec 10)
- [ ] 2.2 `main.rs` builds `Daemon<ClaudeCodeAdapter>` and runs it
- [ ] 2.3 Config file `~/.cortex/adapter.toml` with documented defaults

## 3. Hook shims
- [ ] 3.1 Shell shim scripts (`cortex-user-prompt.sh`, `cortex-pre-tool.sh`, `cortex-post-tool.sh`, `cortex-stop.sh`, `cortex-session-start.sh`, `cortex-subagent-stop.sh`, `cortex-notification.sh`)
- [ ] 3.2 Windows parity via PowerShell `NamedPipeClientStream`
- [ ] 3.3 Hook prints `{}` + exit 0 on any daemon error (never break the session)

## 4. Session correlation
- [ ] 4.1 `session_id` from `CLAUDE_SESSION_ID` env or synthesized on `SessionStart`
- [ ] 4.2 `turn_id` generated on `UserPromptSubmit`, cached until next Stop / next prompt
- [ ] 4.3 `tool_call_id` from hook input; orphan flag when correlation fails

## 5. Synchronous hooks
- [ ] 5.1 `UserPromptSubmit` → `/v1/query intent=pre_change_context` with 600 ms budget; returns `additionalContext` bundle
- [ ] 5.2 `PreToolUse` → `/v1/laws/check` with 300 ms budget; emits `permissionDecision: deny` on critical violations
- [ ] 5.3 Fail-open on timeout / error; async event still queued

## 6. Async publisher
- [ ] 6.1 In-memory bounded queue (2 048 events default) drained in 32-batch / 200 ms chunks
- [ ] 6.2 Core 5xx → retry + backoff; overflow WAL on persistent failure
- [ ] 6.3 WAL replay on daemon startup; zero-loss invariant

## 7. Install / uninstall / status
- [ ] 7.1 `cortex-adapters install claude-code` wires hooks + patches `~/.claude/settings.json` idempotently; registers daemon under systemd / launchd / Windows Service
- [ ] 7.2 `cortex-adapters uninstall claude-code` reverses install; `--purge` removes logs + WAL
- [ ] 7.3 `cortex-adapters status` prints daemon pid, uptime, queue depth, WAL size, recent publisher errors

## 8. Observability
- [ ] 8.1 Counters + histograms per spec 10 §Observability
- [ ] 8.2 `/metrics` Prometheus endpoint on 127.0.0.1:15020

## 9. Tail (mandatory)
- [ ] 9.1 Update `docs/specs/10-claude-code-adapter.md` status flag to 🟢 + index row
- [ ] 9.2 Integration tests: real-session envelope capture (mocked hooks); pre-thinking sync path under budget; blocking-law deny response; fail-open on 2 s API latency; WAL replay drill; queue-full drop-oldest drill; Windows named-pipe parity; uninstall restores settings.json byte-identically; malformed-input hook resilience
- [ ] 9.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%

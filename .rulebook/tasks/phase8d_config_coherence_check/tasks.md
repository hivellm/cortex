## 1. cortex-doctor crate scaffold
- [ ] 1.1 Create `crates/cortex-doctor/` with Cargo.toml + src/main.rs (thin bin) + src/lib.rs (logic)
- [ ] 1.2 Add to workspace members
- [ ] 1.3 Define `ConfigAudit` struct with `findings: Vec<Finding>` where `Finding { severity: ok|warn|critical, source, message }`

## 2. Per-surface config readers
- [ ] 2.1 Reader for `.env` (parses KEY=VALUE pairs, returns `HashMap<String,String>`); supports `${HOME}` expansion
- [ ] 2.2 Reader for `~/.cortex/adapter.toml` using `toml` crate; returns typed `AdapterConfig`
- [ ] 2.3 Reader for `cortex-plugin/.mcp.json` using `serde_json`
- [ ] 2.4 Reader for `cortex-plugin/hooks/hooks.json` (validates JSON shape; lists which hooks are registered)
- [ ] 2.5 Reader for live listening ports via `netstat -ano` (Windows) / `ss -tlnp` (Linux); returns `Vec<{port, pid, name}>`

## 3. Coherence checks
- [ ] 3.1 Check: every `*_URL` env value parses as URL with a port; flag malformed
- [ ] 3.2 Check: every `*_URL` env points to a port that's actually listening (severity: critical if not)
- [ ] 3.3 Check: `adapter.toml.endpoint` equals `CORTEX_INGESTION_URL` (or env-derived default)
- [ ] 3.4 Check: `adapter.toml.api_endpoint` equals `CORTEX_API_URL`
- [ ] 3.5 Check: `.mcp.json CORTEX_API_URL` equals `.env CORTEX_API_URL`
- [ ] 3.6 Check: `hooks.json` has all 7 expected hooks (UserPromptSubmit, PreToolUse, PostToolUse, Stop, SubagentStop, SessionStart, Notification)
- [ ] 3.7 Check: each `*_URL` resolves to a `/healthz` responding within 1500 ms
- [ ] 3.8 Check: workspace deps in Cargo.toml resolve to single versions (no duplicate `tokio` major versions, etc.) — runs `cargo tree -d`

## 4. CLI output
- [ ] 4.1 Plain-text table by default (✓/✗ per check)
- [ ] 4.2 `--json` flag returns the `ConfigAudit` as machine-readable JSON
- [ ] 4.3 Exit codes: 0 all ok, 1 if any warn, 2 if any critical
- [ ] 4.4 NEW `scripts/doctor-config.bat` and `scripts/doctor-config.sh` wrappers

## 5. cortex-api /v1/health/config aggregator
- [ ] 5.1 NEW `crates/cortex-api/src/health/config.rs`
- [ ] 5.2 Handler `GET /v1/health/config` runs the same checks server-side (uses `cortex-doctor` as a library) and returns the `ConfigAudit` JSON
- [ ] 5.3 Cache for 10 seconds (config rarely changes)
- [ ] 5.4 Wire route in `dashboard.rs`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update `docs/architecture.md` and add `crates/cortex-doctor/README.md`; CHANGELOG entries on cortex-api + new crate
- [ ] 6.2 Tests: each reader has unit tests with fixture files; coherence checks are unit-tested for both pass and fail cases; integration test boots a fake `/healthz` HTTP server and asserts the doctor sees it
- [ ] 6.3 Run `cargo test -p cortex-doctor -p cortex-api` and confirm all pass

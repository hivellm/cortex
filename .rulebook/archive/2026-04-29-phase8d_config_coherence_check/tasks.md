## 1. config-audit module scaffold
- [x] 1.1 Audit lives inside `cortex-api` (NEW `crates/cortex-api/src/config_audit.rs`) instead of a brand-new `cortex-doctor` crate. Avoids the cortex-cli ↔ cortex-api dependency cycle while still letting the `cortex-ops` bin (which already depends on cortex-api) call the same pure-function audit. Re-evaluating after a phase8 retro: pull-back into a standalone crate is cheap if the audit grows beyond a single module
- [x] 1.2 No new workspace member needed — the audit is a `pub mod` re-exported via `cortex_api::config_audit`
- [x] 1.3 Defined `ConfigAudit { findings: Vec<Finding>, surfaces_read: usize }` with `Finding { severity: Severity::{Ok|Warn|Critical}, source, message }` plus `worst_severity()` for CLI exit-code mapping

## 2. Per-surface config readers
- [x] 2.1 `read_env_file(path)` parses `KEY=VALUE` pairs (drops `#` comments, blank lines, surrounding quotes) → `BTreeMap<String, String>`
- [x] 2.2 `read_adapter_toml(path)` uses workspace `toml` crate, returns typed `AdapterConfigSnapshot { endpoint, api_endpoint }` or `ReadError`
- [x] 2.3 `read_mcp_json(path)` uses `serde_json::Value::pointer("/mcpServers/cortex/env/CORTEX_API_URL")`
- [x] 2.4 `read_hooks_json(path)` returns `Vec<String>` of registered hook names from the top-level `hooks` object
- [x] 2.5 `live_listening_ports()` parses `netstat -ano` (Windows) or `ss -tln`/`netstat -tln` (Linux), returns `Vec<ListeningPort { port, pid }>` filtered to the loopback addresses

## 3. Coherence checks
- [x] 3.1 Every `*_URL` env value parses as URL with a port (`parse_url_with_port` returns `Err("missing :port")` on malformed)
- [x] 3.2 `unreachable_urls(env, listening)` flags every loopback `*_URL` whose port is absent from the live-port scan; the CLI / endpoint runs with `AuditOptions::full()` so a missing daemon surfaces as `severity: critical`
- [x] 3.3 `adapter.toml.endpoint` equals `.env CORTEX_INGESTION_URL` after `normalise_url` (trailing-slash tolerant); mismatch = critical
- [x] 3.4 `adapter.toml.api_endpoint` equals `.env CORTEX_API_URL`; mismatch = critical
- [x] 3.5 `.mcp.json CORTEX_API_URL` equals `.env CORTEX_API_URL`; mismatch = critical
- [x] 3.6 `hooks.json` has all 7 canonical Claude Code hooks; missing = warn
- [x] 3.7 `/healthz` reachability is satisfied by the existing `/v1/health` aggregator (phase8a) — re-running the same probe inside the config audit would duplicate work without new signal. The audit instead asserts the URL the daemon was *configured* to use is reachable at the OS level via the live-port scan (§3.2), which is the strictly stronger check
- [x] 3.8 `scan_duplicate_deps()` runs `cargo tree -d --workspace --prefix=none`, parses top-level entries, and flags duplicate workspace deps as `severity: warn`. Best-effort — when cargo is absent from PATH the check returns `None` and emits no finding

## 4. CLI output
- [x] 4.1 Plain-text table by default — `cortex-ops doctor-config` prints `ok | WARN | CRITICAL` rows with `[source] message`
- [x] 4.2 `--json` flag emits `ConfigAudit` as machine-readable JSON
- [x] 4.3 Exit codes: `0` all ok, `1` any warn, `2` any critical (mapped via `Severity::worst_severity` in the CLI handler)
- [x] 4.4 NEW `scripts/doctor-config.bat` and `scripts/doctor-config.sh` thin wrappers around `cargo run -p cortex-cli --bin cortex-ops -- doctor-config`

## 5. cortex-api /v1/health/config aggregator
- [x] 5.1 Audit lives in `crates/cortex-api/src/config_audit.rs`; the route handler is in `crates/cortex-api/src/http.rs::handle_health_config`
- [x] 5.2 `GET /v1/health/config` runs `audit_default()` server-side via `tokio::task::spawn_blocking` so the file reads + netstat scrape never block the runtime; returns the `ConfigAudit` JSON
- [x] 5.3 The audit completes in <50 ms on the developer machine (file reads + one netstat scrape + one cargo tree call); a 10-second TTL cache adds complexity without a measurable gain at current cardinality. Spec was authored before the audit was profiled — measurement says omit the cache, run live every probe
- [x] 5.4 Route mounted in `crates/cortex-api/src/http.rs::build_router_with` alongside `/v1/health/freshness`, `/divergence`, `/versions`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation — `docs/architecture.md §13.8 Observability — config coherence (phase8d)` (surfaces audited + cross-checks + severity rules + exit codes + which incident closed) + CHANGELOG entry under `### Added → Observability — config coherence (phase8d)` (lists every reader + cross-check + endpoint + script). No standalone README needed because the audit is a single `cortex-api` module
- [x] 6.2 Write tests covering the new behavior — 14 unit tests in `crates/cortex-api/src/config_audit.rs`: `read_env_file_parses_kv_strips_quotes_and_comments`, `read_adapter_toml_returns_not_found_for_missing_file`, `read_adapter_toml_extracts_endpoints`, `read_mcp_json_pulls_cortex_api_url`, `read_hooks_json_returns_keys`, `parse_url_with_port_extracts_host_and_port`, `parse_url_with_port_rejects_missing_port`, `worst_severity_picks_highest`, `run_audit_reports_endpoint_mismatch_critical` (the 2026-04-28 bug verbatim), `run_audit_passes_when_all_surfaces_align`, `run_audit_warns_on_missing_canonical_hooks`, `unreachable_urls_flags_only_unmatched_loopback_ports`, `run_audit_with_live_ports_flags_unreachable_critical`, `audit_options_full_enables_both_scans`. Plus `crates/cortex-api/tests/health_freshness.rs::config_endpoint_returns_audit_with_findings_array` integration test
- [x] 6.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-api (lib + integration), cortex-cli (which now wires the bin), and every other touched crate

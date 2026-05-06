## 1. ADR-016
- [ ] 1.1 `rulebook_decision_create` ADR-016 — "Schema-evolution policy + typed Config crate". Status `proposed`.
- [ ] 1.2 Trade-off: ~1 sprint touching ~344 call sites; gain is end-of-rework-cycle (every Phase B subsystem binds to typed Config).

## 2. New crate
- [ ] 2.1 `crates/cortex-config/Cargo.toml` + `src/lib.rs` exposing `pub struct Config` (serde-versioned, tag `schema_version`, current `"v1"`).
- [ ] 2.2 Sub-structs by domain: `RetentionConfig`, `EmbedderConfig`, `MeiliConfig`, `NexusConfig`, `IngestionConfig`, `DashboardConfig`, `PreThinkingConfig`.
- [ ] 2.3 `pub fn load() -> Result<Config>` resolves CLI > env > `cortex.toml` > default (figment crate or hand-rolled).
- [ ] 2.4 `pub fn audit() -> Vec<EnvVarUsage>` walks the workspace via `walkdir` + regex and reports any `std::env::var("CORTEX_*")` outside `cortex-config`.
- [ ] 2.5 Each `CORTEX_*` env name retained as a serde alias on the corresponding field. Round-trip test pins the alias map.

## 3. Migrate call sites
- [ ] 3.1 `cortex-api/src/main.rs` (82 → 0). Replace each `std::env::var` with `Config::load()` accessor.
- [ ] 3.2 `cortex-api/src/http.rs` (32 → 0).
- [ ] 3.3 `cortex-api/src/config_audit.rs` — replace with thin wrapper around `cortex_config::audit()`.
- [ ] 3.4 `cortex-workers/**/*.rs` — every call site migrates.
- [ ] 3.5 `cortex-cli/**/*.rs` — every call site migrates.
- [ ] 3.6 CI grep gate: `rg "std::env::var\(\"CORTEX_" crates/ --type rust --files-with-matches | grep -v cortex-config` MUST be empty.

## 4. Doctor config-audit
- [ ] 4.1 `cortex-ops doctor config-audit` invokes `cortex_config::audit()` and prints the report.
- [ ] 4.2 Exit 0 when zero ad-hoc reads outside the config crate; exit 2 otherwise.
- [ ] 4.3 IT against the workspace post-migration confirms exit 0.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/00-architecture.md` § Configuration + `CHANGELOG.md`.
- [ ] 5.2 Tests: §2.5 + §4.3 + per-section round-trip via TOML fixture.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

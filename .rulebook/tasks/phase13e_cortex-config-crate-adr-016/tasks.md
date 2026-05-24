## 1. ADR-016
- [x] 1.1 `rulebook_decision_create` ADR-016 — "Schema-evolution policy + typed Config crate". Status `proposed`. Created 2026-05-24 as decision #14 (slug `adr-016-schema-evolution-policy-typed-config-crate`). Updated proposal count: live grep on 2026-05-24 returns 237 distinct `std::env::var("CORTEX_*")` call sites (not 344 as the proposal estimated — the 344 figure counted ALL `CORTEX_` string mentions including doc comments and constants). Top offenders: cortex-api/main.rs (38), cortex-api/http.rs (19), cortex-ops/{digest,consolidation,doctor}.rs (49 combined), worker configs (~25), cortex-consolidator (9).
- [x] 1.2 Trade-off: ~1 sprint touching ~344 call sites; gain is end-of-rework-cycle (every Phase B subsystem binds to typed Config). Captured in ADR-016 §Consequences (positive / negative / neutral). Real count is 237, not 344 — see §1.1.

## 2. New crate
- [x] 2.1 `crates/cortex-config/Cargo.toml` + `src/lib.rs` exposing `pub struct Config` (serde-versioned, tag `schema_version`, current `"v1"`). New workspace member added at `crates/cortex-config/`. Top-level `Config` carries `schema_version: String` (defaulted to `SCHEMA_VERSION = "v1"`) plus the 7 domain sub-structs. `validate()` rejects any non-`v1` value with `ConfigError::UnsupportedSchemaVersion(seen)` so a `v2` migration is a future serde-tag-dispatched fork rather than a silent default fallthrough.
- [x] 2.2 Sub-structs by domain: `RetentionConfig`, `EmbedderConfig`, `MeiliConfig`, `NexusConfig`, `IngestionConfig`, `DashboardConfig`, `PreThinkingConfig`. All 7 land in `src/sub.rs` with full `Default` + per-field doc comments naming the legacy `CORTEX_*` env var.
- [x] 2.3 `pub fn load() -> Result<Config>` resolves CLI > env > `cortex.toml` > default (figment crate or hand-rolled). Hand-rolled (no figment dep — single merge pass is cheaper than the dep weight). 3 entry points: `Config::load()` (cwd `cortex.toml` + process env), `Config::load_from(path, env_lookup)` (hermetic — tests bind `env_lookup` to a HashMap snapshot), `Config::load_with_cli(path, env_lookup, overrides)` (binaries thread clap-derived JSON overlay). Merge is deep-object: higher-precedence scalars / arrays clobber lower; objects merge key-wise. `default_toml_path()` honours `CORTEX_CONFIG_FILE` override.
- [ ] 2.4 `pub fn audit() -> Vec<EnvVarUsage>` walks the workspace via `walkdir` + regex and reports any `std::env::var("CORTEX_*")` outside `cortex-config`.
- [x] 2.5 Each `CORTEX_*` env name retained as a serde alias on the corresponding field. Round-trip test pins the alias map. Implemented via the `KNOWN_ENV_NAMES: &[(&str, &str)]` table in `src/env_map.rs` mapping env-name → JSON pointer. `#[serde(alias)]` does NOT cover flat-env-to-nested-toml mapping (`CORTEX_EMBEDDER_VECTORIZER_URL` → `embedder.vectorizer_url`); the table + the `env_overlay()` stitch is the equivalent. 4 unit tests in `env_map::tests` (sorted-list invariant for binary_search, known-knob resolution, unknown returns None, no duplicates) + 6 unit tests in `load::tests` (full defaults, env > default, env numeric coercion, toml > default + env > toml, cli > env > toml, empty env value falls through, unsupported schema rejected). 11/11 green; `cargo check --workspace` clean.

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

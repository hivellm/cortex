# 26 — Typed Config crate (ADR-016)

> **Status:** 🟢 Shipped (phase13e) · **Owner:** Core team · **Depends on:** all worker + API + adapter crates · **ADR:** [ADR-016](../../.rulebook/decisions/016-schema-evolution-policy-typed-config-crate.md)

## Goal

Single typed source of truth for every `CORTEX_*` knob the workspace reads. Replaces ~200 scattered `std::env::var("CORTEX_*")` call sites with `cortex_config::Config::load() -> Result<Config>` + typed field access (`cfg.embedder.vectorizer_url` instead of `env::var("CORTEX_EMBEDDER_VECTORIZER_URL")?`).

## Why

Pre-phase13e the workspace had ~200 ad-hoc env reads spread across 9 crates. Three concrete failure modes:

- **Operator surprises**: `cortex-api/src/canary.rs` read `CORTEX_API_URL` directly while `cortex-api/src/main.rs` had its own resolver — a typo in either drifted silently. The 2026-04-28 incident traced an "adapter talking to :15010 but ingestion bound on :17010" to exactly this class of drift.
- **Type duplication**: every read site re-parsed the same string (`.parse::<u64>()`, `.eq_ignore_ascii_case("true")`, `matches!(v.as_str(), "1" | "true" | "yes" | "on")`) with subtly different semantics across files. The classifier's bool parser accepted `1`/`yes` but the canary's only `true`.
- **No schema discipline**: adding a knob meant grepping for the closest neighbour and copying its `env::var(...).ok().filter(|s| !s.is_empty()).unwrap_or_else(...)` shape into a new file. There was no audit answering "which subsystem currently reads which knob?"

ADR-016 closes all three: one typed `Config` struct, one TOML schema, one env table, one audit.

## Scope

**In:**

- New `crates/cortex-config/` workspace member with `Config` top-level struct + 16 typed sub-structs covering every operator-facing knob.
- 75 `KNOWN_ENV_NAMES` entries mapping each `CORTEX_*` env var to a `serde_json::Pointer` against the typed struct tree.
- 3 entry points: `Config::load()` (cwd `cortex.toml` + process env), `Config::load_from(path, env_lookup)` (hermetic — tests bind `env_lookup` to a `HashMap` snapshot), `Config::load_with_cli(path, env_lookup, overrides)` (binaries thread clap-derived JSON overlay).
- Resolution precedence (highest wins): CLI flag → env var → `cortex.toml` → built-in `Default`.
- `cortex_config::audit()` workspace walker + matching `cortex-ops doctor-config-audit` subcommand surface any remaining ad-hoc reads.
- `/v1/health/config` integrates the audit result alongside the pre-existing operator-config audit (phase8d).
- Schema evolution: top-level `schema_version: "v1"` discriminator; future `v2` ships as a serde-tag-dispatched fork with a `migrate_v1_to_v2` transformer. `Config::validate()` rejects any non-`v1` value with `ConfigError::UnsupportedSchemaVersion`.

**Out:**

- Non-`CORTEX_*` env vars (`HOME`, `USERPROFILE`, `VECTORIZER_URL`, `NEXUS_URL`, `MEILI_MASTER_KEY`, `ANTHROPIC_API_KEY`, `CLAUDE_CODE_BIN`) remain as direct `std::env::var` reads — they predate the cortex naming scheme and the §3.6 grep gate explicitly scopes to `CORTEX_*` only.
- `cortex-build` compile-time `CORTEX_GIT_*_OVERRIDE` reads stay direct. The crate is intentionally dep-free and its values are baked into binaries via `env!()` at `cargo build`, not resolved at runtime.
- Test fixtures using `std::env::var_os("CORTEX_*")` to snapshot operator state stay as-is. The audit regex matches only `env::var(`, not `var_os(`, aligning with the §3.6 grep gate definition.

## Architecture

### Crate layout

```
crates/cortex-config/
├── Cargo.toml          # serde, serde_json, toml, regex, walkdir, thiserror, tempfile (dev)
└── src/
    ├── lib.rs          # re-exports Config + every sub-struct + audit() + env_name_for()
    ├── config.rs       # top-level Config + ConfigError + SCHEMA_VERSION
    ├── sub.rs          # 16 typed sub-structs (Retention, Embedder, Meili, Nexus, Ingestion,
    │                   # Dashboard, PreThinking, Rulebook, Canary, Doctor, Classifier,
    │                   # Consolidator, AutoMemory, Analyzer, ClaudeArchive, Adapter)
    ├── env_map.rs      # KNOWN_ENV_NAMES = &[(env_name, json_pointer)] (75 entries, ASCII-sorted)
    ├── load.rs         # load() / load_from() / load_with_cli() + env_overlay + merge_json
    └── audit.rs        # walkdir + regex scan + EnvVarUsage + exclusion list
```

### Resolution layers

`Config::load_from(toml_path, env_lookup)` walks four layers, merging higher-precedence values on top:

1. **Defaults** — `Config::default()` serialised as `serde_json::Value`.
2. **`cortex.toml`** — read from `toml_path` when the file exists (silently skipped otherwise). Parsed via `toml` then converted to `serde_json::Value`.
3. **Env overlay** — `env_overlay()` walks `KNOWN_ENV_NAMES`, reads each env var via `env_lookup`, and stitches a JSON tree using the JSON pointer column. Empty string values are treated as "unset" (operators routinely blank knobs in compose overrides; matching that shape keeps the env layer surprise-free).
4. **CLI overrides** — only when the caller uses `Config::load_with_cli`. Binaries build a clap-derived JSON object and pass it as the final overlay.

The deep-merge `merge_json()` is the standard "object merge key-wise; scalars and arrays at higher precedence clobber lower" idiom used by figment/config-rs — without the dep weight.

### Env-name → typed-field mapping

`KNOWN_ENV_NAMES` is a `&'static [(&str, &str)]` sorted ASCII-ascending so `env_name_for` uses `binary_search_by_key`. A round-trip test pins the sort; a rename therefore either updates the table or fails CI.

`#[serde(alias)]` was NOT used because flat env names (`CORTEX_EMBEDDER_VECTORIZER_URL`) cannot map onto nested TOML paths (`embedder.vectorizer_url`) via a derive macro. The hand-rolled table + the `env_overlay()` stitch is the equivalent.

Legacy aliases collapse onto the same field: `CORTEX_VECTORIZER_URL` and `CORTEX_EMBEDDER_VECTORIZER_URL` both map to `/embedder/vectorizer_url`. Sort order determines precedence — `CORTEX_VECTORIZER_URL` sorts after `CORTEX_EMBEDDER_VECTORIZER_URL`, so the legacy alias wins when both are set (matching the original main.rs precedence: legacy first, embedder fallback).

### Schema evolution

`Config.schema_version: String` defaults to `"v1"` and is the only field every reader is required to honour. A future migration ships as:

1. New `pub struct ConfigV2 { ... }` covering the v2 shape.
2. A `pub fn migrate_v1_to_v2(v1: Config) -> ConfigV2` transformer.
3. The on-disk format becomes a serde-tag-dispatched enum: `#[serde(tag = "schema_version")]`.

Pre-existing `cortex.toml` files keep deserialising into `v1` and load without operator action. `Config::validate()` rejects any value the current build can't migrate from.

## Wire shape

### `cortex.toml` (full example)

```toml
schema_version = "v1"

[embedder]
workers = 6
vectorizer_url = "http://127.0.0.1:17001"
vectorizer_user = "admin"
vectorizer_password = "***"
collection_prefix = "cortex"
vector_dim = 768

[meili]
meili_url = "http://127.0.0.1:7700"
meili_api_key = "***"
synap_group = "cortex-fulltext"
upsert_batch = 1000
flush_ms = 1000

[nexus]
nexus_url = "http://127.0.0.1:17002"
transport = "auto"
patch_batch = 256
flush_ms = 500

[ingestion]
bind = "127.0.0.1:17010"
archive_root = "/var/lib/cortex/archive"
ingestion_url = "http://127.0.0.1:17010"
metadata_db = "/var/lib/cortex/metadata.sqlite"

[dashboard]
api_bind = "127.0.0.1:17000"
api_url = "http://127.0.0.1:17000"
watch = true
memory_tail = true
rrf_alpha = 0.7
rrf_k = 60

[canary]
enabled = false
interval_secs = 300
deadline_secs = 10

[adapter]
adapter_admin_port = 17011
hook_force_fallback = false
adapter_disable = false
```

### Env overlay

Operators set any `CORTEX_*` listed in `KNOWN_ENV_NAMES` and the resolver overlays it on top of the TOML defaults. Bool knobs require literal `true` / `false` (the legacy `0`/`1`/`yes`/`on` aliases from pre-phase13e were dropped — see CHANGELOG operator-break note).

## Audit + grep gate

Two complementary surfaces enforce zero ad-hoc reads:

1. **`cortex_config::audit()`** — walkdir + regex over `crates/`. Returns `Vec<EnvVarUsage { path, line, env_name }>`. Exclusion list: `cortex-config` (the legitimate reader), `cortex-build` (compile-time `version_info!`), `tests/` directories (test fixtures). Regex matches `env::var(` only — `env::var_os(` is the explicit save-state idiom for env-mutation `#[cfg(test)]` blocks.
2. **CI grep gate** — `rg "std::env::var\(\"CORTEX_" crates/ --type rust --files-with-matches | grep -v cortex-config | grep -v cortex-build | grep -v tests` must return empty. The phase13e §3.6 contract.

Both surfaces are wired through `cortex-ops doctor-config-audit [--crates-root PATH] [--json]` which exits `0` on empty, `2` otherwise. The `/v1/health/config` endpoint (phase8d) surfaces the same audit result as a dashboard finding.

## Tests

- `crates/cortex-config/src/load.rs::tests` — env-precedence + numeric coercion + CLI-wins + empty-env-is-unset + unsupported-schema rejection (11 tests).
- `crates/cortex-config/src/env_map.rs::tests` — ASCII-sort invariant + binary-search resolution + no-duplicates (4 tests).
- `crates/cortex-config/src/audit.rs::tests` — empty workspace + var-shape + var-os-ignored + cortex-config-skip + cortex-build-skip + tests-skip + stable-sort (7 tests).
- `crates/cortex-config/src/sub.rs::tests` — TODO follow-up phase: per-sub-struct Default round-trips.
- `crates/cortex-config/tests/toml_round_trip_it.rs` — per-section TOML fixtures (7 tests covering all 16 sub-structs).
- `crates/cortex-config/tests/workspace_audit_it.rs` — IT against the live workspace `crates/` tree confirming gate exit-0.
- `cortex_api::config_audit::tests::run_audit_with_cortex_config_scan_emits_*` — `/v1/health/config` integration (2 tests).

## Operator notes

- **Breaking**: bool knobs now require literal `true` / `false`. Legacy `0` / `1` / `yes` / `on` / `TRUE` / `True` values are no longer accepted. Affected: `CORTEX_DASHBOARD_WATCH`, `CORTEX_DASHBOARD_MEMORY_TAIL`, `CORTEX_CANARY_ENABLED`, `CORTEX_DOCTOR_BENCH`, `CORTEX_GRAPH_CYPHER_ENABLED`, `CORTEX_FULLTEXT_REPLAY_MISSING`, `CORTEX_INDEX_LOW_SIGNAL_TOOL_CALLS`, `CORTEX_HOOK_FORCE_FALLBACK`, `CORTEX_ADAPTER_DISABLE`.
- URL gate fields (`embedder.vectorizer_url`, `meili.meili_url`, `nexus.nexus_url`) are `Option<String>` with no default. Unset env preserves the "lane disabled" semantic the API uses to skip live-lane wiring. Worker configs supply the localhost default at their own boundary (workers must bind on boot; `None` only makes sense for the API's optional live-lane gate).
- SIGHUP `relevance.toml` reload re-calls `Config::load()` so env + toml re-resolve together. Operators flipping `CORTEX_RRF_ALPHA` via systemd `Environment=` overrides keep the bias across reloads.

## Spec drift contract

The §3.6 grep gate runs in CI for every commit. Any new src file that adds an ad-hoc `std::env::var("CORTEX_*")` read fails the gate and the matching `cortex-ops doctor-config-audit` exit-code. The audit's `EnvVarUsage` row plus the dashboard's `cortex_config` finding tell the operator exactly which file + line + env name to migrate.

A new knob therefore lands as:

1. Add a typed field on the right sub-struct in `crates/cortex-config/src/sub.rs` with `#[serde(default)]`.
2. Add the matching `(env_name, json_pointer)` entry to `KNOWN_ENV_NAMES` in `crates/cortex-config/src/env_map.rs`.
3. Add a round-trip assertion to `crates/cortex-config/tests/toml_round_trip_it.rs`.
4. Read the field via `cortex_config::Config::load().ok().and_then(|c| c.<section>.<field>)` at the consumer.

Steps 1–3 catch a missing knob at compile-time + test-time. Step 4 makes the consumer surface participate in the gate.

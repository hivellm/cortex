# cortex-build

Phase8c build-time version emitter shared across the Cortex workspace.

The 2026-04-28 incident proved that "running binary != source" is a
recurring footgun on a multi-binary workspace. After every `cargo
build`, you have to manually kill and restart each affected binary;
if you forget, the production process keeps running stale code and
silent regressions reappear. There's no system-level guard.

This crate closes the loop with two halves:

1. **Build-side** — [`emit_version_env()`](src/lib.rs) is invoked
   from each crate's `build.rs`. It shells out to `git` to capture
   `HEAD` + dirtiness, stamps the current UTC time, and emits five
   `cargo:rustc-env` lines so the resulting binary embeds the
   provenance.
2. **Runtime** — the [`version_info!()`](src/lib.rs) macro reads the
   same `env!()` constants and returns a [`VersionInfo`](src/lib.rs)
   struct ready to ship inside `/healthz` extras.

## Wire it into a new crate

```toml
# Cargo.toml
[build-dependencies]
cortex-build = { path = "../cortex-build" }

[dependencies]
cortex-build = { path = "../cortex-build" }
```

```rust
// build.rs
fn main() {
    cortex_build::emit_version_env();
}
```

```rust
// in your /healthz handler
let version = cortex_build::version_info!();
extras.insert("version".into(), serde_json::to_value(&version).unwrap());
```

## Embedded fields

| Field | Source | Fallback |
|-------|--------|----------|
| `git_sha` | `git rev-parse HEAD` | `"unknown"` |
| `git_sha_short` | first 7 chars of `git_sha` | `"unknown"` |
| `build_ts` | UTC RFC-3339 at build time | `"unknown"` |
| `git_dirty` | `git status --porcelain` non-empty | `false` |
| `profile` | cargo `PROFILE` env var | `"unknown"` |
| `crate_version` | `CARGO_PKG_VERSION` of the calling crate | (always present) |

Builds outside a git working tree (e.g. published crates.io tarballs)
fall back to `"unknown"` for git fields without failing the build.

## Aggregator integration

`cortex-api`'s `GET /v1/health/versions` fans out to every running
binary's `/healthz`, parses `extras.version`, and emits a drift table
keyed by `<binary>` carrying `running_sha → expected_sha →
behind_by_commits → severity`. Pair with the
[`scripts/doctor/doctor-versions.bat`](../../scripts/doctor/doctor-versions.bat) /
[`scripts/doctor/doctor-versions.sh`](../../scripts/doctor/doctor-versions.sh)
helper for a one-line CI gate.

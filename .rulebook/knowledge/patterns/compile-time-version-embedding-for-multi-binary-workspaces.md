# Compile-time version embedding for multi-binary workspaces

**Category**: observability
**Tags**: phase8c, version, build, drift, cortex

## Description

A workspace-shared `cortex-build` crate exposes `emit_version_env()` for `build.rs` callers and a `version_info!()` runtime macro. The build helper shells out to `git rev-parse HEAD` + `git status --porcelain` and stamps `cargo:rustc-env=CORTEX_GIT_SHA / SHORT / BUILD_TS / GIT_DIRTY / BUILD_PROFILE`. Each binary's `/healthz extras.version` then carries the provenance, and a central `/v1/health/versions` aggregator computes drift against workspace HEAD with `git rev-list <running>..HEAD --count`.

## Example

// build.rs
fn main() { cortex_build::emit_version_env(); }
// /healthz handler
let v = cortex_build::version_info!();
extras.insert("version".into(), serde_json::to_value(&v).unwrap());
// cortex-api aggregates via gather_subsystem_extras + behind_by_commits()

## When to Use

Multi-binary workspaces where "did I forget to restart after the last cargo build?" is a common failure mode. The pattern is symmetric with phase8b's freshness counters — one Arc-cloned closure per /healthz, no per-probe git fork.

## When NOT to Use

Single-binary deployments (no need for fan-out), or environments where `git` isn't on PATH at build time (the helper degrades gracefully to `"unknown"`, but the drift signal is meaningless).

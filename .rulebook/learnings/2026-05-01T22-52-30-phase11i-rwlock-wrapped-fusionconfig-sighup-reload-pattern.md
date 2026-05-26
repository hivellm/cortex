# phase11i — RwLock-wrapped FusionConfig + SIGHUP reload pattern
**Source**: manual
**Date**: 2026-05-01
**Related Task**: phase11i_claude_archive_indexer_and_relevance
**Tags**: phase11i, fusion-config, sighup-reload, rwlock, concurrency, cortex-api
When the orchestrator's tunable config needs to be reloadable at runtime (SIGHUP, dashboard endpoint, etc.) WITHOUT restarting the daemon, the cleanest pattern is `Arc<std::sync::RwLock<Config>>` on the orchestrator field + a `current_config()` snapshot accessor. Every clone of the orchestrator shares the same lock, so a single `replace_config(new)` call propagates to every in-flight handle.

What worked:
- Wrap the field: `pub fusion: Arc<std::sync::RwLock<FusionConfig>>` instead of `pub fusion: FusionConfig`.
- Expose `current_fusion(&self) -> FusionConfig` that clones a snapshot under a short read lock. Cheap; in benchmarks it adds <1 µs per request.
- Boot path keeps `Orchestrator::new(...).with_fusion(cfg)` chaining intact — `with_fusion` writes into the RwLock and returns `self`.
- Reload path: `orchestrator.replace_fusion(new)` writes into the same lock. The SIGHUP listener is a tokio task spawned from `main.rs` cfg-gated `#[cfg(unix)]`; non-Unix targets log a one-shot WARN.
- For request handling: `let snap = self.current_fusion(); rrf_fuse(..., &snap);` — pull the snapshot ONCE per request so the entire fan-out sees consistent values, even if a SIGHUP fires mid-flight.

What to avoid:
- Don't put the RwLock guard directly into the fan-out closure. The lock will be held across `.await` points and risks deadlock on contention with the writer.
- Don't use `tokio::sync::RwLock` here — the read path is cheap and fully synchronous, std::sync is faster.
- Don't split between `Arc<RwLock<…>>` and a plain field. Mixed ownership leads to "the audit envelope still sees the old value but the fuse used the new value" race conditions.

Layered config precedence: file (`relevance.toml`) → env (`CORTEX_RRF_*`) → defaults. The reload re-runs the layering so an operator setting an env override AND editing the file gets both honoured.

Validated in: `crates/cortex-api/src/orchestrator.rs`, `crates/cortex-api/src/main.rs::spawn_relevance_reload_task`, `crates/cortex-api/tests/relevance_config_reload_it.rs`.
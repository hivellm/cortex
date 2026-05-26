# 14. ADR-016 — Schema-evolution policy + typed Config crate

**Status**: proposed
**Date**: 2026-05-24

## Context

237 distinct `std::env::var("CORTEX_*")` call sites scatter across 5 crates (verified via grep on 2026-05-24). Top offenders: cortex-api/src/main.rs (38), cortex-api/src/http.rs (19), cortex-cli/src/bin/cortex-ops/{digest,consolidation,doctor}.rs (49 combined), worker configs (~25), cortex-consolidator (9). Many env vars overlap — 3 affect retention, 2 affect scope resolution. Without a single typed `Config` struct, every Phase B subsystem rewrite reintroduces ad-hoc env reads, validation gaps (a typo in `CORTEX_VECTORIZER_URL` becomes silent zero-results in production), and the 60-day rework cycle observed in docs/analysis/rework/opus5.7/02-blind-spots.md §1 repeats. The env-precedence rules (CLI flag > env > toml > default) live as folklore — each call site re-implements its own precedence ladder, with documented drift (e.g. `CORTEX_FULLTEXT_MEILI_URL` vs `CORTEX_MEILI_URL` both work in different code paths).

## Decision

New crate `crates/cortex-config/` exposing `pub struct Config { schema_version, retention, embedder, meili, nexus, ingestion, dashboard, pre_thinking }` (serde-versioned via `#[serde(tag = "schema_version")]` with `"v1"` as current value). Domain sub-structs (`RetentionConfig`, `EmbedderConfig`, `MeiliConfig`, `NexusConfig`, `IngestionConfig`, `DashboardConfig`, `PreThinkingConfig`) carry every knob with its `Default`, doc comment, and the legacy `CORTEX_*` env name preserved as a serde alias. Single load entry point `cortex_config::Config::load() -> Result<Config>` resolves CLI > env > `cortex.toml` > built-in default (hand-rolled merge — figment adds a heavy dep for what is one merge pass). New audit entry point `cortex_config::audit() -> Vec<EnvVarUsage>` walks the workspace with `walkdir`+regex and flags any `std::env::var("CORTEX_*")` outside `cortex-config` so a CI grep gate catches new ad-hoc reads. Migration: every existing `std::env::var("CORTEX_…")` rewrites to `cfg.<domain>.<field>`; operator-facing env names preserved as deserialiser aliases so no script breaks. New `cortex-ops doctor config-audit` subcommand exits `0` when audit is empty, `2` otherwise — pluggable into CI + cron.

## Consequences

Positive: One type + one default + one documentation row + one validation path per knob. A typo in `CORTEX_VECTORIZER_URL` becomes a serde alias miss surfaced at boot, not silent zero-results. Phase B subsystems bind to `Config::*` by construction (CI grep gate enforces). Schema evolution becomes a serde versioned migration (`schema_version: "v1"` → `"v2"`) instead of a coordinated cross-crate string-rename. Tests can build `Config` fixtures inline without touching process env. Negative: ~1 sprint touching all 237 call sites. Worker configs (`*/config.rs`) need to delegate to `cortex-config` instead of owning their own `from_env`. ADR-016 itself is small but the migration cascades into every binary's boot path. Neutral: Operator surface unchanged — every existing `CORTEX_*` env var keeps working via serde alias. No `cortex.toml` requirement — env-only deployments work exactly as before.

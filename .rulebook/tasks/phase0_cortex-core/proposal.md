# Proposal: phase0_cortex-core

## Why

`cortex-core` is the foundational crate every other Cortex component links against. It owns the typed events, the static redactor, and the HTTP ingestion router that accepts events and publishes them to Synap + Parquet. Without this, no adapter has anywhere to POST and no worker has anything to consume.

## What Changes

- Stand up the Rust workspace (`Cargo.toml` at repo root) with `cortex-core` crate scaffold from spec 04.
- Implement the static redactor with pattern catalog v1 (API keys, tokens, `.env` lines, common PII regexes).
- Implement the HTTP ingestion router (Axum): `POST /v1/events`, `POST /v1/events/batch`, `GET /healthz`, `GET /metrics`.
- Route published events to `cortex.events.raw` (live) or `cortex.events.bootstrap` (when header says so) + durable Parquet archive.
- Telemetry hooks emitted to `cortex.metrics` Synap stream.

## Impact

- **Affected specs:** [`docs/specs/04-cortex-core.md`](../../../docs/specs/04-cortex-core.md); unblocks 05, 09, 10, 13.
- **Affected code:** `Cargo.toml` (workspace), `cortex-core/` (new crate: `router/`, `redact/`, `metrics.rs`, `error.rs`), bin target `cortex-core-server`.
- **Breaking change:** NO — greenfield.
- **User benefit:** adapters have a stable ingestion endpoint; workers have a stable event stream.

## Source

`docs/specs/04-cortex-core.md` · depends on specs 01 + 02 · PRD FR-4.

//! Ingestion proxy surface — accepts envelopes from external
//! producers (CLI, MCP host, adapters) and routes them onto the
//! event bus.
//!
//! Previously lived at `crates/cortex-api/src/ingest_proxy.rs`;
//! moved into `ingest/proxy.rs` for thematic grouping. External
//! `cortex_api::ingest_proxy` path is preserved via a `pub use`
//! re-export in [`crate`] (see `lib.rs`).

pub mod proxy;

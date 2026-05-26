//! Search subsystem — orchestrator, intent rewriter, fusion / RRF,
//! analyzer, lane strategies, response cache, byte budget, and the
//! HTTP search proxy.
//!
//! Every file in this bucket previously lived at
//! `crates/cortex-api/src/<name>.rs`. The bucketing is a 1:1 rename
//! to give the module tree a thematic home; external paths are
//! preserved via `pub use search::<child>` re-exports in
//! [`crate`] (see `lib.rs`).

pub mod analyzer;
pub mod budget;
pub mod cache;
pub mod consolidation_get;
pub mod events_by_kind;
pub mod files_touched;
pub mod fusion;
pub mod orchestrator;
pub mod query_rewrite;
pub mod relevance_config;
pub mod search_proxy;
pub mod strategies;
pub mod tool_calls;
pub mod topic_search;

//! Cortex relevance harness (was crate `cortex-relevance-eval`).
//!
//! Loads a labeled query set, replays each entry against a running
//! `cortex-api`, scores `recall@10` + `MRR` per query, aggregates per
//! intent and globally, and emits a deterministic JSON report.
//! Closes F-008 (phase6e).

pub mod harness;
pub mod queries;
pub mod report;

pub use harness::{run_harness, HarnessOptions, ScoredQuery};
pub use queries::{LabeledQuery, QuerySet};
pub use report::{IntentScores, QueryResult, RegressionVerdict, RelevanceReport};

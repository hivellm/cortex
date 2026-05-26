//! Phase14c — golden-set eval harness for Cortex.
//!
//! Three suites measure end-to-end quality regression between
//! Cortex releases:
//!
//! - **retrieval** — MRR@10 + recall@5 against
//!   `tests/golden/retrieval.csv`.
//! - **consolidation** — entity-recall + fact-recall against
//!   `tests/golden/consolidation.csv`.
//! - **classification** — per-kind F1 + macro-F1 against
//!   `tests/golden/classification.csv`.
//!
//! Library surface ships the metric primitives + suite types so
//! the bin (`cortex-eval`) and unit tests share one implementation.
//!
//! Quality bar per the phase14c proposal:
//!
//! | Suite          | Metric           | Floor   |
//! |----------------|------------------|---------|
//! | retrieval      | MRR@10           | 0.60    |
//! | retrieval      | recall@5         | 0.50    |
//! | consolidation  | entity-recall    | 0.85    |
//! | classification | macro-F1         | 0.90    |
//!
//! CI gate (phase14c §4) blocks PRs whose suite drops > 5% on any
//! metric vs the baseline checked into the main branch.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod metrics;
pub mod report;
pub mod suite;

pub use metrics::{f1, macro_f1, mrr_at_k, recall_at_k};
pub use report::{MetricRow, SuiteReport};
pub use suite::{
    classification::{classification_acceptance, ClassificationRow},
    consolidation::{consolidation_acceptance, ConsolidationRow},
    retrieval::{retrieval_acceptance, RetrievalRow},
    AcceptanceVerdict, SuiteName,
};

/// CI regression threshold — a metric whose absolute drop vs the
/// baseline exceeds this fraction fails the eval gate. `0.05` =
/// "5% slip relative to baseline" per the phase14c proposal.
pub const CI_REGRESSION_THRESHOLD: f64 = 0.05;

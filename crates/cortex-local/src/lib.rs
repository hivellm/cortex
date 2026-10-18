//! Standalone local Cortex engine shared by CLI, MCP, hooks and web transports.
#![forbid(unsafe_code)]

pub mod audit;
pub mod classification;
pub mod dashboard;
pub mod feedback;
pub mod feedback_import;
pub mod files;
pub mod graph;
pub mod hooks;
pub mod import;
pub mod index;
pub mod install;
pub mod mcp;
pub mod model;
pub mod ranking;
mod readonly;
pub mod recall;
pub mod runtime;
pub mod store;
pub mod upgrade;
pub mod web;

pub use audit::*;
pub use dashboard::{BrowsePage, BrowseQuery, RepositoryInventory, RepositorySummary};
pub use feedback::{
    FeedbackEligibility, FeedbackEntry, FeedbackEvidence, FeedbackJudgment, FeedbackListPage,
    FeedbackListQuery, FeedbackRecord, FeedbackRequest, FeedbackSource, FeedbackWriteOutcome,
    LEGACY_FEEDBACK_REPOSITORY, MAX_FEEDBACK_BYTES, MAX_FEEDBACK_EVIDENCE,
};
pub use model::*;
pub use ranking::{
    FeedbackExplanation, FeedbackRankingReason, RankingOptions, FEEDBACK_CANDIDATE_LIMIT,
    FEEDBACK_POLICY,
};
pub use store::LocalStore;

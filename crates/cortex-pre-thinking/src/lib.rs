//! Cortex pre-thinking injection (spec 12).
//!
//! Wraps `cortex-api /v1/query` with adapter-side heuristics + a
//! deterministic Markdown formatter + a budget clipper. Returns the
//! string an adapter pastes into the model's system prompt as
//! `additionalContext`.
//!
//! The pipeline is fail-open by construction — every error path
//! returns an empty bundle so the calling adapter never breaks the
//! session.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod breaker;
pub mod budget;
pub mod formatter;
pub mod health_source;
pub mod intent_select;
pub mod metrics;
pub mod pipeline;
pub mod scope;

pub use budget::{clip_to_budget, ClippedBundle, TrimStep};
pub use formatter::{clip_utf8, format_bundle, section_caps, FormatOptions, SnippetTrim};
pub use intent_select::{select as select_intent, select_with, Rule, DEFAULT_RULES};
pub use metrics::Metrics;
pub use pipeline::{
    run, ClosureQueryFn, PreThinkingBudget, PreThinkingInput, PreThinkingOutput, QueryFn,
};
pub use scope::{
    derive as derive_scope, extract_prompt_files, repo_from_cwd, DerivedScope, FileStatus,
    RecentFile, MAX_PROMPT_FILES, RECENT_FILE_MAX_AGE_SECS,
};

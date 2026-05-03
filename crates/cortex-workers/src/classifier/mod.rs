//! Cortex event classifier.
//!
//! The pipeline takes a redacted event, emits `ClassifierOutput` with
//! topics, severity, PII risk, and (for oversize payloads) a summary.
//! Default composition: `Budgeted ← Cached ← (HaikuCli | HaikuSdk)`
//! with `StaticClassifier` as budget-exhausted / offline fallback.

pub mod budget;
pub mod cache;
pub mod composer;
pub mod errors;
pub mod haiku_cli;
pub mod prompt;
pub mod statics;
pub mod stats;
pub mod types;

pub use budget::{BudgetTracker, BudgetedClassifier};
pub use cache::{CacheError, CachedClassifier, ClassifierCache, InMemoryCache};
pub use composer::{build_offline_stack, build_stack, ClassifierStack};
pub use errors::ClassifierError;
pub use haiku_cli::{HaikuCliClassifier, HaikuCliConfig};
pub use prompt::{
    render_consolidate_auto_memory, PromptTemplate, CONSOLIDATE_AUTO_MEMORY_V1, TOPIC_VOCAB_V1,
};
pub use statics::StaticClassifier;
pub use stats::{ClassifierSpend, PricingTable};
pub use types::{
    Classifier, ClassifierMode, ClassifierOutput, ClassifierSource, EnrichmentInput,
    ExtractedEntity, ExtractedRelation, PiiRisk, RedactionSuggestion, Severity,
};

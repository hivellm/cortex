//! Cortex core: event schema, canonical JSON, ULID, content hash, validator.
//!
//! JSON Schemas under `schemas/` are the public wire contract for every Cortex
//! component (adapters, workers, query API, dashboard). Rust types in [`events`]
//! mirror those schemas and are exercised against them in the test suite so
//! neither side can drift.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod canonical_json;
pub mod content_hash;
pub mod events;
pub mod ids;
pub mod validate;
pub mod vocab;

pub use canonical_json::{canonicalize, CanonicalJsonError};
pub use content_hash::{content_hash, ContentHash};
pub use events::{
    AgentCall, AnalysisPayload, ArtifactPayload, Context, DecisionPayload, Envelope, Kind,
    LawViolationPayload, MemoryPayload, Stream, ToolCall, Turn,
};
pub use ids::{event_id, session_id, EventId, SessionId, Ulid};
pub use validate::{validate_event, validate_envelope, ValidationError, Validator};

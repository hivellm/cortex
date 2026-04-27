//! Per-`Kind` collection routing.
//!
//! The table follows `docs/specs/06-embedder.md` §Collections. The returned
//! string is `"{prefix}-{suffix}"` — e.g. `cortex-code`, `cortex-docs`.

use cortex_core::events::Kind;

use crate::chunker::ChunkSource;

/// Return the collection name for a given event kind and deployment prefix.
///
/// Note that for [`Kind::Artifact`] this returns the *doc* fallback —
/// callers that have a [`ChunkSource`] in hand should prefer
/// [`collection_for_chunk`] so code chunks land in `cortex-code` and
/// doc chunks land in `cortex-docs`.
pub fn collection_for(kind: &Kind, prefix: &str) -> String {
    let suffix = match kind {
        // tool_call.* — code symbols (spec: "tool_call.* (code)" → cortex-code).
        Kind::ToolCall => "code",
        // artifact.* — defaults to docs; chunks routed via
        // `collection_for_chunk` get the precise code/docs split.
        Kind::Artifact => "docs",
        Kind::Decision => "decisions",
        Kind::Turn => "turns",
        Kind::LawViolation => "governance",
        // Catch-all for kinds not explicitly called out in the spec table.
        Kind::AgentCall | Kind::Memory | Kind::Analysis => "misc",
    };
    format!("{prefix}-{suffix}")
}

/// Route a single chunk to its destination collection. For
/// [`Kind::Artifact`] the [`ChunkSource`] discriminates between the
/// code lane (`cortex-code`) and the doc lane (`cortex-docs`); other
/// kinds delegate to [`collection_for`] which is event-level.
///
/// `Summary` and `RawOversize` chunks inherit the parent event's
/// classification: they live with the original chunk they replaced or
/// shadow, which is captured here by the `parent_source` argument the
/// substitution pass already has on hand.
pub fn collection_for_chunk(kind: &Kind, source: &ChunkSource, prefix: &str) -> String {
    match (kind, source) {
        (Kind::Artifact, ChunkSource::Code) => format!("{prefix}-code"),
        (Kind::Artifact, ChunkSource::Doc) => format!("{prefix}-docs"),
        (Kind::Artifact, ChunkSource::FallbackWindow) => {
            // No language hint — keep with docs so search still finds it.
            format!("{prefix}-docs")
        }
        // Summary / RawOversize keep whatever the caller already set on
        // the chunk; we just pass through the event-level routing as a
        // safety default.
        _ => collection_for(kind, prefix),
    }
}


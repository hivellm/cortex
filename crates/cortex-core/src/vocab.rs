//! Controlled vocabularies for the envelope.
//!
//! These must stay in sync with the `enum` lists in `schemas/envelope.schema.json`.
//! The test `vocab_matches_schema` in `tests/schema_alignment.rs` enforces this.

use crate::events::Kind;

/// Adapter identifiers allowed in [`crate::events::Envelope::tool`].
pub const TOOL_IDS: &[&str] = &[
    "claude-code",
    "cursor",
    "codex",
    "gemini",
    "copilot",
    "windsurf",
    "cortex-cli",
    "cortex-bootstrap",
    "git-hook",
    "fs-watcher",
];

/// Event discriminators allowed in [`crate::events::Envelope::kind`].
///
/// Phase12e — every variant of [`Kind`] must appear here. The
/// [`const _: () = ...`] assertion below catches drift at compile
/// time so a new variant added to `Kind` without an entry here fails
/// the build instead of silently dropping events at ingest.
pub const KIND_IDS: &[&str] = &[
    "turn",
    "tool_call",
    "agent_call",
    "memory",
    "decision",
    "analysis",
    // phase26a §4 — law definitions (rules files) distinct from violations.
    "law",
    "law_violation",
    "artifact",
    // phase10e — auto-imported `.rulebook/{knowledge,learnings}` corpora.
    "knowledge",
    "learning",
    // phase11j — distilled Session / Topic / DecisionTrace summaries.
    "consolidation",
    // phase11r — living-synthesis topic card.
    "topic_card",
];

/// Compile-time guard: `KIND_IDS.len()` MUST equal [`Kind::COUNT`].
///
/// Adding a new variant to `Kind` without an entry in `KIND_IDS` is a
/// silent-data-loss bug: ingestion validates the envelope's `kind`
/// string against this list, so a missing entry rejects every
/// envelope of that kind. The assertion fires at `cargo check` time
/// — well before any classifier could emit the unknown kind.
const _: () = assert!(
    KIND_IDS.len() == Kind::COUNT,
    "KIND_IDS is out of sync with Kind enum — add the new variant's snake_case stem"
);

/// Streams the ingestion router accepts.
pub const STREAM_IDS: &[&str] = &["live", "bootstrap"];

/// Platform identifiers allowed in [`crate::events::Context::platform`].
pub const PLATFORM_IDS: &[&str] = &["win32", "darwin", "linux"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_kind_ids() {
        // Phase12e §1.4 — every variant of Kind must serialise to a
        // string that appears in KIND_IDS, and KIND_IDS must contain
        // exactly that many distinct entries. The compile-time
        // const_assert catches the count mismatch; this test catches
        // a stem typo (right count, wrong string).
        let stems: Vec<&str> = [
            Kind::Turn,
            Kind::ToolCall,
            Kind::AgentCall,
            Kind::Memory,
            Kind::Decision,
            Kind::Analysis,
            Kind::Law,
            Kind::LawViolation,
            Kind::Artifact,
            Kind::Knowledge,
            Kind::Learning,
            Kind::Consolidation,
            Kind::TopicCard,
        ]
        .iter()
        .map(|k| k.schema_stem())
        .collect();

        assert_eq!(
            stems.len(),
            Kind::COUNT,
            "the manual variant list above is out of sync with Kind::COUNT — \
             add the missing variant"
        );

        for stem in &stems {
            assert!(
                KIND_IDS.contains(stem),
                "KIND_IDS missing entry for `{stem}` — every Kind variant's \
                 schema_stem MUST appear in the vocab"
            );
        }

        // The reverse: every KIND_IDS entry corresponds to a real
        // variant. Walk the array and look up each stem in the
        // variant list — catches accidentally-added orphan entries.
        for id in KIND_IDS {
            assert!(
                stems.contains(id),
                "KIND_IDS has orphan entry `{id}` — no Kind variant maps to it"
            );
        }
    }

    #[test]
    fn kind_ids_has_no_duplicates() {
        let mut seen: Vec<&&str> = Vec::with_capacity(KIND_IDS.len());
        for id in KIND_IDS {
            assert!(
                !seen.contains(&id),
                "KIND_IDS contains duplicate entry `{id}`"
            );
            seen.push(id);
        }
    }
}

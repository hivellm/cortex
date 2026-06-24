//! Schema bootstrap statements for the Cortex graph.
//!
//! Mirrors `docs/specs/07-graph-writer.md` §Schema bootstrapping. Every
//! statement is idempotent: re-running on an already-bootstrapped Nexus
//! instance is a no-op. Failure on any statement is fatal — the worker
//! refuses to accept events against an unknown schema.
//!
//! Statement ordering: constraints first (they are what the writer
//! relies on for upsert idempotency), then secondary indexes used by
//! the read path (spec 11). Each constraint name is unique inside Nexus
//! so re-running the bootstrap is cheap.
//!
//! ## Phase11l §4 — `natural_key` constraints retired
//!
//! The seven `*_natural_key` and the `Symbol.natural_key` uniqueness
//! constraints (`artifact_natural_key`, `symbol_natural_key`,
//! `external_package_natural_key`, `unresolved_import_natural_key`,
//! `doc_section_natural_key`) shipped in phase4c–phase11k were retired
//! in phase11l once the writer started keying on Nexus 2.1's reserved
//! `_id` slot. The external-id catalog index Nexus maintains
//! (`ExternalIdIndex`, phase9 §1.4) enforces uniqueness structurally
//! — a duplicate `_id` cannot be inserted regardless of label, so a
//! per-label `IS UNIQUE` constraint on the legacy `natural_key`
//! property is redundant. Removing them shrinks the bootstrap
//! surface and removes the silent-overwrite trap where two upserts
//! against the same `natural_key` could land on different internal
//! ids when the constraint was missing on a fresh DB. Secondary
//! identity constraints (`session_id`, `turn_id`, …, `repo_name`,
//! `spec_path`) stay because those properties carry semantic meaning
//! beyond `_id` (they are the wire-shape adapters expose to callers).

/// Cypher statements applied at worker startup, in order.
///
/// The list mirrors the schema in spec 07 §Schema bootstrapping
/// (post-phase11l revision): identity constraints on the
/// adapter-facing properties (`Session.id`, `Turn.id`, …,
/// `Repo.name`, `Spec.path`) plus a small set of secondary indexes
/// the read path uses. The legacy `*_natural_key` constraints were
/// retired alongside the §3 templates rewrite — see the module
/// header.
pub const SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE CONSTRAINT session_id IF NOT EXISTS FOR (s:Session) REQUIRE s.id IS UNIQUE",
    "CREATE CONSTRAINT turn_id IF NOT EXISTS FOR (t:Turn) REQUIRE t.id IS UNIQUE",
    "CREATE CONSTRAINT tool_call_id IF NOT EXISTS FOR (tc:ToolCall) REQUIRE tc.id IS UNIQUE",
    "CREATE CONSTRAINT decision_id IF NOT EXISTS FOR (d:Decision) REQUIRE d.id IS UNIQUE",
    "CREATE CONSTRAINT memory_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.id IS UNIQUE",
    "CREATE CONSTRAINT analysis_id IF NOT EXISTS FOR (a:Analysis) REQUIRE a.id IS UNIQUE",
    "CREATE CONSTRAINT law_id IF NOT EXISTS FOR (l:Law) REQUIRE l.id IS UNIQUE",
    "CREATE CONSTRAINT violation_id IF NOT EXISTS FOR (v:LawViolation) REQUIRE v.id IS UNIQUE",
    "CREATE CONSTRAINT repo_name IF NOT EXISTS FOR (r:Repo) REQUIRE r.name IS UNIQUE",
    "CREATE CONSTRAINT spec_path IF NOT EXISTS FOR (s:Spec) REQUIRE s.path IS UNIQUE",
    // Phase18 §2.2 — Branch identity. Composite `<project>:<name>`
    // id per ADR-019.
    "CREATE CONSTRAINT branch_id IF NOT EXISTS FOR (b:Branch) REQUIRE b.id IS UNIQUE",
    // Phase18 §2.3 — TimelineEvent identity.
    "CREATE CONSTRAINT timeline_event_id IF NOT EXISTS FOR (t:TimelineEvent) REQUIRE t.id IS UNIQUE",
    "CREATE INDEX artifact_repo_path IF NOT EXISTS FOR (a:Artifact) ON (a.repo, a.path)",
    "CREATE INDEX turn_ts IF NOT EXISTS FOR (t:Turn) ON (t.ts)",
    "CREATE INDEX tool_call_name IF NOT EXISTS FOR (tc:ToolCall) ON (tc.tool_name)",
    "CREATE INDEX symbol_repo_name IF NOT EXISTS FOR (s:Symbol) ON (s.repo, s.name)",
    // Phase18 §2.5 — bitemporal retrieval indexes per design.md §1.1.
    // The temporal classifier (§3) walks `(project_id, valid_to)` to
    // drop EXPIRED rows, `(project_id, branch_id, valid_from)` to
    // scope branch retrievals, and `(superseded_at)` to drop
    // SUPERSEDED rows. Indexed on every label that carries the
    // bitemporal stamp (graph/bitemporal.rs writes the columns
    // uniformly).
    "CREATE INDEX decision_project_valid_to IF NOT EXISTS FOR (d:Decision) ON (d.project_id, d.valid_to)",
    "CREATE INDEX decision_project_branch_valid_from IF NOT EXISTS FOR (d:Decision) ON (d.project_id, d.branch_id, d.valid_from)",
    "CREATE INDEX decision_superseded_at IF NOT EXISTS FOR (d:Decision) ON (d.superseded_at)",
    "CREATE INDEX memory_project_valid_to IF NOT EXISTS FOR (m:Memory) ON (m.project_id, m.valid_to)",
    "CREATE INDEX memory_project_branch_valid_from IF NOT EXISTS FOR (m:Memory) ON (m.project_id, m.branch_id, m.valid_from)",
    "CREATE INDEX memory_superseded_at IF NOT EXISTS FOR (m:Memory) ON (m.superseded_at)",
    "CREATE INDEX analysis_project_valid_to IF NOT EXISTS FOR (a:Analysis) ON (a.project_id, a.valid_to)",
    "CREATE INDEX analysis_superseded_at IF NOT EXISTS FOR (a:Analysis) ON (a.superseded_at)",
    "CREATE INDEX law_violation_project_valid_to IF NOT EXISTS FOR (v:LawViolation) ON (v.project_id, v.valid_to)",
    "CREATE INDEX timeline_event_project_branch_valid_time IF NOT EXISTS FOR (t:TimelineEvent) ON (t.project_id, t.branch_id, t.valid_time)",
    "CREATE INDEX branch_project IF NOT EXISTS FOR (b:Branch) ON (b.project_id, b.status)",
    // phase25 §3 — single-property indexes on every node-MERGE key.
    // The writer upserts each node as `MERGE (n:Label { <key>: <v> })`
    // (key = `LiveNexusClient::key_field_for`: Artifact->natural_key,
    // Repo->name, all others->id). Nexus 2.3.1's NodeIndexSeek (#8) +
    // index-backed MERGE existence (#9) only engage when a *single-prop*
    // property index covers `(label, key)`. A unique CONSTRAINT does
    // NOT back a planner-usable index on 2.3.1 (verified: a seek on a
    // constraint-only id still emits `UnindexedPropertyAccess`), and
    // the composite `artifact_repo_path` / `symbol_repo_name` indexes
    // above do not cover the natural_key / id MERGE key. Without these
    // the edge-upsert endpoint MATCH falls back to an O(N) label scan
    // and edge writes melt the server on a large graph.
    "CREATE INDEX artifact_natural_key_seek IF NOT EXISTS FOR (a:Artifact) ON (a.natural_key)",
    "CREATE INDEX repo_name_idx IF NOT EXISTS FOR (r:Repo) ON (r.name)",
    "CREATE INDEX session_id_idx IF NOT EXISTS FOR (s:Session) ON (s.id)",
    "CREATE INDEX turn_id_idx IF NOT EXISTS FOR (t:Turn) ON (t.id)",
    "CREATE INDEX tool_call_id_idx IF NOT EXISTS FOR (tc:ToolCall) ON (tc.id)",
    "CREATE INDEX agent_call_id_idx IF NOT EXISTS FOR (ac:AgentCall) ON (ac.id)",
    "CREATE INDEX symbol_id_idx IF NOT EXISTS FOR (s:Symbol) ON (s.id)",
    "CREATE INDEX decision_id_idx IF NOT EXISTS FOR (d:Decision) ON (d.id)",
    "CREATE INDEX memory_id_idx IF NOT EXISTS FOR (m:Memory) ON (m.id)",
    "CREATE INDEX analysis_id_idx IF NOT EXISTS FOR (a:Analysis) ON (a.id)",
    "CREATE INDEX knowledge_id_idx IF NOT EXISTS FOR (k:Knowledge) ON (k.id)",
    "CREATE INDEX learning_id_idx IF NOT EXISTS FOR (l:Learning) ON (l.id)",
    "CREATE INDEX law_id_idx IF NOT EXISTS FOR (l:Law) ON (l.id)",
    "CREATE INDEX law_violation_id_idx IF NOT EXISTS FOR (v:LawViolation) ON (v.id)",
    "CREATE INDEX consolidation_id_idx IF NOT EXISTS FOR (c:Consolidation) ON (c.id)",
    "CREATE INDEX topic_card_id_idx IF NOT EXISTS FOR (tc:TopicCard) ON (tc.id)",
    "CREATE INDEX branch_id_idx IF NOT EXISTS FOR (b:Branch) ON (b.id)",
    "CREATE INDEX timeline_event_id_idx IF NOT EXISTS FOR (t:TimelineEvent) ON (t.id)",
    // phase23d — non-code parser node labels
    "CREATE INDEX schema_table_nk_idx IF NOT EXISTS FOR (t:SchemaTable) ON (t.natural_key)",
    "CREATE INDEX schema_nk_idx IF NOT EXISTS FOR (s:Schema) ON (s.natural_key)",
    "CREATE INDEX infra_resource_nk_idx IF NOT EXISTS FOR (r:InfraResource) ON (r.natural_key)",
    "CREATE INDEX service_nk_idx IF NOT EXISTS FOR (s:Service) ON (s.natural_key)",
    "CREATE INDEX config_nk_idx IF NOT EXISTS FOR (c:Config) ON (c.natural_key)",
    "CREATE INDEX endpoint_nk_idx IF NOT EXISTS FOR (e:Endpoint) ON (e.natural_key)",
];

/// Owned-string clone of [`SCHEMA_STATEMENTS`] for callers (like
/// [`super::nexus_client::GraphClient::ensure_schema`]) that want a
/// `Vec<String>` they can push into.
pub fn statements() -> Vec<String> {
    SCHEMA_STATEMENTS.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_key_constraints_are_retired_post_phase11l() {
        // Phase11l §4.1 — the seven `natural_key`-keyed CONSTRAINT
        // statements are no longer part of the bootstrap. Nexus 2.1's
        // ExternalIdIndex enforces uniqueness on `_id` structurally.
        // A regression here means the writer is back to declaring
        // redundant constraints that conflict with the index seek path.
        //
        // phase25 §3 nuance: the retirement is about the *constraints*,
        // not the property. The writer still upserts Artifact by
        // `natural_key` (`key_field_for(Artifact)`), and Nexus 2.3.1's
        // NodeIndexSeek (#8) needs a single-prop INDEX on that key — so
        // exactly one `CREATE INDEX ... ON (a.natural_key)` is allowed
        // (and required). What stays forbidden is any natural_key
        // CONSTRAINT.
        let joined = SCHEMA_STATEMENTS.join("\n");
        for stmt in SCHEMA_STATEMENTS {
            assert!(
                !(stmt.contains("CONSTRAINT") && stmt.contains("natural_key")),
                "phase11l §4.1: natural_key CONSTRAINTs are retired; got `{stmt}`"
            );
        }
        // The single Artifact natural_key SEEK index is present (phase25
        // §3) — without it the Artifact MERGE/endpoint MATCH is an O(N)
        // label scan.
        assert!(
            joined.contains("FOR (a:Artifact) ON (a.natural_key)"),
            "phase25 §3: the Artifact natural_key seek index must be present"
        );
        // No legacy per-label natural_key CONSTRAINTs sneak back.
        for retired in [
            "symbol_natural_key",
            "external_package_natural_key",
            "unresolved_import_natural_key",
            "doc_section_natural_key",
        ] {
            assert!(
                !joined.contains(retired),
                "phase11l §4.1 dropped `{retired}`; it must not return to SCHEMA_STATEMENTS"
            );
        }
    }

    #[test]
    fn secondary_identity_constraints_remain() {
        // The adapter-facing identity properties keep their
        // uniqueness constraints — they encode wire-shape contracts
        // beyond the `_id` slot (callers query by these values, the
        // dashboard projects them as captions, etc.).
        let joined = SCHEMA_STATEMENTS.join("\n");
        for kept in [
            "session_id",
            "turn_id",
            "tool_call_id",
            "decision_id",
            "memory_id",
            "analysis_id",
            "law_id",
            "violation_id",
            "repo_name",
            "spec_path",
        ] {
            assert!(
                joined.contains(kept),
                "phase11l §4.2 kept `{kept}` as a secondary check; it MUST remain"
            );
        }
    }

    #[test]
    fn statements_helper_round_trips_const() {
        let cloned = statements();
        assert_eq!(cloned.len(), SCHEMA_STATEMENTS.len());
        for (i, owned) in cloned.iter().enumerate() {
            assert_eq!(owned.as_str(), SCHEMA_STATEMENTS[i]);
        }
    }

    #[test]
    fn every_statement_is_idempotent() {
        // Every statement MUST carry an `IF NOT EXISTS` clause —
        // re-running the bootstrap is part of the worker's startup
        // contract, and a non-idempotent CREATE would refuse the
        // second boot.
        for stmt in SCHEMA_STATEMENTS {
            assert!(
                stmt.contains("IF NOT EXISTS"),
                "non-idempotent statement: {stmt}"
            );
        }
    }

    #[test]
    fn phase18_branch_and_timeline_event_constraints_landed() {
        // §2.2 + §2.3 pin: Branch + TimelineEvent identity
        // constraints. Without these the temporal classifier's
        // composite-id seek on `<project>:<branch>` would fall back
        // to a label scan.
        let joined = SCHEMA_STATEMENTS.join("\n");
        assert!(
            joined.contains("FOR (b:Branch) REQUIRE b.id IS UNIQUE"),
            "phase18 §2.2 Branch identity constraint missing"
        );
        assert!(
            joined.contains("FOR (t:TimelineEvent) REQUIRE t.id IS UNIQUE"),
            "phase18 §2.3 TimelineEvent identity constraint missing"
        );
    }

    #[test]
    fn phase18_bitemporal_indexes_landed() {
        // §2.5 pin: the temporal classifier MUST be able to seek by
        // `(project_id, valid_to)`, `(project_id, branch_id,
        // valid_from)`, and `(superseded_at)` on the lifecycle-
        // tracking labels. A regression would force a full label
        // scan on every retrieval.
        let joined = SCHEMA_STATEMENTS.join("\n");
        for required in [
            // (project_id, valid_to)
            "decision_project_valid_to",
            "memory_project_valid_to",
            "analysis_project_valid_to",
            "law_violation_project_valid_to",
            // (project_id, branch_id, valid_from)
            "decision_project_branch_valid_from",
            "memory_project_branch_valid_from",
            // (superseded_at)
            "decision_superseded_at",
            "memory_superseded_at",
            "analysis_superseded_at",
            // TimelineEvent + Branch composite indexes
            "timeline_event_project_branch_valid_time",
            "branch_project",
        ] {
            assert!(
                joined.contains(required),
                "phase18 §2.5 index `{required}` missing"
            );
        }
    }
}

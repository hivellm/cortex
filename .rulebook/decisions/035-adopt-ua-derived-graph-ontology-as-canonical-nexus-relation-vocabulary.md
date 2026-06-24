# 35. Adopt UA-derived graph ontology as canonical Nexus relation vocabulary

**Status**: proposed
**Date**: 2026-06-24
**Related Tasks**: phase23a_ua-graph-ontology

## Context

Cortex's code/doc graph lane was limited to a handful of relations (IMPORTS_FILE, DOCUMENTED_BY, CITES, CALLS, IMPLEMENTS, EXTENDS, CONTAINS — 18 EdgeType variants total). That vocabulary is too thin to answer structural questions: "what touches table X", "if I change this file what breaks", "which service exposes this endpoint". The Understand-Anything (UA) analysis (`docs/analysis/understand-anything/`) documents a battle-tested ontology — 21 node types and 35 edge types spanning code, infra, data, domain, and knowledge — compiled from the UA TypeScript reference (`packages/core/src/types.ts`). Cortex already covers 8 of the 18 UA structural edges (via aliases). The remaining 22 adopted edges + 17 adopted node kinds are additive — they do not replace any existing relation.

## Decision

Adopt the ✅ subset of the UA taxonomy as the canonical Nexus graph relation vocabulary for Cortex (phase23a). 

Adopted node kinds (17): file, function, class, module, config, document, service, table, endpoint, pipeline, schema, resource, article, entity, topic, claim, source.

Adopted edge kinds (22): imports, exports, contains, inherits, implements, calls, reads_from, writes_to, depends_on, tested_by, configures, deploys, provisions, triggers, migrates, documents, routes, defines_schema, cites, contradicts, builds_on, categorized_under.

Implementation:
1. Add a `NodeKind` enum to `cortex-storage/src/graph.rs` covering all 17 UA-adopted kinds + all Cortex-only kinds (Session, Turn, ToolCall, AgentCall, Memory, Decision, Analysis, Law, LawViolation, Artifact, Repo, Symbol, DocSection, ExternalPackage, UnresolvedImport, Knowledge, Learning, Consolidation, TopicCard).
2. Add 15 new variants to `EdgeType` in `cortex-workers/src/graph/analyzer/mod.rs` for the UA edges not yet in the enum (Exports, ReadsFrom, WritesTo, DependsOn, TestedBy, Configures, Deploys, Provisions, Triggers, Migrates, Routes, DefinesSchema, Contradicts, BuildsOn, CategorizedUnder). Existing variants are kept unchanged (backward compat). Deprecated aliases documented: ImportsFile→imports, Documents/DocumentedBy→documents, Cites→cites.
3. Extend `EdgeOp` in `cortex-workers/src/graph/patch.rs` with the UA edge shape fields: direction (Forward/Backward/Bidirectional), weight (f32), description (Option<String>), provenance (Option<String>), valid_from (Option<i64>), valid_to (Option<i64>). All optional/serde-default for backward compat.
4. Add `EdgeType::from_nexus_label()` for round-trip deserialization including legacy alias resolution.

Cortex-only edges preserved: SUPERSEDES (decisions), GOVERNED_BY (planned), DERIVED_FROM/CONSOLIDATES (memory provenance), plus all session-graph relations (CONTAINS, INVOKED, READ, WROTE, EXECUTED, etc.).

The two ontologies compose — UA fills the code/infra/doc-structure layer beneath Cortex's session/decision/memory layer.

## Alternatives Considered

- Keep existing thin vocabulary and add edges ad-hoc per-phase: slower, divergent naming, no shared contract
- Adopt full UA taxonomy (all 35 edges): brings in domain/flow/step and semantic edges (related, similar_to) that are too speculative for Cortex's deterministic-first policy
- Use a different ontology (Schema.org, CodeOntology): less battle-tested against LLM-driven code analysis workflows, no direct evidence in the codebase

## Consequences

Positive: richer structural graph enabling blast-radius queries, shared vocabulary that later extraction/parser phases (23b-23e) emit into, contradicts/builds_on edges enable graph-layer contradiction detection for consolidation quality. Negative: 15 new EdgeType variants increase the exhaustive-match surface in language analyzers (compiler-enforced). EdgeOp gains 6 optional fields — pre-23a edges read back as zero-weight forward-direction edges (additive, not breaking). GOVERNED_BY and CONSOLIDATES are documented in ADR but not yet in code (planned for later phases).

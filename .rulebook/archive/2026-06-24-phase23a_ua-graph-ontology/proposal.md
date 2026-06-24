# Proposal: phase23a_ua-graph-ontology

## Why

Cortex's code/doc graph lane currently expresses only a handful of relations
(`IMPORTS_FILE`, `DOCUMENTED_BY`, `CITES`). That is too thin to answer structural
questions ("what touches table X", "if I change this file what breaks", "which
service exposes this endpoint"). The Understand-Anything analysis
(`docs/analysis/understand-anything/`) documents a battle-tested ontology — 21 node
types and 35 edge types spanning code, infra, data, domain, and knowledge — that is a
superset of what Cortex emits today. Adopting it as the canonical Nexus relation
vocabulary is the foundation every later phase (incremental indexer, parsers,
docs-as-graph) builds on. This phase defines the ontology and the bitemporal-aware
edge shape; it does not yet populate the new node/edge kinds.

Source: `docs/analysis/understand-anything/03-ontology-mapping.md`,
`docs/analysis/understand-anything/02-findings.md` (F-4).

## What Changes

- Extend the Cortex graph node-kind and edge-kind enums (`cortex-core` /
  `cortex-storage` graph types + the Nexus relation vocabulary) with the adopted
  subset of the UA taxonomy (see the crosswalk in `03-ontology-mapping.md`).
- Adopt UA's edge record shape `{source, target, type, direction, weight,
  description?}` and extend it with Cortex's bitemporal envelope (`valid_from`,
  `valid_to`) and `provenance` — fields UA lacks.
- Keep Cortex-only edges (`SUPERSEDES`, `GOVERNED_BY`, `DERIVED_FROM`/`CONSOLIDATES`)
  and Cortex-only nodes (`session`, `decision`, `law`, `consolidation`, `turn`,
  `tool_call`) — the two ontologies compose, they do not replace each other.
- Write an ADR recording the adoption, citing UA as prior art.

## Impact

- Affected specs: graph ontology / relation vocabulary (this task's spec delta);
  downstream phases 23b–23e depend on it.
- Affected code: `crates/cortex-core` graph types, `crates/cortex-storage` graph
  schema / Nexus relation enum, edge serialization.
- Breaking change: NO (additive enum variants; existing relations are aliased, not
  removed).
- User benefit: a richer graph that can answer structural and blast-radius questions,
  and a shared vocabulary that later extraction/parser phases emit into.

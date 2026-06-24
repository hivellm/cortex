# Proposal: phase23d_ua-noncode-parsers

## Why

Cortex's graph covers source code but is blind to infrastructure and data files —
`.sql`, Terraform, protobuf, GraphQL, Dockerfiles — so it cannot answer "what touches
table X", "which resource does this module provision", or "who exposes this endpoint".
In a multi-repo ecosystem (Vectorizer, Nexus, Synap…) these relationships are
high-value. Understand-Anything ships a pluggable parser registry that feeds the same
node/edge schema from deterministic non-code parsers. This phase adds a parser registry
and the highest-value parsers, emitting into the ontology from phase23a through the
reconciliation gate from phase23c.

Source: `docs/analysis/understand-anything/06-parsers.md`,
`docs/analysis/understand-anything/02-findings.md` (F-6).

## What Changes

- Add a deterministic `Parser` trait + registry (match by extension/filename; code
  falls back to the existing extractor).
- Implement parsers in priority order, each emitting the adopted node/edge kinds:
  - SQL → `table`/`schema` + `defines_schema`/`migrates`/`reads_from`/`writes_to`
  - Terraform → `resource`/`service` + `provisions`/`depends_on`
  - protobuf → `schema`/`endpoint`/`service` + `defines_schema`/`routes`
  - GraphQL → `schema`/`endpoint` + `defines_schema`/`routes`
  - Dockerfile → `config`/`service` + `deploys`/`depends_on`
- Each parser output passes through the reconciliation gate (phase23c) and is golden-
  file tested.

## Impact

- Affected specs: non-code parser registry (this task's spec delta).
- Affected code: `crates/cortex-workers` (or adapter) parser registry + per-parser
  modules, graph upsert wiring.
- Breaking change: NO (additive node/edge population over the phase23a vocabulary).
- User benefit: the graph answers infra/data questions — tables, provisioned
  resources, service/API contracts — across the ecosystem.

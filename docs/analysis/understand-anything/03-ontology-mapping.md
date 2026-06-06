# Ontology Crosswalk — UA ↔ Cortex/Nexus

Full mapping of Understand-Anything's node/edge taxonomy onto Cortex's graph, with adopt/skip
notes. Source: `packages/core/src/types.ts` (see [findings.md](02-findings.md) F-4).

---

## 1. Node types (UA → Cortex)

| UA node | Group | Adopt? | Cortex mapping / note |
|---------|-------|--------|------------------------|
| `file` | code | ✅ | Already implied by `IMPORTS_FILE`; make a first-class node |
| `function` | code | ✅ | New code-granularity node (ID `function:<path>:<name>`) |
| `class` | code | ✅ | New |
| `module` | code | ✅ | Maps to crate/package |
| `concept` | code | ◐ | Overlaps Cortex `topic`; merge into topic |
| `config` | non-code | ✅ | From parsers (TOML/YAML/env) |
| `document` | non-code | ✅ | Cortex already has `DOCUMENTED_BY`; make doc a node |
| `service` | non-code | ✅ | HiveLLM services (Vectorizer, Nexus…) |
| `table` | non-code | ✅ | From SQL parser |
| `endpoint` | non-code | ✅ | From GraphQL/protobuf/route parsers |
| `pipeline` | non-code | ✅ | CI / data pipelines |
| `schema` | non-code | ✅ | From protobuf/GraphQL/SQL DDL |
| `resource` | non-code | ✅ | From Terraform |
| `domain` | domain | ◐ | Business-domain layer; optional for Cortex |
| `flow` | domain | ◐ | Optional |
| `step` | domain | ◐ | Optional |
| `article` | knowledge | ✅ | Wiki/doc article (Karpathy KB) |
| `entity` | knowledge | ✅ | Named entity in docs |
| `topic` | knowledge | ✅ | **Already in Cortex** (topic cards) — unify |
| `claim` | knowledge | ✅ | First-class claim node → graph-layer contradiction |
| `source` | knowledge | ✅ | Citation source |

✅ adopt · ◐ optional/merge.

**Cortex additions UA lacks:** `session`, `decision`/ADR, `law`, `consolidation`, `turn`,
`tool_call`. Cortex's graph is memory-centric; UA's is structure-centric. They compose — UA fills
the code/infra/doc-structure layer beneath Cortex's session/decision layer.

---

## 2. Edge types (UA → Cortex)

| UA edge | Category | Adopt? | Note |
|---------|----------|--------|------|
| `imports` | structural | ✅ | = Cortex `IMPORTS_FILE` (rename or alias) |
| `exports` | structural | ✅ | |
| `contains` | structural | ✅ | file→function, module→file |
| `inherits` | structural | ✅ | |
| `implements` | structural | ✅ | |
| `calls` | behavioral | ✅ | call graph |
| `subscribes` | behavioral | ◐ | event systems |
| `publishes` | behavioral | ◐ | event systems |
| `middleware` | behavioral | ◐ | web frameworks |
| `reads_from` | data flow | ✅ | code→table/resource |
| `writes_to` | data flow | ✅ | |
| `transforms` | data flow | ◐ | |
| `validates` | data flow | ◐ | |
| `depends_on` | dependency | ✅ | generic dep |
| `tested_by` | dependency | ✅ | code↔test (blast radius) |
| `configures` | dependency | ✅ | config→service |
| `related` | semantic | ◐ | weak edge; keep weighted |
| `similar_to` | semantic | ◐ | embedding-derived |
| `deploys` | infra | ✅ | |
| `serves` | infra | ◐ | |
| `provisions` | infra | ✅ | Terraform resource |
| `triggers` | infra | ✅ | hooks/CI |
| `migrates` | schema | ✅ | SQL migrations |
| `documents` | schema | ✅ | = Cortex `DOCUMENTED_BY` |
| `routes` | schema | ✅ | endpoint routing |
| `defines_schema` | schema | ✅ | protobuf/GraphQL/DDL |
| `contains_flow` | domain | ◐ | domain layer only |
| `flow_step` | domain | ◐ | |
| `cross_domain` | domain | ◐ | |
| `cites` | knowledge | ✅ | = Cortex `CITES` |
| `contradicts` | knowledge | ✅ | **graph-layer contradiction** (high value) |
| `builds_on` | knowledge | ✅ | claim supersession-ish |
| `exemplifies` | knowledge | ◐ | |
| `categorized_under` | knowledge | ✅ | topic taxonomy |
| `authored_by` | knowledge | ◐ | provenance |

**Cortex-only edges to keep:** `SUPERSEDES` (decisions), `GOVERNED_BY` (laws),
`DERIVED_FROM`/`CONSOLIDATES` (memory), plus the bitemporal `valid_from/valid_to` envelope which
UA entirely lacks.

---

## 3. Edge shape decision

Adopt UA's edge record, extend with Cortex bitemporal + provenance:

```text
Edge {
  source: NodeId
  target: NodeId
  type: EdgeType            // from unified taxonomy above
  direction: forward | backward | bidirectional
  weight: f32               // 0..1, semantic strength
  description: Option<String>
  // Cortex extensions (UA lacks all of these):
  valid_from: Timestamp
  valid_to:   Option<Timestamp>   // bitemporal close
  provenance: Source              // which extractor/agent emitted it
}
```

---

## 4. Verification gate for Phase A

Every relation Cortex currently emits MUST have a row in §2 with ✅ and a target — no orphan
Cortex relation, no UA edge adopted without a documented Cortex use. Re-run this check before
extending the Nexus enum.

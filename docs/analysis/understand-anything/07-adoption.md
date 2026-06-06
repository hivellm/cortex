# Understand-Anything → Cortex — Adoption Map & Phased Plan

Concrete mapping of each borrowable concept to a Cortex crate/subsystem, with effort, risk, and a
sequenced plan. Cross-references [findings.md](02-findings.md) (F-N) and the detailed specs in this
folder.

---

## 1. Borrow / adapt / reject matrix

| UA concept (finding) | Decision | Cortex home | Effort | Risk |
|----------------------|----------|-------------|--------|------|
| Node/edge ontology (F-4) | **Borrow** (port taxonomy) | Nexus relation vocab + `cortex-core` types | M | Low |
| Incremental patch algorithm (F-1) | **Borrow** | `cortex-workers` graph/embed indexer | M | Low |
| Change classifier tiers (F-2) | **Adapt** | consolidation scheduler | S | Low |
| Two-phase analyzer + reconciliation contract (F-3) | **Borrow** | adapter / extraction worker | M | Med |
| Knowledge-base claim graph (F-5) | **Adapt** | docs/decisions lane + topic cards | M | Med |
| Non-code parsers (F-6) | **Adapt** (Rust ports, prioritized) | adapter parser registry | L | Med |
| Auto-update hooks (F-7) | **Confirm/extend** | Cortex hook surface | S | Low |
| Guided tours (F-8) | **Park** | GUI onboarding | — | — |
| Diff impact (F-9) | **Adapt** | `cortex-pre-thinking` enrichment | S | Low |
| Fuse.js / in-mem cosine | **Reject** | (Cortex hybrid RRF already superior) | — | — |
| Single-JSON graph storage | **Reject** | (Nexus is the store) | — | — |

Effort: S ≤ 1 day-ish, M = a few days, L = multi-week.

---

## 2. Per-target detail

### 2.1 Nexus relation vocabulary (F-4) — `ontology-mapping.md`
Adopt UA's 35-edge / 21-node taxonomy as the canonical relation set, mapping Cortex's existing
`IMPORTS_FILE`/`DOCUMENTED_BY`/`CITES` into it (see [ontology-mapping.md](03-ontology-mapping.md) for
the full crosswalk + the proposed Cortex/Nexus type list). Keep UA's edge shape:
`{source, target, type, direction, description?, weight}`. Add Cortex-specific `valid_from/valid_to`
for bitemporal (UA has none — Cortex's timeline work is strictly ahead here).

### 2.2 Incremental graph indexer (F-1, F-2) — `incremental-patching.md`
Implement `fingerprint = last_indexed_commit_hash` per repo in the worker. On commit/session:
`git diff <hash>..HEAD --name-only` → classify (SKIP/PARTIAL/ARCH/FULL) → patch. Full algorithm,
data structures, and edge-pruning rules in [incremental-patching.md](04-incremental-patching.md).

### 2.3 Extraction reconciliation contract (F-3) — `extraction-contract.md`
Adopt the "deterministic facts gate the LLM" rule: extractor emits the authoritative fact set;
LLM may only annotate; a reconciliation step rejects unreconciled edges and enforces import-count
equality. Spec + rejection rules in [extraction-contract.md](05-extraction-contract.md).

### 2.4 Knowledge/claim graph (F-5)
Materialize `claim`/`entity`/`topic`/`source` nodes + `contradicts`/`builds_on` edges over
Cortex docs + decisions. Lets contradiction detection run at the graph layer (retrieval-time),
complementing consolidation-time detection. Reuses topic-card design already in `prethinking`.

### 2.5 Non-code parser registry (F-6) — `parsers.md`
Pluggable parser trait in the adapter; prioritize SQL, Terraform, protobuf, GraphQL, Dockerfile
(highest ecosystem value). Full per-parser node/edge output table in [parsers.md](06-parsers.md).

### 2.6 Diff-impact pre-thinking enrichment (F-9)
New optional band in the pre-thinking bundle: seed = changed files from `git status`, expand
1–2 hops over `imports`/`depends_on`/`tested_by`, attach blast-radius decisions/laws/sessions.
Pure reuse of existing graph-expansion + fits the fail-open bundle model.

---

## 3. Phased execution plan

> Sequenced per LAW-CORTEX-001 discipline; each phase ends with a verifiable gate.
> These are **proposed** phases for a future Rulebook task — not yet materialized.

**Phase A — Ontology (foundation).**
A1. Crosswalk Cortex relations → UA taxonomy (doc) → verify: every current relation maps.
A2. Extend Nexus relation enum + `cortex-core` edge type → verify: `cargo check`.
A3. ADR: "Adopt UA-derived graph ontology" citing UA as prior art → verify: `rulebook_decision_create`.

**Phase B — Incremental indexer.**
B1. Per-repo `last_indexed_commit_hash` persistence → verify: round-trips.
B2. `git diff` changed-file resolver + node-id↔filePath index → verify: unit test on a fixture repo.
B3. Change classifier (SKIP/PARTIAL/ARCH/FULL) → verify: table-driven tests per threshold.
B4. `merge_graph_update` (remove-by-filePath, prune dangling edges, append) → verify: idempotent re-run = no-op.

**Phase C — Extraction contract.**
C1. Reconciliation gate (endpoints ∈ fact set; import-count equality) → verify: rejects a seeded hallucinated edge.

**Phase D — Parsers (incremental, one per sub-task).**
D1 SQL → D2 Terraform → D3 protobuf → D4 GraphQL → D5 Dockerfile; each → verify: golden-file node/edge output.

**Phase E — Pre-thinking diff-impact band.**
E1. Blast-radius expander + bundle band (fail-open) → verify: returns ∅ on error, non-empty on a known diff.

---

## 4. Open questions for the user

1. **Graph store:** confirm Nexus is the sole graph backend (UA's JSON file is rejected) — assumed yes.
2. **Parser priority:** is the SQL→Terraform→protobuf→GraphQL→Dockerfile order right for the HiveLLM stack?
3. **Scope:** single-repo first (Cortex itself) or workspace-wide (all HiveLLM repos) for the indexer?
4. **License:** OK to reimplement-from-spec and cite UA, vs. vendoring any TS? (default: reimplement.)

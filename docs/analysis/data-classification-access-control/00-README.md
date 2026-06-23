# Analysis — Data Classification & Access Control (phase21)

**Subject:** Adding a data-classification + access-control plane to Cortex.
**Date:** 2026-06-23.
**ADRs produced:** ADR-028 through ADR-034 (classification model, principal source, default posture, enforcement strategy, merge semantics, authority model, classification vs redaction).
**Method:** Threat-model analysis of the existing single-trust-domain architecture; enumeration of attack surfaces; mapping of the phase18 bitemporal column-stamper + retrieval-wedge pattern onto the classification axis; survey of Bell-LaPadula MAC literature for the lattice predicate.

> Per [`.claude/rules/consult-analysis-before-implementing.md`](../../../.claude/rules/consult-analysis-before-implementing.md): read this index before implementing any phase21 item, and check `rulebook_knowledge_list` for `analysis:data-classification-access-control`.

---

## 1. Problem statement

Cortex today is a single-trust-domain store. Any caller that can reach the daemon can retrieve every indexed fact — decisions, consolidations, turns, source files — across every repo. That is the correct default for a single maintainer on a local stack. It is a hard blocker for any multi-user enterprise deployment.

The gap is not authentication (API key auth already exists) but **authorization at the data level**: there is no sensitivity label on stored facts and no clearance on query principals. A junior engineer querying "what's our runway?" can surface the CFO's board-deck consolidation because both live in the same flat retrieval corpus.

---

## 2. Threat model

### 2.1 Threats addressed by this phase

| # | Threat | Attacker / mis-query | Control that stops it |
|---|--------|---------------------|----------------------|
| T1 | Horizontal data leak — low-clearance user retrieves `restricted` fact | Under-cleared API key caller; a `/v1/query` or MCP tool call | Per-lane backend filter + post-fusion ACL wedge (ADR-031) |
| T2 | Compartment leak — cleared-for-level but wrong compartment | A principal cleared to `confidential` but not `[financial]` retrieving a board deck | Compartment subset check in `can_read` (ADR-028) |
| T3 | Unauthenticated access to classified data | An HTTP caller without an API key | `deny_on_missing_principal` default = public-only when AC enabled (ADR-030) |
| T4 | Privilege escalation via grant API | A `confidential` principal grants itself `restricted` | No-escalation invariant on `/v1/acl/grant` (ADR-033) |
| T5 | Classifier downgrade attack | A malicious / misconfigured classifier lowers `restricted → public` | Declared floor is inviolable; classifier may only escalate (ADR-032) |
| T6 | Raw proxy bypass — using `/v1/search/vector` directly to bypass orchestrator | A caller that knows the raw API | Raw proxy independently enforces the principal filter (ADR-031 §5.7) |
| T7 | Pre-thinking bundle leak — classified fact enters the LLM prompt | An agent-constructed pre-thinking call with an under-cleared API key | Bundle assembler applies `can_read` before any section is added (ADR-031 §5.6) |
| T8 | Storage breach leaks secrets in classified facts | Backend DB dump by attacker | PII redaction runs BEFORE classification stamping; secrets are erased at ingestion regardless of class level (ADR-034) |
| T9 | Admin action without audit trail | Operator grants clearance without accountability | Every Tier-1/Tier-2 mutation recorded as `acl_admin_action` event (ADR-033) |

### 2.2 Threats explicitly out of scope for v1

| # | Threat | Why deferred |
|---|--------|-------------|
| T10 | Timing side-channel via query latency (reveals classified rows exist) | Per-lane backend filter mitigates the worst case; full mitigation requires homomorphic search or fixed-time responses — deferred |
| T11 | Insider threat by an `acl_admin` principal | The audit trail limits damage but cannot prevent a rogue admin; full separation-of-duties requires HSM-backed key management — v2 |
| T12 | Token replay / API key theft | Covered by existing API key rotation playbook; short-lived tokens (OAuth) deferred to v2 IdP integration |
| T13 | Inference via absence ("no results" reveals a classified topic exists) | Non-trivial to mitigate without fake results; deferred |

---

## 3. Classification model

### 3.1 Level ordinal

```
public     = 0  (unrestricted; default for single-operator stacks)
internal   = 1  (company-internal; default for new facts when AC enabled)
confidential = 2  (project-restricted; finance, HR, legal, security)
restricted = 3  (need-to-know only; board-level, active incidents)
```

The ordinal is stored as a `u8`. New levels cannot be inserted between existing ones without a migration (the ordinal mapping is fixed). Promoting a level (e.g. `internal → confidential`) requires a re-index.

### 3.2 Compartments (need-to-know labels)

Compartments are orthogonal to levels — a `confidential` fact may or may not carry `[financial]`. The canonical vocabulary:

| Compartment | Semantics |
|-------------|-----------|
| `financial` | Revenue, runway, cap table, board decks |
| `hr` | Salaries, performance reviews, org chart |
| `legal` | Litigation, contracts, IP |
| `security` | Incident post-mortems, CVEs, pen-test reports |
| `customer_pii` | PII attributable to specific customers |

The vocabulary is open: operators may define additional compartments in `cortex.toml`. Unknown compartment names are rejected at config-parse time to catch typos.

### 3.3 The lattice predicate

```rust
fn can_read(principal: &Principal, fact_level: u8, fact_compartments: &[String]) -> bool {
    principal.clearance_level >= fact_level
        && fact_compartments.iter().all(|c| principal.compartment_grants.contains(c))
}
```

This is the single source of truth. All four enforcement points call this function with the same arguments.

---

## 4. Phase18 pattern reuse map

Phase18 built two reusable patterns for the bitemporal axis: the **column-stamper** (stamps `valid_from`/`valid_to`/`as_of_ts` on every entity at ingestion) and the **retrieval-wedge** (a post-fusion step that drops hits outside the queried time window). The classification axis reuses both patterns directly:

| Phase18 component | Classification equivalent | Rust module |
|-------------------|--------------------------|-------------|
| `graph/bitemporal.rs::stamp_one_node` | `graph/classification.rs::stamp_one_node` | stamps `class_level`/`class_compartments` on `NodeOp` |
| `fulltext/bitemporal.rs::apply_bitemporal_projection` | `fulltext/builders.rs::apply_classification_projection` | stamps classification fields on `Document` |
| `embedder/chunker.rs::stamp_bitemporal` | `embedder/chunker.rs::stamp_classification` | stamps classification on `ChunkMetadata` |
| `search/orchestrator.rs::apply_temporal_classifier` | `search/orchestrator.rs::apply_acl_wedge` | post-fusion drop of unauthorized `LaneHit`s |
| Phase18 `as_of: Option<DateTime<Utc>>` on `QueryRequest` | `principal: Option<PrincipalRef>` on `QueryRequest` | optional, backward-compat |
| Meili `valid_from`/`valid_to` filterable fields | Meili `class_level` filterable+sortable + `class_compartments` filterable | Meili settings v7 → v8 |
| `ReindexAliasConstants` | `classification_reindex_alias.rs` constants | for the classification cut-over |

The structural template is identical. This reduces implementation risk significantly — the pattern is proven in production by phase18.

---

## 5. Design decisions summary

| ADR | Question | Answer |
|-----|----------|--------|
| ADR-028 | What classification model? | Bell-LaPadula: 4-level ordinal + orthogonal compartment set |
| ADR-029 | Where does the principal come from? | API-key → RBAC role binding; signed header for gateway scenarios; IdP deferred to v2 |
| ADR-030 | Default posture? | Feature OFF by default; deny-by-default when ON; `deny_on_missing_principal=true` |
| ADR-031 | Where to enforce? | Defense-in-depth: 4 enforcement points (backend filter primary); application-layer only |
| ADR-032 | How are classifications merged? | Declared floor + classifier escalation-only; explicit emitter override wins |
| ADR-033 | Who sets classifications and grants? | Operator via `cortex.toml` + `acl_admin` role; no-escalation invariant; audit trail mandatory |
| ADR-034 | How does classification relate to redaction? | Orthogonal and sequential: redact first, then classify; neither substitutes for the other |

---

## 6. Implementation phase map

| Phase | Description | Key deliverables |
|-------|-------------|-----------------|
| P0 (§1) | Design ADRs | ADR-028 through ADR-034 + this analysis |
| P1 (§2) | Schema + storage + migration | `class_level`/`class_compartments` on envelope + 3 backends; Meili v8; backfill CLI |
| P2 (§3) | Classification assignment | `[cortex.classification]` config; bootstrap stamper; classifier content detection; merge function |
| P3 (§4) | Principal model + resolution | `Principal` type; `PrincipalStore`; `ApiKeyStore` role binding; `QueryRequest.principal` |
| P4 (§5) | Enforcement in retrieval | ACL filter builder; 4 enforcement points wired; ITs per point |
| P5 (§6) | Public surfaces | Admin CLI; `/v1/acl/*`; principal-aware query; MCP tools |
| P6 (§7) | Config + default posture | `AccessControlConfig`; disabled pass-through; snapshot handle |
| P7 (§8) | Audit + observability | `access_decision` audit event; dashboard panels |
| P8 (§9) | Eval + leak-detection gate | Golden suite; adversarial leak probe; CI gate |

---

## 7. References

- ADR-028: [classification-model](../../../.rulebook/decisions/028-classification-model-bell-lapadula-lattice-with-linear-sensitivity-levels-and-orthogonal-need-to-know-compartments.md)
- ADR-029: [principal-and-identity-source](../../../.rulebook/decisions/029-principal-and-identity-source-api-key-bound-rbac-roles-for-v1-external-idp-deferred.md)
- ADR-030: [default-posture](../../../.rulebook/decisions/030-access-control-default-posture-feature-off-by-default-deny-by-default-when-enabled-mirrors-adr-020.md)
- ADR-031: [enforcement-strategy](../../../.rulebook/decisions/031-access-enforcement-strategy-defense-in-depth-with-backend-filter-primary-application-layer-only-no-backend-native-rls.md)
- ADR-032: [merge-semantics](../../../.rulebook/decisions/032-classification-assignment-merge-semantics-declared-floor-classifier-escalation-only-explicit-emitter-override-wins.md)
- ADR-033: [authority-model](../../../.rulebook/decisions/033-access-control-authority-model-operator-via-config-sets-classification-rules-acl-admin-role-grants-clearances-every-admin-action-is-audited.md)
- ADR-034: [classification-vs-redaction](../../../.rulebook/decisions/034-classification-vs-pii-redaction-redaction-removes-secrets-at-ingestion-classification-gates-visibility-per-principal-they-compose-sequentially-never-substitute.md)
- DEC-017 / phase15e: ingestion-time PII redaction (predecessor)
- Phase18 bitemporal specs (30/31): structural template for column-stamper + retrieval-wedge
- Bell-LaPadula model: D.E. Bell and L.J. LaPadula, "Secure Computer Systems: Mathematical Foundations", MITRE Technical Report MTR-2547 (1973)

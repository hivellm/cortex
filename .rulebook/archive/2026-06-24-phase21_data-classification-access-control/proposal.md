# Proposal: phase21_data-classification-access-control

## Why

Cortex today is a single-trust-domain store: any caller that can reach the
daemon can retrieve **every** indexed fact across every repo, decision,
consolidation, and turn. That is fine for a single maintainer on a local
stack, but it is a hard blocker for any **enterprise / multi-user**
deployment. In a company, knowledge is not flat — financial figures,
HR records, legal strategy, security findings, and customer PII must be
visible **only** to principals cleared for them. A junior engineer
querying "what's our runway?" must not have the CFO's board-deck
consolidation surface in their bundle; a contractor must not retrieve a
security incident post-mortem.

There is no classification axis on stored facts and no principal axis on
queries. The existing `AclStore` is a coarse repo/topic gate, not a
per-user least-privilege control, and identity stops at an opaque
`x-cortex-caller` header used only for audit. Without this, Cortex cannot
be deployed past a single trusted operator — every shared deployment is a
data-exfiltration surface.

This phase adds a **data-classification + access-control** plane:
every fact is labelled with a sensitivity classification at ingestion,
every query carries a principal (identity + clearance + compartments),
and retrieval enforces least-privilege at **every** lane and surface
(defense-in-depth), default-deny for classified data, with a full audit
trail of every access decision. The model mirrors the proven Bell-LaPadula
lattice (linear sensitivity level + orthogonal need-to-know compartments)
layered with RBAC role bindings, and it reuses the exact column-stamper +
retrieval-wedge machinery phase18 built for the bitemporal/branch axes.

## What Changes

A classification + authorization dimension lands across the data model and
every retrieval surface, in 9 phases:

- **P0 — Design ADRs.** Lock the open design questions: classification
  model (linear level + compartment lattice), principal/identity source,
  default posture (feature default OFF; deny-by-default for classified
  once ON), enforcement strategy (backend-filter-primary + fusion-wedge
  defense-in-depth), assignment merge semantics (declared floor +
  classifier escalation), authority model (who may set classifications and
  grants), and the relationship between classification and the existing
  PII redaction.
- **P1 — Schema, storage, migration.** Two columns on every retrievable
  entity: `class_level` (ordinal int: `public=0 < internal=1 <
  confidential=2 < restricted=3`) and `class_compartments` (string array,
  e.g. `["financial","hr"]`). Land them on the envelope + all three
  backends (Meili doc, Vectorizer payload, Nexus node) via a stamper that
  mirrors phase18 §2.1. Meili settings bump (filterable + sortable on
  `class_level` + filterable on `class_compartments`). Idempotent backfill
  imputing a configurable default level (`internal`) + empty compartments
  for pre-existing rows.
- **P2 — Classification assignment (ingestion).** A `[cortex.classification]`
  block in `cortex.toml` declares path/kind rules (`finance/** →
  confidential+financial`), applied by the bootstrap walker/emitter as the
  deterministic **floor**. The classifier worker adds content-based
  detection (financial / HR / legal / secret signals) that may only
  **escalate** (never downgrade). Merge: `level = max(declared, detected)`,
  `compartments = union`. Integrates with — but is distinct from — the PII
  redaction pass.
- **P3 — Principal model + resolution.** A `Principal { id, clearance_level,
  compartment_grants, roles }` type + a `PrincipalStore` with RBAC role
  bindings (role → clearance + compartments). Resolve the principal from
  the authenticated API key (extend `ApiKeyStore` with a role binding) or a
  signed caller-identity header. Thread an optional `principal` onto
  `QueryRequest` (Option-typed, backward-compatible like phase18's
  `as_of`/`branch`/`projects`).
- **P4 — Enforcement in retrieval (defense-in-depth).** The lattice check
  `clearance_level >= fact.level AND fact.compartments ⊆
  principal.compartment_grants` enforced at every layer: (a) per-lane
  backend filter (Meili filter clause, Vectorizer payload filter, Nexus
  `WHERE`) so unauthorized rows never leave the backend; (b) a post-fusion
  ACL drop-wedge mirroring `apply_temporal_classifier`; (c) the
  pre-thinking bundle assembler; (d) the raw `/v1/search/*` proxies.
- **P5 — Public surfaces (CLI / HTTP / MCP).** Admin CLI (`cortex-ops acl
  role|grant|classify-rule|whoami`), HTTP `/v1/acl/*` + principal-aware
  `/v1/query`, MCP admin tools + principal-aware `cortex_query`. `403`
  semantics when a classified scope is queried without a sufficient
  principal.
- **P6 — Config + default posture.** `AccessControlConfig { enabled
  (default false), default_level, deny_on_missing_principal, ... }` —
  default OFF so existing single-operator local stacks stay fully open
  (backward-compat, mirrors ADR-020's opt-in posture). When ON, classified
  facts are deny-by-default to principals that don't clear the lattice.
- **P7 — Audit + observability.** An `access_decision` audit envelope
  (principal, fact id, level, compartments, verdict, reason) on every
  grant/deny. Dashboard panels: denial rate, classification distribution,
  per-principal access volume. An **adversarial leak-detection** gate (a
  low-clearance principal must NEVER retrieve a restricted fact through any
  surface).
- **P8 — Specs + eval.** New specs (classification model, principal model,
  enforcement, admin API). A golden access-control eval suite in
  `cortex-eval` (principal X must / must-not see fact Y), wired as a CI
  gate — a leak is a hard CI failure.

## Impact

- **Affected specs:** new `docs/specs/40-classification-model.md`,
  `41-principal-and-rbac.md`, `42-access-enforcement.md`,
  `43-acl-admin-api.md`, `44-access-audit-and-eval.md`.
- **Affected code:** `crates/cortex-core/src/events.rs` (envelope fields),
  `crates/cortex-core/src/redact.rs` (classification-aware),
  `crates/cortex-workers/src/{graph/bitemporal.rs, fulltext/{document.rs,
  builders.rs}, embedder/chunker.rs, classifier/**}`,
  `crates/cortex-workers/settings/settings.v1.json` (filterable bump),
  `crates/cortex-cli/src/bootstrap/{walker.rs, emitter.rs, config.rs}`,
  `crates/cortex-cli/src/bin/cortex-ops/{acl.rs, migrate_classification.rs}`,
  `crates/cortex-api/src/{types.rs, search/orchestrator.rs, lanes/*.rs,
  acl.rs, storage/api_keys.rs, http.rs, mcp.rs}`,
  `crates/cortex-pre-thinking/src/bundle.rs`,
  `crates/cortex-config/src/{sub.rs, config.rs, env_map.rs}`,
  `crates/cortex-mcp-server/src/tools.rs`, `crates/cortex-eval/src/suite/`.
- **Breaking change:** NO. Schema migrations are additive; the feature
  defaults OFF (`access_control.enabled = false`) so existing deployments
  behave identically; `QueryRequest.principal` is an optional field that
  omitted callers round-trip unchanged.
- **User benefit:** Cortex becomes deployable in a multi-user company —
  financial / HR / legal / security knowledge is retrievable only by
  cleared principals; every access is audited; a misconfigured query
  fails closed (deny) rather than leaking. Unlocks the enterprise
  deployment path that single-trust-domain Cortex structurally blocks
  today.

## Source

`docs/analysis/data-classification-access-control/` (to be authored in P0
alongside the ADRs). Reuses the phase18 bitemporal stamper + retrieval-wedge
patterns (specs 30/31) as the structural template for the classification
axis. Cross-references the existing PII redaction (DEC-017 /
phase15e_ingestion-redaction-policy).

## Dependencies

- Soft: phase18 P1/P2 (bitemporal columns + temporal wedge) — the
  classification column-stamper and the ACL drop-wedge reuse that exact
  machinery; landing on top of it keeps one stamper/wedge pattern.
- Soft: a working principal source. P3 ships an API-key-bound RBAC default;
  external IdP / JWT (OIDC) integration is explicitly out of scope for v1
  and noted as a follow-up.
- Backend note: enforcement is **primary at the application layer** (the
  daemon filters before returning). Backend-native row-level security in
  Meili / Vectorizer / Nexus is NOT assumed — the per-lane filters are
  query-builder clauses the daemon constructs, so this phase does not block
  on any Synap/Vectorizer/Nexus feature work.

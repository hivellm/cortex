## 1. P0 — Design ADRs (lock open questions)
- [ ] 1.1 ADR — Classification model: linear sensitivity level (`public=0 < internal=1 < confidential=2 < restricted=3`) + orthogonal need-to-know compartments (string set). Bell-LaPadula "no read up + need-to-know". Document the ordinal mapping + the canonical compartment vocabulary (`financial`, `hr`, `legal`, `security`, `customer_pii`) as an open, config-extensible set.
- [ ] 1.2 ADR — Principal & identity source: v1 resolves the principal from the authenticated API key (RBAC role binding on `ApiKeyStore`) OR a signed caller-identity header; external IdP / OIDC-JWT is out of scope for v1. Document the trust boundary (who mints principals) + the unauthenticated-caller fallback.
- [ ] 1.3 ADR — Default posture: feature default OFF (`access_control.enabled = false`) for backward compat; when ON, classified facts are **deny-by-default** to principals that fail the lattice, and `deny_on_missing_principal` controls whether an un-authenticated caller sees only `public` or is rejected. Mirrors ADR-020 opt-in.
- [ ] 1.4 ADR — Enforcement strategy: defense-in-depth, backend-filter-primary. The per-lane backend filter is the load-bearing control (sensitive rows never leave the backend); the post-fusion wedge + bundle filter + surface 403 are redundant belts. Application-layer enforcement only — no dependency on backend-native RLS.
- [ ] 1.5 ADR — Assignment merge semantics: declared (config path/kind rules) is the **floor**; the classifier worker may only **escalate** level + **union** compartments, never downgrade. Explicit envelope `class_*` from a trusted emitter wins as an upper override. Document the precedence chain.
- [ ] 1.6 ADR — Authority model: who may set classification rules (operator via `cortex.toml` + admin API) and who may grant clearances/compartments (an `acl_admin` role). A principal can never grant itself a clearance it lacks. Document the admin-action audit requirement.
- [ ] 1.7 ADR — Classification vs PII redaction: redaction (DEC-017) removes secrets from the payload at ingestion (irreversible); classification gates *visibility* of the (already redacted) fact per principal. They compose: redaction first, then classification stamping. Document the boundary so neither is mistaken for the other.
- [ ] 1.8 Author `docs/analysis/data-classification-access-control/` (README + findings + design + references) capturing the model, the threat model (what attacker / mis-query each control stops), and the phase18-pattern reuse map.

## 2. P1 — Schema, storage, migration
- [ ] 2.1 Add `class_level: Option<u8>` + `class_compartments: Option<Vec<String>>` to the `Envelope`/`EnrichedEvent` (`crates/cortex-core/src/events.rs`), both Option-typed with serde default + omit-when-none so omitted callers round-trip unchanged. Unit tests for serde round-trip + default.
- [ ] 2.2 Classification stamper `crates/cortex-workers/src/graph/classification.rs` (mirror `graph/bitemporal.rs::stamp_one_node`): stamp `class_level` + `class_compartments` on every `NodeOp`, idempotent `entry().or_insert()`, default level from config, never overwrite an emitter-set value. Unit tests.
- [ ] 2.3 Meili projection: extend `Document` (`crates/cortex-workers/src/fulltext/document.rs`) with `class_level` + `class_compartments` (Option, omit-when-none) + `apply_classification_projection` in `builders.rs` (runs after bitemporal projection). Unit test.
- [ ] 2.4 Vectorizer projection: extend `ChunkMetadata` (`crates/cortex-workers/src/embedder/chunker.rs`) with the two fields + `stamp_classification(event)` mirroring `stamp_bitemporal`; wire all chunker emit sites. Unit test.
- [ ] 2.5 Meili settings bump `v7 → v8` (`crates/cortex-workers/settings/settings.v1.json`): `class_level` filterable + sortable; `class_compartments` filterable. Pin test for the new attrs.
- [ ] 2.6 Versioned reindex alias for the classification cut-over (`graph/reindex_alias.rs` — add `*-classification-v1` constants) + invariant tests, mirroring phase18 §2.8.
- [ ] 2.7 Migration scaffolding `crates/cortex-workers/src/graph/classification_migration.rs` (per-project report, default-level imputation, anomaly carrier) + thin CLI wrapper `crates/cortex-cli/src/bin/cortex-ops/migrate_classification.rs` (`--root --project --default-level --dry-run --json`). Backfill imputes `class_level = <default>` + empty compartments on rows lacking the columns. Unit tests + dry-run.
- [ ] 2.8 Spec `docs/specs/40-classification-model.md` (column set, level ordinal, compartment vocab, stamper rules, backfill, reindex, migration shape, pinned tests).

## 3. P2 — Classification assignment (ingestion)
- [ ] 3.1 `[cortex.classification]` config block in `crates/cortex-cli/src/bootstrap/config.rs` (mirror `[cortex.laws]` promote_patterns): `rules = [{ pattern, level, compartments }]` (glob → level + compartments). Parse + validate (reject unknown level names). Unit tests.
- [ ] 3.2 Walker/emitter integration (`bootstrap/walker.rs` + `emitter.rs`): stamp the declared `class_level`/`class_compartments` floor on `WalkEntry` → carried onto the synthetic event. Path-rule match is longest-prefix-wins + most-restrictive-wins on ties. Unit tests.
- [ ] 3.3 Classifier-worker content detection (`crates/cortex-workers/src/classifier/**`): add `sensitivity` output (level + compartments) from content signals (financial/HR/legal/security keyword + structure heuristics; reuse the existing PII-risk + severity signals). Escalate-only. Unit tests over labelled fixtures.
- [ ] 3.4 Merge semantics in the enrichment path: `level = max(declared, detected, explicit)`, `compartments = union`; explicit-from-trusted-emitter is the upper override. Single merge function + exhaustive unit tests over the precedence chain.
- [ ] 3.5 Redaction integration (`crates/cortex-core/src/redact.rs`): document + test that redaction runs BEFORE classification stamping; a classification-bearing rule may also flag a redaction (e.g. raw SSN → `customer_pii` compartment + redact). No downgrade of either.
- [ ] 3.6 Spec section in `docs/specs/40-...` covering the assignment pipeline + merge precedence.

## 4. P3 — Principal model + resolution
- [ ] 4.1 `Principal { id, clearance_level, compartment_grants, roles }` type (`crates/cortex-api/src/acl.rs` or a new `principal.rs`) + the lattice predicate `can_read(principal, level, compartments) -> bool`. Pure, exhaustively unit-tested (clears/denies on level + each compartment-subset case + empty-grant + super-admin).
- [ ] 4.2 `PrincipalStore` + RBAC role bindings (role → clearance + compartments); resolve a `Principal` from a role set. Backed by the existing storage layer; admin-mutable. Unit tests.
- [ ] 4.3 Extend `ApiKeyStore` (`crates/cortex-api/src/storage/api_keys.rs`) with a role binding per key; resolve key → principal. Backward-compat: keys without a binding resolve to the configured default principal. Unit tests.
- [ ] 4.4 Caller-identity resolution: map the authenticated request (API key, or signed `x-cortex-principal` header when configured) → `Principal`; thread it into `QueryService`/orchestrator. Unauthenticated → default principal per ADR-1.3. Unit + IT.
- [ ] 4.5 Thread `principal: Option<PrincipalRef>` onto `QueryRequest` (`crates/cortex-api/src/types.rs`), Option-typed + omit-when-none (mirror phase18 `as_of`/`branch`/`projects`); patch every `QueryRequest { .. }` literal site to `None`. Compile + existing-tests green.
- [ ] 4.6 Spec `docs/specs/41-principal-and-rbac.md`.

## 5. P4 — Enforcement in retrieval (defense-in-depth)
- [ ] 5.1 ACL filter builder `crates/cortex-workers/src/acl/filter.rs`: render the per-lane clause from a principal — Meili (`class_level <= N AND (class_compartments IS EMPTY OR class_compartments IN [...])`), Vectorizer payload filter, Nexus `WHERE`. Pure renderers + unit tests (incl. compartment-subset semantics + escape).
- [ ] 5.2 Meili lane wiring (`crates/cortex-api/src/lanes/meili_lane.rs` + `meili_loader.rs`): inject the ACL clause into every keyword request when AC is enabled. IT against seeded docs at mixed levels.
- [ ] 5.3 Vectorizer lane wiring (`vectorizer_lane.rs`): apply the payload ACL pre-filter. IT.
- [ ] 5.4 Nexus lane wiring (`nexus_graph_lane.rs`): add the ACL `WHERE` to each whitelisted template (inline-literal per the Nexus 2.2.0 param-binding workaround, nexus#3). IT.
- [ ] 5.5 Post-fusion ACL drop-wedge in `crates/cortex-api/src/search/orchestrator.rs` (mirror `apply_temporal_classifier`): after the temporal wedge, drop any hit whose `class_*` (read from `LaneHit::extras`) fails `can_read(principal, ..)`; extend `LANE_EXTRAS_KEYS` with `class_level`/`class_compartments`. Pinned IT (drop/keep across the lattice).
- [ ] 5.6 Pre-thinking bundle filter (`crates/cortex-pre-thinking/src/bundle.rs`): apply `can_read` before any section (laws/decisions/snippets/similar) enters the bundle, so sensitive facts never reach the assembled prompt. IT.
- [ ] 5.7 Raw search proxies (`crates/cortex-api/src/search_proxy.rs` `/v1/search/{keyword,vector,graph}`): apply the principal filter (these bypass the orchestrator). IT — a raw proxy MUST NOT leak a classified row.
- [ ] 5.8 Spec `docs/specs/42-access-enforcement.md` (the lattice predicate, the four enforcement points, the defense-in-depth rationale, pinned tests).

## 6. P5 — Public surfaces (CLI / HTTP / MCP)
- [ ] 6.1 Admin CLI `crates/cortex-cli/src/bin/cortex-ops/acl.rs`: `cortex-ops acl role create|list`, `acl grant <principal> --role|--clearance|--compartments`, `acl classify-rule list`, `acl whoami` (resolve + print the caller's effective clearance/compartments). Unit tests for arg parsing + the `whoami` lattice render.
- [ ] 6.2 HTTP `/v1/acl/*` (`crates/cortex-api/src/http.rs` + new `acl_routes.rs`): role CRUD, principal grant, whoami; all mutations gated by the `acl_admin` role; `403` envelope shape. IT.
- [ ] 6.3 Principal-aware `/v1/query` + raw search routes: accept the resolved principal; return `403 forbidden_classified` when a classified scope is queried without sufficient clearance per `deny_on_missing_principal`. IT.
- [ ] 6.4 MCP: principal-aware `cortex_query` (extend schema in `crates/cortex-api/src/mcp.rs`) + admin tools `cortex_acl_whoami` / `cortex_acl_grant` in `crates/cortex-mcp-server/src/tools.rs`; bump the tool-count assertions. The MCP caller's principal derives from its API key. mcp-server tests green + the §4.8-style schema gate stays green.
- [ ] 6.5 Spec `docs/specs/43-acl-admin-api.md`.

## 7. P6 — Config + default posture
- [ ] 7.1 `AccessControlConfig` in `crates/cortex-config/src/sub.rs`: `enabled` (default false), `default_level` (default `internal`), `deny_on_missing_principal` (default true when enabled), `default_compartments` (default empty). Wire into top-level `Config` + `lib.rs` export; register `CORTEX_ACCESS_CONTROL_*` env vars ASCII-sorted in `env_map.rs`. Unit tests (defaults, TOML round-trip, partial-TOML fallback).
- [ ] 7.2 Orchestrator + bundle + proxies read the AC snapshot via an `Arc<RwLock<AccessControlConfig>>` handle (mirror the temporal/cross_project handles); when `enabled=false` every enforcement point is a no-op pass-through (backward-compat). Pinned IT: disabled ⇒ a low-clearance principal sees everything.
- [ ] 7.3 Spec section in `docs/specs/42-...` documenting the config knobs + the disabled-pass-through guarantee.

## 8. P7 — Audit + observability
- [ ] 8.1 `access_decision` audit envelope on the `cortex_audit` target from every enforcement point (principal id, fact/doc id, fact level + compartments, verdict grant|deny, reason, query_id) + a per-request `access_decision_summary` (evaluated / granted / denied counts). Pinned IT with a recording subscriber (mirror `temporal_audit_it.rs`).
- [ ] 8.2 Dashboard panels + GUI surface: denial rate over time, classification distribution per repo, top denied principals. Reads the audit store. GUI view + vitest (happy/empty/error).
- [ ] 8.3 Spec `docs/specs/44-access-audit-and-eval.md` (audit envelope shapes + dashboard contract).

## 9. P8 — Eval + leak-detection gate
- [ ] 9.1 Golden access-control suite in `crates/cortex-eval/src/suite/access_control.rs` + `tests/golden/access_control.csv` (rows: principal clearance/compartments × fact level/compartments × expected visible|hidden). Metric: zero false-grants (a leak) + correct grants recall. Acceptance: **false-grant count MUST be 0**.
- [ ] 9.2 Adversarial leak probe: a low-clearance principal runs every surface (query / raw proxies / pre-thinking / MCP tools) against a corpus seeded with `restricted` facts and MUST retrieve none. Wired as a hard CI gate (a single leak fails CI).
- [ ] 9.3 Spec section in `docs/specs/44-...` documenting the eval gates + the zero-leak CI gate.

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation — specs 40-44 + the 7 P0 ADRs + the `docs/analysis/data-classification-access-control/` analysis + inline doc comments at every code site.
- [ ] 99.2 Write tests covering the new behavior — unit tests per stamper/predicate/filter + ITs per enforcement point + the P8 golden access-control suite + the adversarial leak probe.
- [ ] 99.3 Run tests and confirm they pass — `cargo check --workspace` clean, `cargo clippy --workspace -- -D warnings` clean, all unit + IT green, the access-control eval suite reports **zero false-grants**.

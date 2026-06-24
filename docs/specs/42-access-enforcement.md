# Spec 42 — Access Enforcement

Phase21 §5 — defense-in-depth ACL filtering across every retrieval surface.

Related: [Spec 40 — Classification Model](40-classification-model.md),
[Spec 41 — Principal & RBAC](41-principal-and-rbac.md)

---

## 1. Design principle — defense-in-depth

Sensitive rows MUST be filtered at every layer of the retrieval stack.
No single enforcement point is authoritative in isolation; each layer is
independently load-bearing so that a bug in one does not expose classified
data through another.

```
Request → [§5.7 raw proxy ACL]
         → [§5.2 Meili filter]
              → [§5.3 Vectorizer post-filter]
                   → [§5.4 Nexus WHERE]
                        → [§5.5 post-fusion wedge]
                             → [§5.6 bundle filter]
                                  → Response
```

Each layer reduces the working set; the post-fusion wedge (§5.5) is the
final authoritative gate before the result reaches the caller.  Backend
filters (Meili, Nexus) use an OR-ANY approximation for compartments to
minimise rows sent over the wire; the wedge re-applies the exact predicate.

---

## 2. The `can_read` lattice predicate

See [Spec 41 §2](41-principal-and-rbac.md#2-bell-lapadula-can_read-predicate)
for the full definition.  Summary:

```
can_read(principal, fact_level, fact_compartments) -> bool
  = (principal.clearance_level >= fact_level)
    AND (acl_admin OR every fact_compartment in principal.compartment_grants)
```

`acl_admin` bypasses compartment checks but never the level gate.

---

## 3. Enforcement point — §5.2 Meili keyword lane

**File**: `crates/cortex-api/src/lanes/meili_lane.rs` +
`crates/cortex-workers/src/acl/filter.rs::meili_acl_filter`

Filter injected into every Meili search body when an `AclContext` is present:

```
(class_level IS EMPTY OR class_level <= N)
AND (class_compartments IS EMPTY OR class_compartments IN ['c1', 'c2', ...])
```

Compartment clause uses OR-ANY (see §1 note).  `acl_admin` principals
receive only the level clause (no compartment restriction).

**What this stops**: prevents the Meili index from returning classified
documents to network-level eavesdroppers or callers who bypass the API
and query Meili directly.

**Test**: `tests/meili_keyword_lane.rs` — seeded docs at mixed levels;
IT asserts low-clearance callers receive only eligible hits.

---

## 4. Enforcement point — §5.3 Vectorizer lane

**File**: `crates/cortex-api/src/lanes/vectorizer_lane.rs::acl_matches`

Vectorizer has no server-side filter parameter.  The lane applies the
exact `vectorizer_acl_matches` predicate client-side after receiving hits:

```rust
cortex_workers::acl::filter::vectorizer_acl_matches(
    clearance_level,
    compartment_grants,
    hit.extras["class_level"],
    hit.extras["class_compartments"],
)
```

`class_level` and `class_compartments` are projected from `hit.payload`
(canonical ≥3.0.0) with fallback to `hit.payload.payload` for legacy builds.

**What this stops**: prevents classified vector-search results from
reaching the fusion layer even if a Vectorizer backend bug returns them.

**Test**: `tests/vectorizer_lane.rs` — `acl_filter_drops_hit_above_principal_clearance`,
`acl_filter_drops_hit_missing_required_compartment`, `no_acl_context_passes_all_hits_through`.

---

## 5. Enforcement point — §5.4 Nexus graph lane

**File**: `crates/cortex-api/src/lanes/nexus_graph_lane.rs`

`nexus_acl_where_for_alias` appended to the Cypher `WHERE` clause of every
whitelisted template via inline literals (nexus#3 workaround — Nexus 2.2.0
does not execute bound parameters at query time):

```
(n.class_level IS NULL OR n.class_level <= N)
AND (n.class_compartments IS NULL OR SIZE(n.class_compartments) = 0
     OR ANY(c IN n.class_compartments WHERE c IN ["c1", "c2"]))
```

**What this stops**: prevents confidential graph relationships from
leaking via the KG traversal path.

**Test**: `tests/nexus_graph_lane.rs` — ACL WHERE injected for each
whitelisted template; `acl_admin` receives the clause without compartment
restriction.

---

## 6. Enforcement point — §5.5 Post-fusion wedge

**File**: `crates/cortex-api/src/search/orchestrator.rs`

After RRF fusion and temporal classification, any hit whose `class_*`
extras fail `can_read(principal, ..)` is dropped before the result bag
reaches the caller.  This is the final and authoritative gate.

`can_read` is the exact predicate (no OR-ANY approximation); the backend
filters (§§5.2–5.4) may let through hits that fail this check in edge
cases — the wedge catches them.

**What this stops**: a backend that returns a row that squeaked past its
approximate filter; a fusion step that joins results from multiple lanes
without re-checking.

**Test**: `tests/acl_wedge_it.rs` — 3 tests: drops above clearance,
drops missing compartment, no-ACL passes all.

---

## 7. Enforcement point — §5.6 Pre-thinking bundle filter

**File**: `crates/cortex-pre-thinking/src/bundle.rs`

Before assembling the prompt bundle (laws + decisions + snippets + similar),
every section is checked with `can_read`.  Classified sections are silently
omitted from the bundle so they never reach the LLM context.

`principal` is threaded from the HTTP handler → `PreThinkingInput` → bundle.
When no principal is set (`None`), all sections pass — backward compat.

**What this stops**: even if a classified snippet somehow reached the
results bag (wedge missed), the bundle filter ensures the LLM never sees it.

**Test**: `crates/cortex-pre-thinking/tests/bundle_acl_it.rs` — 3 tests:
drops above clearance, drops missing compartment, no principal passes all.

---

## 8. Enforcement point — §5.7 Raw search proxies

**File**: `crates/cortex-api/src/search/search_proxy.rs`

The three `/v1/search/{keyword,vector,graph}` handlers bypass the
fusion orchestrator.  Each applies its own enforcement:

### 8.1 Keyword proxy (`POST /v1/search/keyword`)

Injects the ACL filter into the Meili request body using
`merged_meili_filter(caller_filter, acl_filter)`:

```
(caller_filter) AND (acl_filter)
```

When `caller_filter` is absent, only the ACL clause is set.
`acl_admin` principals forward the caller filter verbatim (no ACL clause).

### 8.2 Vector proxy (`POST /v1/search/vector`)

Client-side post-filter after receiving hits from Vectorizer, using
`vector_hit_passes_acl` — reads `hit["payload"]["class_level"]` and
`hit["payload"]["class_compartments"]` (canonical path; falls back to
`hit["payload"]["payload"][*]` for legacy builds).  Drops any hit that
fails the exact Bell-LaPadula predicate.

### 8.3 Graph proxy — Neighbors mode (`POST /v1/search/graph` + `mode=neighbors`)

Injects `WHERE {nexus_acl_where_for_alias("n")} ` before `RETURN` in the
generated Cypher.  The WHERE fragment is built from the resolved principal's
clearance level and compartment grants.

### 8.4 Graph proxy — Cypher mode (`POST /v1/search/graph` + `mode=cypher`)

**Blocked for non-`acl_admin` principals** — returns `403 acl_required`.

Raw Cypher can traverse arbitrary graph paths; injecting a WHERE clause
into an unknown user-supplied statement is not safe.  Operators that need
Cypher must hold `acl_admin` clearance.

Gate fires before the nexus-configured check so the response is always
`403`, never `503`.

### Principal resolution (all three handlers)

```rust
let principal = state.service.resolve_principal(&headers);
// super_admin("system") when principal_store: None → no ACL applied
```

`is_acl_admin()` = `true` → skip filter injection (pass-through).

**What this stops**: MCP callers or external tools that bypass the
`/v1/query` fusion path must still satisfy the full ACL lattice.

**Test**: `crates/cortex-api/tests/raw_proxy_acl_it.rs` — 3 tests:
- `keyword_proxy_injects_acl_filter_for_non_admin` — asserts `class_level <= 0` in Meili body
- `vector_proxy_drops_classified_hits_for_non_admin` — restricted hit absent, public hit present
- `graph_proxy_cypher_blocked_for_non_admin` — returns `403 acl_required`

---

## 9. Invariants

1. **No enforcement point is optional.** The five layers from §§3–8 MUST all
   be active when `access_control.enabled = true`.  Disabling any one is a
   security regression.

2. **Backend filters use OR-ANY; wedge uses exact predicate.** This is
   intentional: OR-ANY is cheaper and reduces network load; the wedge
   provides correctness.  A false-positive from the backend (too permissive)
   is always caught by the wedge.  A false-negative (too restrictive) causes
   a recall loss, not a security breach.

3. **`acl_admin` bypasses compartments, never levels.** The Bell-LaPadula
   no-read-up rule is absolute — no role escalates clearance level.

4. **`principal_store: None` = super_admin pass-through.** Existing
   deployments that have not wired AC are unrestricted.  `pass_through()`
   produces the same behaviour and is the intended backward-compat migration
   path.

5. **Raw proxies MUST NOT leak.** A classified row MUST NOT appear in the
   response of any raw proxy handler when the principal's clearance is
   insufficient.  This is verified by the adversarial leak probe in §9.2.

---

## 10. Config knobs + disabled-pass-through guarantee (Phase21 §7)

### 10.1 `[access_control]` config block

| TOML field | Env var | Default | Description |
|---|---|---|---|
| `enabled` | `CORTEX_ACCESS_CONTROL_ENABLED` | `false` | Master switch. When `false` every enforcement point is a **no-op pass-through** (see §10.2). |
| `default_level` | `CORTEX_ACCESS_CONTROL_DEFAULT_LEVEL` | `1` (`internal`) | Sensitivity level assigned to facts that carry no explicit `class_level`. |
| `deny_on_missing_principal` | `CORTEX_ACCESS_CONTROL_DENY_ON_MISSING_PRINCIPAL` | `true` | When `enabled = true` and the caller supplies no Bearer token / API key, return `403 forbidden_classified` rather than downgrading to the default clearance. |
| `default_compartments` | `CORTEX_ACCESS_CONTROL_DEFAULT_COMPARTMENTS` | `[]` | Compartment grants assigned to principals that resolve to the default binding. |

Type: `crates/cortex-config/src/sub.rs::AccessControlConfig`.
Wired into `Config` at `access_control`, exported from `cortex_config`.
Env vars are registered in `env_map.rs` (ASCII-sorted between `CORTEX_ACCESS_CONTROL_*` and `CORTEX_ADAPTER_*`).

### 10.2 Disabled-pass-through guarantee

**When `access_control.enabled = false` (the default), every enforcement point MUST be a silent no-op regardless of principal clearance or compartment grants.**

This guarantee exists to ensure deployments that have not opted into AC (all deployments before phase21) continue to work byte-for-byte with no behaviour change.

Enforcement points and their disabled behaviour:

| Point | File | Disabled behaviour |
|---|---|---|
| `run_with_principal` | `orchestrator.rs` | Early-returns `self.run(req)` — no lattice check, no ACL wedge. |
| Post-fusion ACL wedge (`apply_acl_wedge`) | `orchestrator.rs` | Never reached when `enabled = false` (no `AclContext` is passed to `run_with_acl`). |
| Deny-on-missing-principal gate | `http.rs`, `search_proxy.rs` | First condition `state.cfg.access_control.enabled` is `false` → short-circuits, gate skipped. |
| Bundle filter | `cortex-pre-thinking/src/bundle.rs` | Caller threads `principal = None` when AC is off → `filter_snippets_by_acl(_, None)` is a no-op. |
| Per-lane filters (Meili, Vectorizer, Nexus) | `meili_lane.rs`, `vectorizer_lane.rs`, `nexus_graph_lane.rs` | `AclContext` is not injected by the orchestrator when AC is off → lanes receive `acl: None` → no filter clause. |

**SHALL requirement:**

The system SHALL allow a principal with clearance_level=0 and no compartment grants to receive all retrieval results (including `restricted` level=3 and compartment-gated facts) when `access_control.enabled = false`.

#### Scenario: AC disabled — low-clearance principal sees everything

```
Given access_control.enabled = false (default)
  And a corpus seeded with hits at levels 0, 1, 2, 3 and various compartments
  And a principal with clearance_level = 0 and no compartment_grants
When run_with_principal is called
Then all 4 hits must appear in the response
  And no hit must be dropped by the ACL wedge
```

**Pinned test**: `crates/cortex-api/tests/ac_disabled_passthrough_it.rs::disabled_ac_passes_all_hits_to_low_clearance_principal`

#### Scenario: AC enabled — same principal sees only eligible hits

```
Given access_control.enabled = true
  And a corpus seeded with hits at levels 0, 1, 2, 3
  And a principal with clearance_level = 1 and no compartment_grants
When run_with_principal is called
Then only level=0 and level=1 hits must appear (2 hits)
  And level=2 and level=3 hits must be dropped
```

**Pinned test**: `crates/cortex-api/tests/ac_disabled_passthrough_it.rs::enabling_ac_restores_filtering_for_low_clearance_principal`

---

## 11. Filter renderer reference

`crates/cortex-workers/src/acl/filter.rs`

| Function | Output | Surface |
|---|---|---|
| `meili_acl_filter(level, grants)` | Meili filter grammar string | §§5.2, 8.1 |
| `vectorizer_acl_matches(level, grants, fact_level, fact_comps)` | `bool` | §§5.3, 8.2 |
| `nexus_acl_where(level, grants)` | Cypher WHERE fragment (alias `n`) | §5.4 |
| `nexus_acl_where_for_alias(level, grants, alias)` | Cypher WHERE fragment (any alias) | §§5.4, 8.3 |

Compartment strings in Meili output are single-quoted and escaped (`\'`, `\\`).
Compartment strings in Nexus output are double-quoted and escaped (`\"`, `\\`).

---

## 11. Pinned tests

| File | Count | What |
|---|---|---|
| `crates/cortex-workers/src/acl/filter.rs` (unit) | 17 | All renderer variants + escaping |
| `crates/cortex-api/tests/acl_wedge_it.rs` | 3 | Post-fusion wedge |
| `crates/cortex-api/tests/ac_disabled_passthrough_it.rs` | 2 | §7.2 disabled pass-through + re-enable restores filtering |
| `crates/cortex-api/src/search/orchestrator.rs` (unit) | 3 | `AccessControlConfig` handle methods |
| `crates/cortex-api/tests/raw_proxy_acl_it.rs` | 3 | Raw proxy enforcement |
| `crates/cortex-api/tests/meili_keyword_lane.rs` | subset | Meili lane ACL |
| `crates/cortex-api/tests/vectorizer_lane.rs` | 3 | Vectorizer lane ACL |
| `crates/cortex-api/tests/nexus_graph_lane.rs` | subset | Nexus lane ACL |
| `crates/cortex-pre-thinking/tests/bundle_acl_it.rs` | 3 | Bundle filter |

All tests MUST remain green.  `cargo clippy --workspace -- -D warnings` MUST
pass with zero warnings.

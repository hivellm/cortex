# Spec 41 — Principal Model & RBAC

Phase21 §4 — caller-identity resolution, role-based clearance bindings, and the
Bell-LaPadula `can_read` lattice predicate.

Related: [Spec 40 — Classification Model](40-classification-model.md),
[Spec 42 — Access Enforcement](42-access-enforcement.md)

---

## 1. Principal type

`crates/cortex-api/src/security/principal.rs`

```rust
pub struct Principal {
    pub id: String,
    pub clearance_level: u8,
    pub compartment_grants: Vec<String>,
    pub roles: Vec<String>,
}
```

| Constructor | clearance_level | compartment_grants | Use |
|---|---|---|---|
| `public_only(id)` | 0 | `[]` | Unauthenticated / anonymous callers |
| `super_admin(id)` | 255 | `["*"]` | System pass-through (no AC store configured) |

`is_acl_admin(&self) -> bool` — true when `roles` contains `"acl_admin"`.

### `PrincipalRef`

`pub type PrincipalRef = String` — the `principal_id` field threaded through
`QueryRequest` and audit envelopes.  Decouples the serialised wire string from the
in-memory struct.

---

## 2. Bell-LaPadula `can_read` predicate

```
can_read(principal, fact_level, fact_compartments) -> bool
```

### Rules (in priority order)

1. **No-read-up** (absolute): `principal.clearance_level >= fact_level` MUST hold.
   If this fails, access is denied regardless of roles or compartments.

2. **acl_admin bypass** (compartment only): when `principal.is_acl_admin()`,
   the compartment check is skipped.  The level check (rule 1) still applies —
   `acl_admin` is a compartment-management role, not a clearance escalation.

3. **Need-to-know**: every compartment in `fact_compartments` MUST appear in
   `principal.compartment_grants`.  Empty `fact_compartments` → this check passes.

4. **Wildcard grant**: `"*"` in `compartment_grants` satisfies any compartment set
   (used by `super_admin`).

### Decision table

| clearance vs level | compartments satisfied | acl_admin | verdict |
|---|---|---|---|
| `<` | any | any | **deny** |
| `>=` | yes | any | **grant** |
| `>=` | no | no | **deny** |
| `>=` | no | yes | **grant** (compartment bypass) |

---

## 3. PrincipalStore & RBAC role bindings

`crates/cortex-api/src/security/principal_store.rs`

### RoleBinding

```rust
pub struct RoleBinding {
    pub clearance_level: u8,
    #[serde(default)]
    pub compartments: Vec<String>,
}
```

Serialisable/deserialisable — loaded from `cortex.toml` or an admin API payload.
A role name maps 1:1 to one `RoleBinding`.

### PrincipalStore

| Method | Behaviour |
|---|---|
| `new()` | Default principal = `public_only("default")` |
| `deny_by_default()` | Default principal = `public_only("anonymous")` (same; explicit name) |
| `pass_through()` | Default principal = `super_admin("system")` — effectively disables AC |
| `insert(role, binding)` | Register / overwrite a role binding |
| `resolve(principal_id, roles)` | Return a resolved `Principal` |

### `resolve` semantics (escalate-only merge)

1. For each role in `roles`, look up its `RoleBinding`.
2. Merge matched bindings: `clearance_level = max(...)`, `compartments = union(...)`.
3. If no role matches, return `default_principal` with `id` set to `principal_id`.

Callers never have their clearance *downgraded* by role resolution — they receive
the maximum of all their bound roles.

---

## 4. ApiKeyStore role extension

`crates/cortex-api/src/storage/api_keys.rs`

An optional `role TEXT` column is added to the `api_keys` table via a
backward-compatible `ALTER TABLE ADD COLUMN IF NOT EXISTS` migration applied at
startup.  Keys created before this migration have `role = NULL` and resolve to the
default principal.

| Method | Signature | Notes |
|---|---|---|
| `issue_with_role` | `(scope, label, role: Option<&str>) -> Result<IssuedKey>` | Primary issue path |
| `issue` | `(scope, label) -> Result<IssuedKey>` | Delegates to `issue_with_role(..., None)` |
| `assign_role` | `(id: &str, role: Option<&str>) -> Result<()>` | Admin-mutable post-issuance |
| `verify_with_role` | `(candidate: &str) -> Result<(String, Option<String>)>` | Returns `(key_id, role)` |
| `verify` | `(candidate: &str) -> Result<String>` | Delegates to `verify_with_role` (backward compat) |

---

## 5. Caller-identity resolution

`crates/cortex-api/src/security/principal_resolver.rs`

```
resolve_request_principal(headers, api_key_store, principal_store) -> Principal
```

Resolution order (first match wins):

1. **`x-cortex-principal` header** — honoured only when
   `CORTEX_TRUST_PRINCIPAL_HEADER=1` is set.  The header value is treated as the
   principal id; roles are resolved from `principal_store` using that id as the
   role label (no Bearer verification).  **This mode is restricted to internal,
   trusted deployment topologies.**

2. **Bearer token** — `Authorization: Bearer <token>` → `api_key_store.verify_with_role()`
   → returns `(key_id, Option<role>)` → `principal_store.resolve(key_id, roles)`.

3. **Default principal** — `principal_store.default_principal` with `id = "anonymous"`.

### `QueryService` wiring

`QueryService` carries:

```rust
pub principal_store: Option<Arc<RwLock<PrincipalStore>>>,
pub api_key_store:   Option<Arc<ApiKeyStore>>,
```

When `principal_store` is `None`, `resolve_principal(headers)` returns
`Principal::super_admin("system")` — existing callers that have not wired AC are
unrestricted (backward-compatible pass-through).

### `QueryRequest.principal`

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub principal: Option<String>,
```

Carries the resolved `PrincipalRef` (principal id) through the orchestrator pipeline
to audit envelopes and enforcement points.  Mirrors `as_of` / `branch` / `projects`
from phase18.

---

## 6. Pinned tests

| File | Tests |
|---|---|
| `security/principal.rs` | 13 — level gate, compartment gate, acl_admin bypass, super_admin |
| `security/principal_store.rs` | 14 — insert/resolve, escalate-only merge, no-match fallback |
| `storage/api_keys.rs` | 16 — role issuance, assign_role, verify_with_role, backward compat |
| `security/principal_resolver.rs` | 11 — all three resolution paths + env flag branch |

All 54 unit tests MUST remain green. `cargo check --package cortex-api --tests` MUST
compile clean.

---

## 7. Invariants

- **Escalate-only**: `PrincipalStore::resolve` never lowers clearance below the
  maximum of the matched bindings.
- **Level gate is absolute**: no role, compartment, or feature flag bypasses the
  `clearance_level >= fact_level` check.
- **No AC store = super_admin pass-through**: when `principal_store: None`, every
  request is treated as `super_admin("system")` to preserve backward compatibility.
- **Default posture is deny-by-default**: `PrincipalStore::deny_by_default()` is the
  recommended initialiser for production deployments; `pass_through()` is for
  development / migration.

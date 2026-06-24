# Spec 43 — ACL Admin API

Phase21 §6 — public surfaces for managing the Bell-LaPadula access-control
model: Admin CLI, HTTP endpoints, and MCP tools.

Related: [Spec 41 — Principal & RBAC](41-principal-and-rbac.md),
[Spec 42 — Access Enforcement](42-access-enforcement.md)

---

## 1. Admin CLI — `cortex-ops acl`

**File**: `crates/cortex-cli/src/bin/cortex-ops/acl.rs`

Four sub-command groups:

| Command | HTTP backing | Description |
|---|---|---|
| `acl role create` | `POST /v1/acl/roles` | Create or overwrite a RBAC role binding |
| `acl role list` | `GET /v1/acl/roles` | List all registered role bindings |
| `acl grant <principal_id>` | `POST /v1/acl/grants` | Assign clearance / role to a principal |
| `acl classify-rule list` | `GET /v1/acl/classify-rules` | List active path-classification rules |
| `acl whoami` | `GET /v1/acl/whoami` | Print the caller's effective clearance |

### `acl role create`

```
cortex-ops acl role create <name> --clearance <0-3> [--compartments c1,c2] [--api-url <url>] [--json]
```

Creates or overwrites a role binding. `--clearance` is required (0 = public,
1 = internal, 2 = confidential, 3 = restricted). Compartments are
comma-separated. `--json` emits the raw response JSON.

### `acl grant`

```
cortex-ops acl grant <principal_id> [--role <name>] [--clearance <0-3>] [--compartments c1,c2] [--api-url <url>] [--json]
```

At least one of `--role`, `--clearance`, or `--compartments` is required.
When `--role` is given the named binding is copied into a synthetic
`"principal:<id>"` binding.

### `acl whoami`

Prints:

```
principal:    <id>
clearance:    <label> (level <N>)
compartments: <c1, c2, ...>
roles:        <r1, r2, ...>
```

Clearance labels: `public` (0), `internal` (1), `confidential` (2),
`restricted` (3), `restricted+` (> 3, should not occur in practice).

**Tests**: `crates/cortex-cli/src/bin/cortex-ops/acl.rs` — unit tests for
`render_whoami`, `level_name`, and argument validation.

---

## 2. HTTP routes — `/v1/acl/*`

**File**: `crates/cortex-api/src/security/acl_routes.rs`

All mutation routes (`POST`) require the caller to hold the `acl_admin` role.
A missing or insufficient role returns:

```json
{ "reason": "forbidden", "detail": "acl_admin role required for this operation" }
```

### `GET /v1/acl/roles`

Returns all registered RBAC role bindings sorted alphabetically by name.

```json
{
  "roles": [
    { "name": "analyst", "clearance_level": 1, "compartments": ["financial"] },
    { "name": "admin",   "clearance_level": 3, "compartments": [] }
  ]
}
```

Any authenticated caller may call this endpoint.

### `POST /v1/acl/roles`  [acl_admin]

Request body:

```json
{ "name": "analyst", "clearance_level": 1, "compartments": ["financial"] }
```

- `clearance_level` MUST be 0–3; values above 3 return HTTP 400.
- `name` MUST be non-empty.
- Creates or overwrites the binding.

Response:

```json
{ "ok": true, "name": "analyst", "clearance_level": 1, "compartments": ["financial"] }
```

When no `PrincipalStore` is configured (AC disabled): HTTP 503
`no_principal_store`.

### `POST /v1/acl/grants`  [acl_admin]

Request body:

```json
{
  "principal_id": "key-abc123",
  "role": "analyst",
  "clearance_level": null,
  "compartments": []
}
```

Fields:

- `principal_id` — required; the API key id or principal identifier.
- `role` — optional; name of an existing binding to copy.
- `clearance_level` — optional; direct clearance (0–3). Used when `role` is absent.
- `compartments` — optional; list of compartment strings to union into the binding.

At least one of `role`, `clearance_level`, or `compartments` MUST be provided.
When `role` is given, its binding is copied verbatim into a synthetic
`"principal:<id>"` role (clearance + compartments from the named role).
When `role` is absent, a binding is built from the direct values.

If the daemon has an `ApiKeyStore` wired, the synthetic role is also assigned
to the matching key row so the principal resolver picks it up.

Response:

```json
{
  "ok": true,
  "principal_id": "key-abc123",
  "synthetic_role": "principal:key-abc123",
  "clearance_level": 1,
  "compartments": ["financial"]
}
```

### `GET /v1/acl/classify-rules`

Returns active path-classification rules from `cortex.toml`.

Phase21 §6.2 initial cut returns an empty list:

```json
{ "rules": [] }
```

Full live-reload is wired in §7.1 when `AccessControlConfig` is added to
`cortex-config`.

### `GET /v1/acl/whoami`

Returns the effective principal for the current caller, resolved from the
Bearer token / `x-cortex-principal` header. When no credential is supplied and
no `PrincipalStore` is configured, returns the backward-compat super-admin:

```json
{
  "id": "super-admin",
  "clearance_level": 3,
  "compartment_grants": [],
  "roles": ["acl_admin"]
}
```

**Tests**: `crates/cortex-api/tests/acl_routes_it.rs` — 5 integration tests
covering whoami, role CRUD, and `acl_admin` guard.

---

## 3. MCP tools — `cortex_acl_whoami` and `cortex_acl_grant`

**File**: `crates/cortex-mcp-server/src/tools.rs`

Phase21 §6.4 adds two MCP admin tools (tool count 35 → 37):

### `cortex_acl_whoami`

No required inputs. Returns the effective principal for the API key the MCP
server is configured with. Useful for verifying authentication before running
classified queries.

### `cortex_acl_grant`

Required: `principal_id` (string). Optional: `role`, `clearance_level` (0–3),
`compartments` (string array). Requires `acl_admin` on the calling API key.
Calls `POST /v1/acl/grants` and returns the grant confirmation.

### Principal resolution in the MCP transport

The MCP server forwards its configured API key as `Authorization: Bearer <key>`
on every upstream HTTP call to `cortex-api`. `cortex-api` resolves the
principal from the RBAC binding attached to that key and applies the
Bell-LaPadula lattice filter before returning results.

The `api_key` is configured on `ToolContext` via `.with_api_key(key)`.

**Tests**: `crates/cortex-mcp-server/src/tools.rs` — `acl_whoami_descriptor_is_well_formed`,
`acl_grant_descriptor_requires_principal_id`, `acl_grant_rejects_missing_principal_id`,
`acl_grant_rejects_empty_principal_id`; plus the schema gate
`every_tool_descriptor_inputschema_is_valid_json_schema` covers all 37 tools.

---

## 4. Security constraints

### SHALL requirements

The system SHALL reject `POST /v1/acl/roles` and `POST /v1/acl/grants` with
HTTP 403 when the caller does not hold the `acl_admin` role.

The system SHALL validate `clearance_level` is 0–3 on every inbound mutation
and return HTTP 400 with `reason: "invalid_clearance"` for values outside
this range.

The system SHALL return HTTP 503 `no_principal_store` for any mutation when
access control is not configured (no `PrincipalStore` active).

The system SHALL resolve the caller's principal from the Authorization Bearer
token or `x-cortex-principal` header; it MUST NOT honour a caller-supplied
principal override in the request body.

### Given / When / Then scenarios

#### Scenario: non-admin caller attempts role create

Given a caller without `acl_admin`  
When they POST `/v1/acl/roles`  
Then the response is HTTP 403 `forbidden`

#### Scenario: acl_admin creates a role

Given a caller with `acl_admin`  
When they POST `/v1/acl/roles` with `{ "name": "analyst", "clearance_level": 1, "compartments": ["financial"] }`  
Then the response is HTTP 200 `{ "ok": true, "name": "analyst", ... }` and the
binding is retrievable via GET `/v1/acl/roles`

#### Scenario: grant with unknown role

Given a caller with `acl_admin`  
When they POST `/v1/acl/grants` with `{ "principal_id": "x", "role": "nonexistent" }`  
Then the response is HTTP 404 `role_not_found`

#### Scenario: whoami without principal store

Given access control is not configured (`principal_store = None`)  
When any caller GET `/v1/acl/whoami`  
Then the response is HTTP 200 with the super-admin pass-through principal

#### Scenario: clearance above 3

Given a caller with `acl_admin`  
When they POST `/v1/acl/roles` with `clearance_level: 5`  
Then the response is HTTP 400 `invalid_clearance`

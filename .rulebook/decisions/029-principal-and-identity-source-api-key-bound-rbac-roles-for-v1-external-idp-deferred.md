# 29. Principal and identity source: API-key-bound RBAC roles for v1; external IdP deferred

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

Enforcement requires a principal (identity + clearance + compartments) for every query. We need to decide: where does the principal come from, what is the trust boundary, and what happens for unauthenticated callers? The system already has an `ApiKeyStore`; adding IdP / OIDC-JWT integration in v1 would multiply scope significantly.

## Decision

For v1 the principal is resolved from the authenticated API key: each API key carries a role binding, and each RBAC role carries a clearance level + compartment grants. Resolution order: (1) authenticated API key → bound role → principal; (2) signed `x-cortex-principal` header (a compact JWT minted by the operator, for trusted-caller scenarios such as an internal gateway) when configured as a trusted issuer; (3) unauthenticated → the configured default principal (default: public-only, no compartments). External IdP / OIDC-JWT integration is explicitly out of scope for v1 and recorded as the follow-up path. Trust boundary: Cortex trusts the `x-cortex-principal` header only when `access_control.trusted_principal_header = true` (default false); otherwise it is ignored. A principal can never assert a clearance or compartment it was not granted by the operator.

## Alternatives Considered

- OIDC-JWT from an external IdP (Okta, Entra ID) — deferred: the integration adds a dependency on an external service, token rotation, JWKS endpoint management, and claim-to-clearance mapping; all are real engineering; deferring keeps v1 scope bounded while the RBAC layer ships.
- mTLS client certificates — rejected for v1: requires PKI infrastructure; too heavyweight for a local/small-team stack.
- Per-request clearance assertion by the caller — rejected: a caller asserting its own clearance is self-elevating; the operator must be the authority.
- No principal model (just API key auth) — rejected: API key auth already exists; without a principal the lattice check has no clearance to compare against.

## Consequences

Pros: builds on the existing ApiKeyStore without a new infrastructure dependency; role-based model lets one role grant cover many keys; the trusted-header path accommodates a future gateway integration without code changes. Cons: key compromise means clearance compromise (no short-lived tokens in v1); IdP deferred means multi-org deployments must wait for v2; the `x-cortex-principal` trusted-header path requires careful operator configuration to avoid trust escalation.

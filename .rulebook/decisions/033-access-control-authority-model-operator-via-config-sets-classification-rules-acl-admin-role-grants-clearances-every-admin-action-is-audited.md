# 33. Access control authority model: operator-via-config sets classification rules; acl_admin role grants clearances; every admin action is audited

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

We need to define who has the authority to (a) set classification rules on data, (b) grant clearances and compartments to principals, and (c) verify those actions are audited. Without a clear authority model, a principal could escalate its own clearance or a misconfigured rule could over-classify public data into restricted, both of which undermine the model.

## Decision

Two authority tiers. Tier 1 — classification rules (what data is classified at what level): set by the operator via `cortex.toml` `[cortex.classification]` rules and via the admin API (`/v1/acl/classify-rule`), both gated by the `acl_admin` role. Classification rules are evaluated at ingestion time (bootstrap + classifier worker); changing a rule requires a re-index to propagate to already-indexed facts. Tier 2 — principal grants (who gets what clearance): managed by an `acl_admin` principal via the admin API (`/v1/acl/grant`) or the CLI (`cortex-ops acl grant`). A principal CANNOT grant a clearance level or compartment that it does not itself possess (no privilege escalation). The `acl_admin` role itself can only be granted by the initial bootstrap key (the key configured in `cortex.toml` at install time). Every Tier 1 and Tier 2 mutation is recorded in the audit log as an `acl_admin_action` event (principal, action, target, before, after) so the trail is immutable and queryable. The `cortex-ops acl whoami` command lets any caller inspect their own effective principal without exposing other principals.

## Alternatives Considered

- Any authenticated caller can grant clearances — rejected: privilege escalation. A junior-clearance caller granting itself `restricted` clearance breaks the entire model.
- Classification rules are immutable after deployment (no admin API) — rejected: operational reality requires rules to evolve (new project, new data type); a re-deploy just to add a rule is impractical.
- Audit is optional / operator-configurable — rejected: auditing admin actions is non-negotiable for a security control; operators cannot opt out of the admin audit trail.

## Consequences

Pros: clear authority boundary (operator config vs runtime grants); no-escalation invariant is enforced at the grant API level; the audit trail provides an immutable record for compliance; `whoami` gives operators a debugging surface without exposing other principals' clearances. Cons: the bootstrap-key-only path to `acl_admin` means losing the bootstrap key requires a config-file reset; rule changes require a re-index (mitigated by `cortex-ops migrate-classification --dry-run` to preview impact before committing).

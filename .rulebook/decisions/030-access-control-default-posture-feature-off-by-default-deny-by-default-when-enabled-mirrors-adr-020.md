# 30. Access control default posture: feature OFF by default; deny-by-default when enabled; mirrors ADR-020

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

Cortex is currently deployed as a single-operator local stack where any caller sees everything. Adding access control must not break existing deployments when they upgrade. At the same time, once operators opt in to access control, the system must fail closed — a misconfigured query must not leak classified facts.

## Decision

The access control feature defaults OFF (`access_control.enabled = false`). When disabled, every enforcement point is a no-op pass-through: all facts are returned to any caller, preserving the existing single-trust-domain behaviour exactly. When enabled, classified facts are deny-by-default to any principal whose clearance or compartments do not satisfy the lattice. A second knob `deny_on_missing_principal` (default `true` when `enabled=true`) controls the unauthenticated-caller case: `true` means an unauthenticated caller sees only `public` facts (level 0, no compartments); `false` means an unauthenticated caller is treated as having the configured default principal (backward-compat bridge for phased rollouts). This mirrors ADR-020 (cross-project propagation opt-in) in spirit: the feature ships behind a flag so operators can deploy the code before activating the policy.

## Alternatives Considered

- Default ON — rejected: would immediately break every existing deployment by denying facts to callers that have no principal set; a flag-day breaking change on upgrade is unacceptable.
- Default ON, but only classify newly-ingested facts — rejected: creates a split corpus (classified new + unclassified old) that is confusing and hard to audit; the backfill step ensures the corpus is uniformly classified when the feature is enabled.
- No global enabled/disabled flag (always enforce) — rejected: same breakage as Default ON above; a flag is the only safe migration path for existing operators.

## Consequences

Pros: existing single-operator deployments upgrade without any behaviour change; operators can run the classification backfill and configure principals before activating enforcement; mirrors the proven ADR-020 opt-in pattern. Cons: a misconfigured operator who activates enforcement before principals are set up will see 403s on every query (but this is the correct fail-closed behaviour, not a bug); the feature flag adds a branch at every enforcement point (mitigated by an `Arc<RwLock<AccessControlConfig>>` snapshot so the branch is a single bool read).

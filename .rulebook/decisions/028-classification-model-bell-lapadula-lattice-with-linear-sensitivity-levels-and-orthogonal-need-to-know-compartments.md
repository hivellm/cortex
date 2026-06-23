# 28. Classification model: Bell-LaPadula lattice with linear sensitivity levels and orthogonal need-to-know compartments

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

Cortex is a single-trust-domain store: every caller can retrieve every indexed fact. To support enterprise / multi-user deployments, facts must carry a sensitivity label so the retrieval engine can enforce least-privilege. We need to pick a classification model that is simple enough to implement in a single phase, expressive enough for real enterprise data, and maps cleanly onto the existing column-stamper + retrieval-wedge machinery from phase18.

## Decision

Adopt a two-axis classification model. Axis 1 — sensitivity level: a linear ordinal `public=0 < internal=1 < confidential=2 < restricted=3`. Axis 2 — compartments: an orthogonal set of need-to-know labels (canonical vocabulary: `financial`, `hr`, `legal`, `security`, `customer_pii`; open/config-extensible). The read predicate is Bell-LaPadula "no read up + need-to-know": `principal.clearance_level >= fact.class_level AND fact.class_compartments ⊆ principal.compartment_grants`. Two columns land on every retrievable entity: `class_level: u8` and `class_compartments: Vec<String>`. Facts lacking an explicit classification receive the configured default level (`internal`) + empty compartments via idempotent backfill. The compartment vocabulary is open — operators may define domain-specific compartments beyond the canonical set in `cortex.toml`.

## Alternatives Considered

- Mandatory Access Control with a pure lattice (no compartments) — rejected: cannot express need-to-know (a CFO-level fact visible to finance only, not all `restricted` principals); the compartment axis is essential.
- Attribute-Based Access Control (ABAC) with arbitrary policies — rejected: too expressive for a v1 implementation; Bell-LaPadula covers 95 % of enterprise use cases with a provably sound model.
- Tag-only model (no ordinal level) — rejected: tag equality lacks the dominance relation needed for "clearance ≥ level" queries and Meili range filters.
- Five-level system (adding `top_secret`) — rejected: adds complexity with no current corpus need; the four-level model matches standard commercial data classification (public / internal / confidential / restricted).
- Single level only (no compartments) — rejected: cannot separate financial from HR data at the same sensitivity level; a CFO must not see HR records and vice versa.

## Consequences

Pros: linear level enables O(1) `<=` lattice check and a Meili range filter (`class_level <= N`); compartment-subset check is straightforward set containment; model is formally sound (Bell-LaPadula); column schema is compact (1 int + 1 string array); maps directly onto the phase18 bitemporal column-stamper pattern. Cons: compartment-subset check requires the full compartment set to travel with every fact (slight storage overhead); the ordinal mapping is fixed — adding a level between existing ones requires a migration; the canonical compartment vocabulary must be documented and agreed upon before first data lands.

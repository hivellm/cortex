# Access Control & Data Classification

## ADDED Requirements

### Requirement: Every retrievable fact carries a classification
The system SHALL stamp every retrievable entity (Meili document, Vectorizer
payload, Nexus node) with a `class_level` (ordinal: `public=0 < internal=1 <
confidential=2 < restricted=3`) and a `class_compartments` set at ingestion.
Facts lacking an explicit classification MUST receive the configured default
level and an empty compartment set via the idempotent backfill.

#### Scenario: Ingested financial doc is classified
Given a `cortex.toml` classification rule `finance/** → confidential + [financial]`
When the bootstrap walker ingests `finance/runway.md`
Then the emitted event carries `class_level = confidential` and `class_compartments = ["financial"]`
And all three backend projections (Meili / Vectorizer / Nexus) carry the same labels.

#### Scenario: Classifier escalates but never downgrades
Given a doc declared `internal` by path rule whose body contains salary tables
When the classifier worker detects HR-sensitive content
Then the merged classification is `confidential` with compartment `hr`
And the classifier MUST NOT lower a declared level.

### Requirement: Retrieval enforces the lattice for the query principal
The system SHALL return a fact to a principal only when
`principal.clearance_level >= fact.class_level` AND
`fact.class_compartments ⊆ principal.compartment_grants`. Enforcement MUST
apply at every retrieval surface: each backend lane filter, the post-fusion
wedge, the pre-thinking bundle, the raw `/v1/search/*` proxies, and every MCP
tool that returns facts.

#### Scenario: Under-cleared principal cannot read a restricted fact
Given a principal with `clearance_level = internal` and no compartments
And a corpus containing a `restricted + [security]` incident post-mortem
When the principal queries any surface for that incident
Then no surface returns the restricted fact
And an `access_decision` audit envelope records the deny with the principal id.

#### Scenario: Compartment need-to-know is enforced
Given a principal cleared to `confidential` but granted only `[hr]`
And a `confidential + [financial]` board deck
When the principal queries for it
Then the fact is denied because `[financial] ⊄ [hr]`.

### Requirement: The feature defaults OFF and fails closed when ON
The system SHALL default `access_control.enabled = false` so existing
single-operator deployments behave identically. When enabled, a classified
fact MUST be deny-by-default to any principal that does not clear the lattice,
and a missing/unauthenticated principal MUST be treated per
`deny_on_missing_principal` (default: see only `public`).

#### Scenario: Disabled access control is a pass-through
Given `access_control.enabled = false`
When any caller queries any surface
Then every fact is returned regardless of `class_level` (backward-compat).

#### Scenario: Enabled access control fails closed on missing principal
Given `access_control.enabled = true` and `deny_on_missing_principal = true`
When an unauthenticated caller queries a classified scope
Then the surface returns `403 forbidden_classified` (or only `public` facts per config)
And never leaks a classified fact.

### Requirement: A leak is a hard CI failure
The system SHALL ship a golden access-control eval suite and an adversarial
leak probe. The suite's false-grant count (a principal seeing a fact it is not
cleared for) MUST be exactly zero; a single leak MUST fail CI.

#### Scenario: Zero-leak gate
Given the access-control golden suite over mixed-classification fixtures
When the eval harness runs every (principal × fact) pair across every surface
Then the false-grant count is 0
And the CI gate blocks merge on any non-zero false-grant count.

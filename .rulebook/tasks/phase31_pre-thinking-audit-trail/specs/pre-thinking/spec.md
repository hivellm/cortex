# Standalone Pre-Thinking Audit

Execution depends on completed `phase36_local-first-core`.
This replaces the full-stack audit delta preserved at Git baseline
`2ba3d2f502f3f3955166ac272ae5d8b28c2aea05`.
See `docs/analysis/local-first-core-2026-10/02-backlog-review.md`.

## ADDED Requirements

### Requirement: Canonical audit survives index reconstruction

The standalone engine SHALL persist bounded immutable audit records before
acknowledging audit persistence. SQLite SHALL be a rebuildable projection.
Records SHALL identify exact scope, query/delivery, delivered memory revision
and hash, source references, timestamps, outcomes/errors and measured latency.
Retries and concurrent writes SHALL not silently duplicate or lose observations.

#### Scenario: Restart and rebuild retain exact evidence

Given a persisted delivery references memory revision 1 and that memory later has revision 2
When Cortex restarts and reconstructs its audit index from canonical records
Then the delivery still references revision 1 and its original sources
And its errors, latency and evidence labels remain inspectable.

#### Scenario: Invalid canonical input cannot destroy the working index

Given a working audit index and an invalid canonical audit record
When reconstruction encounters that record
Then reconstruction reports the source and validation error
And retains the working index without claiming successful reconstruction.

### Requirement: Delivery and use evidence remain distinct

Audit SHALL expose prepared delivery, observed related action, explicitly
evidenced use and unknown as distinct evidence labels with supporting references.
Temporal correlation SHALL NOT imply consumption or compliance. Explicit use
claims SHALL identify their evidence and attribution; self-reported use SHALL
not be presented as independently verified action compliance.

#### Scenario: Adjacent action supplies correlation only

Given a prepared delivery and a later tool action in the same observed session
When only their temporal order links the two
Then the audit may show an observed related action with its correlation method
And SHALL NOT report explicitly evidenced use.

#### Scenario: Missing identifiers remain unknown

Given a client provides no observed session or turn identifier
When its delivery is recorded
Then the unavailable identifiers remain absent
And Cortex invents neither identifiers nor links to another session.

#### Scenario: Failed preparation is not successful delivery

Given recall or context preparation fails
When Cortex records the attempt
Then its error and measured latency remain inspectable
And it is not counted as successful prepared delivery or evidenced use.

### Requirement: Scope applies to all audit surfaces and links

Lookup, action linkage, metrics and GUI results SHALL enforce exact repository
scope. All-repository inventory SHALL require explicit local operator intent.
Untrusted attribution SHALL not grant access or broaden scope.

#### Scenario: Other repository evidence is excluded

Given a delivery in repository A and an action in repository B with matching text
When a client inspects or correlates audit evidence scoped to A
Then B's action is neither returned nor linked nor counted.

### Requirement: Inspection uses the standalone engine

CLI and MCP SHALL inspect bounded audit records without an HTTP server or
external daemon. The embedded GUI SHALL show the same evidence through its
optional local server, preserving authenticated mutation and safe rendering
contracts. Metrics SHALL include denominators, unknowns, errors and latency;
they SHALL NOT describe correlation counts as causal productivity gains.

#### Scenario: Direct inspection works with services stopped

Given canonical audit records and no running HTTP server or Docker services
When the operator queries them through CLI or local MCP
Then matching scoped records and evidence-class metrics are returned.

#### Scenario: GUI exposes uncertainty and provenance

Given an audit record with an uncertain action link and an imported text source
When the operator opens it in the embedded GUI
Then exact memory revision, source, link method, uncertainty, error and latency are visible
And imported text cannot execute as markup or script.

### Requirement: Retention is bounded and explicit without source deletion

Record and query sizes SHALL have documented bounds. Canonical audit retention
SHALL require an explicit operator action with dry-run impact; no background
operation SHALL silently delete audit or original source data. Retention SHALL
never cascade into memories or historical imports, and affected references SHALL
remain preserved or visibly unavailable.

#### Scenario: Retention preview leaves data untouched

Given audit records referring to canonical memories and imported historical sources
When the operator previews retention
Then Cortex reports affected records and references without deleting any file.

#### Scenario: Explicit audit retention cannot remove sources

Given an explicitly authorized audit retention operation
When it applies its documented bounds
Then original memories and imported source data remain unchanged
And subsequent inspection never misrepresents removed evidence as available.

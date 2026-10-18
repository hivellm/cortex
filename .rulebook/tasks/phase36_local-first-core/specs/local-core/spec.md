## ADDED Requirements

### Requirement: Standalone local runtime
The system SHALL provide capture, search, graph traversal, import, classification,
MCP and an embedded web GUI through one executable without mandatory Docker,
external index services, GPU or a separate adapter daemon.

#### Scenario: Agent reads without a server
Given an initialized local store and no running Cortex or Docker service
When an agent invokes the CLI or local MCP search
Then cited, repository-scoped results are read directly from local storage.

### Requirement: Canonical files and reconstructible indexes
The system MUST persist canonical readable files before acknowledging a write and
SHALL reconstruct search, versions and graph links from those files.

#### Scenario: Index recovery
Given canonical records including a correction and typed links
When a new index is built from those records
Then search and bounded graph traversal preserve identity, provenance and authority.

#### Scenario: Interrupted or competing writes
Given concurrent writers or interruption between canonical publication and indexing
When a write is retried or recovery runs
Then acknowledged content remains durable and conflicting content is not overwritten.

### Requirement: Historical corpus preservation
The system MUST import legacy daily SQLite and compressed NDJSON archives without
modifying source files, promoting observations to accepted decisions or evicting
old imported records from the searchable corpus.

#### Scenario: Resumable migration
Given a large archive with a corrupt source and an interrupted earlier import
When import resumes
Then successful records remain searchable without duplicates and failures identify
their source without advancing beyond uncommitted data.

### Requirement: Advisory Jev classification
The system SHALL persist validated typed Jev classifications separately from source
authority, enforce bounded provider work and retain retryable failures locally.

#### Scenario: Provider unavailable
Given a queued record and unavailable Jev credentials or service
When classification is attempted
Then the record remains durably searchable and its classification remains pending
with an explicit reason, without fabricated probabilities or successful status.

### Requirement: Local interface isolation
The system MUST preserve exact repository identity across CLI, MCP, hooks and GUI,
bound graph traversal and returned context, and protect local HTTP mutations.

#### Scenario: Untrusted browser request
Given the local HTTP server and a request from an unrelated website
When the request attempts to read memory or mutate records
Then host/origin and authentication checks reject it without exposing stored data.

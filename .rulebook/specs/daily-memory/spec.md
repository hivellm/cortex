# Daily Memory

## Requirements

### Requirement: Native memory preservation
The system SHALL disable automatic native-agent memory consolidation for fresh
and existing installations before daily recovery activation. It MUST preserve
native memory files, their indexes, raw archives and existing data volumes.

#### Scenario: Existing enabled consolidation job
Given a file-backed scheduler database with `retention.memory_consolidate` enabled
When the recovery migration and default-job seeding run
Then the job is disabled before execution and remains disabled after restart
And native memory file contents and hashes remain unchanged.

### Requirement: Local durable explicit memory
The system SHALL commit explicit decisions and lessons to existing Cortex SQLite
and make them immediately queryable without Jev, GPU or bulk workers. It SHALL
maintain FTS and canonical typed lifecycle evidence atomically with projection.

#### Scenario: Explicit write while optional services are absent
Given Jev, Synap, Meili, Vectorizer, reranker and bulk workers are unavailable
When an authorized client explicitly captures a lesson
Then capture commits durably and local read returns its exact ID and content
And process restart preserves the result.

### Requirement: Honest provenance and compatible capture
The system MUST preserve origin, repository, source event, capture time, status
and available source session/turn and references. It SHALL accept existing
capture clients without fabricating agent or source session identity. Trusted
transport metadata SHALL outrank client claims; attribution MUST NOT grant authority.

#### Scenario: Legacy capture has no agent metadata
Given a client sends existing kind, body and repo fields only
When capture succeeds
Then its memory is locally durable and readable with unknown origin/session
And it is not falsely labeled Claude or assigned a fabricated observed session.

#### Scenario: Model supplies a different agent label
Given an authenticated or trusted adapter provides one origin and input claims another
When capture resolves attribution
Then trusted metadata wins and the conflicting claim grants no extra access.

### Requirement: Idempotent transactional writes
The system SHALL support caller-scoped retry identity and expected revision.
Equivalent retries MUST preserve one logical write; changed content under the
same retry identity MUST fail as a conflict. Revision changes SHALL be atomic.

#### Scenario: Capture retry and conflicting reuse
Given an explicit write with a stable caller/repo/retry key has committed
When the same request is retried
Then the same ID, version and receipt return without another revision bump
And reusing that key for different content returns a conflict without mutation.

### Requirement: Repository and applicability isolation
The system SHALL check repository, authorized global scope, lifecycle, branch/path
restrictions and known source validity before ranking. Normal daily context MUST
exclude unrelated, proposed, superseded, deprecated and stale advice. FTS ranking
and classifier suggestions MUST NOT broaden membership or override repository rules.

#### Scenario: Another repository and unrelated distractor
Given applicable memory in repo R, unrelated memory in R and memory in repo S
When an ordinary prompt in R requests daily context
Then the exact applicable ID is returned and both distractors are absent.

#### Scenario: Referenced source changes
Given a lesson is bound to a known source hash or branch
When deterministic source validation detects a mismatch
Then the lesson is stale or inapplicable and excluded from current advice
And its source and history remain traceable.

### Requirement: Explicit typed supersession
The system SHALL reuse existing typed decision statuses and supersession semantics.
It MUST require an explicit authorized correction with predecessor and matching
revision, validate scope/cycles, and atomically expose the successor. A model
judgment, negative feedback or repeated observation MUST NOT supersede memory.

#### Scenario: Corrected decision within the cache window
Given an accepted predecessor was delivered and its prompt is cached
When an explicit successor commits with the matching expected revision
Then the next identical prompt returns the successor and excludes the predecessor
And restart retains the successor while predecessor history remains accessible.

#### Scenario: Concurrent correction loses the revision race
Given two clients read the same predecessor revision
When both submit different successors against that revision
Then one succeeds and the stale write conflicts without partial lifecycle changes.

### Requirement: Persistent receipts and version-linked feedback
The system SHALL distinguish local acceptance, optional index completion and
response preparation. It SHALL persist exact prepared memory IDs/versions and
feedback in file-backed storage. It MUST NOT claim model consumption from server
response preparation alone or report unobserved index completion.

#### Scenario: Read and feedback survive restart
Given a read produced a delivery ID and feedback references that delivery
When the API process restarts
Then exact prepared versions and feedback are retained
And the receipt's channel/outcome states only what was actually observed.

### Requirement: Candidate-only optional Jev
External classification SHALL require `classify=true` on that capture. The
default and explicit false SHALL make zero provider calls even with a configured
key. Production payloads SHALL exclude all other stored memories, including
local-only records in the same repository.

The system SHALL use optional pinned `jev-1.13.0` typed judgments only on explicit
memory candidates after durable capture. Jev MUST NOT drop explicit writes,
change scope/lifecycle, promote rules, generate summaries or rewrite native files.
The core memory loop SHALL operate with no provider key or provider availability.

#### Scenario: Local-only capture followed by an authorized classification
Given an explicit local-only record and a configured provider key
When a separate candidate requests classification
Then only the opted-in candidate is transmitted
And the local-only record is absent from the provider request.

#### Scenario: Explicit write classified transient
Given a client explicitly captures a durable lesson
When Jev returns transient, uncertain or unavailable
Then the write still persists and remains eligible under deterministic checks
And only advisory metadata reflects the classification result.

#### Scenario: Raw tool traffic
Given ordinary raw turn and tool-call envelopes are processed
When the candidate enrichment gate evaluates them
Then this daily-memory integration makes zero Jev requests for those envelopes.

### Requirement: Bounded sanitized classification
The system SHALL sanitize all Jev egress, validate typed finite answers and cap
state at 8 KiB, response at 16 KiB, attempts at one, deadline at 2 seconds,
concurrency at one and persisted uncached calls at 100 per UTC day. It SHALL
cache successful judgments by model/question/schema/content/version/revision and
record actual usage without logging secret state or raw error bodies.

#### Scenario: Provider failure, unsafe state or exhausted budget
Given input cannot pass sanitization/size validation or the provider fails or budget is exhausted
When candidate classification runs
Then a deterministic advisory fallback returns within the deadline without extra attempts
And the core write/read result remains available with an honest fallback reason.

#### Scenario: Cache and process restart
Given a valid judgment is cached and the daily call allowance is partially spent
When an identical candidate is classified again or the process restarts
Then a valid cache hit makes no network call and persisted allowance is preserved
And a changed model/question/source revision cannot reuse an incompatible judgment.

### Requirement: Bounded ordinary-prompt injection
The system SHALL retrieve active local memories independently of bulk content
intent routing and inject at most five records within 1,536 UTF-8 bytes including
IDs and citations. Total production pre-thinking SHALL respect 600 ms, retain
completed local context when optional lanes fail, and key cache by effective
scope/revisions and relevant request context.

#### Scenario: Ordinary fresh-session prompt
Given session A captured a correction and fresh session B asks a matching task prompt
When ordinary pre-thinking runs with optional stack services down
Then B receives the exact active memory and source inside configured time/byte bounds
And a fail-open outcome is distinguishable from empty relevant context.

### Requirement: Verified installed harness contracts
The system SHALL test packaged Claude lifecycle output with its adapter running
and document actual installed harness/version/MCP capabilities for Claude, Codex,
Grok and Kimi. Available harnesses SHALL exercise real capture/read transports;
unavailable paths MUST be labeled unverified or unsupported. Model API reviews
and MCP registration alone MUST NOT count as lifecycle/context-delivery tests.

#### Scenario: Packaged Claude SessionStart
Given current installed hook binary and adapter are running
When the real packaged SessionStart hook executes
Then it awaits and returns the correct context in the supported response shape
And observed output matches persisted prepared memory IDs.

#### Scenario: A model exists but its CLI harness does not
Given Grok or Kimi is reachable through a model router but no corresponding client is installed
When client acceptance is reported
Then that client path is marked unverified/unsupported with its blocker
And the model response is recorded only as design/review participation.

### Requirement: Bounded optional archival processing
The system SHALL perform no recurring whole-archive decompression in daily mode.
Optional archival mode SHALL publish new evidence through bounded incremental
processing with durable checkpoints, duplicate-safe retries and visible lag/errors.
It MUST preserve raw history and distinguish historical backfill from daily work.

#### Scenario: New archive evidence and checkpoint restart
Given optional archival mode has processed closed historical files
When new evidence appends and the process restarts during publication
Then bounded work resumes from durable state and publishes each logical event once
And closed history is not fully decompressed again to discover the new event.

### Requirement: Lightweight operation and honest capability status
The system SHALL provide a daily runtime profile without mandatory GPU, reranker
or bulk indexing. Profile changes MUST preserve existing data and use no OS
scheduler. Status SHALL identify optional unavailable capabilities and capture/
delivery failures rather than imply full-stack health.

#### Scenario: Lightweight activation
Given existing full-stack data and native files
When the authorized lightweight profile is activated
Then core capture/read/feedback work, data remains intact and optional capabilities are explicit
And CPU, RSS, IO, process counts and query latency are measured over stated intervals.

#### Scenario: Daily profile with inherited full-stack configuration
Given backend URLs, canaries, retention and dashboard watchers remain configured
When runtime.profile or CORTEX_RUNTIME_PROFILE selects daily
Then optional factories, probes, jobs, raw publishing and WAL replay do not start
And the original URLs, credentials, data paths and archive WAL remain unchanged
And optional HTTP endpoints return disabled_by_profile before backend access
And archive incremental processing still requires its separate explicit opt-in.

#### Scenario: Daily health with unavailable local storage
Given the daily API is running but its metadata database cannot be opened
When basic health or status checks local memory availability
Then memory is unavailable rather than ready
And disabled bulk services do not trigger probes or mask the local failure
And successful SQLite observation never claims a model consumed prepared context.

#### Scenario: Manual launcher ownership and stale binaries
Given an occupied port, existing pipe, reused PID or an older full-profile binary
When the daily launcher starts or stops its recorded processes
Then it requires the expected process identity, listener ownership and daily health profile
And unrelated processes and all existing volumes remain untouched
And no OS schedule is created.

#### Scenario: Hook-started runtime after a reboot
Given no Cortex process is running and the hook uses the default adapter endpoint
When a synchronous Claude Code or Codex hook finds the endpoint absent
Then cortex-hook starts the sibling daily runtime detached under a spawn lock and backoff
And the hook exits 0 within its budget, showing a notice only when the start fails
And no Run key, scheduled task, Startup entry or service is registered.

### Requirement: Behavioral acceptance and independent review
The system SHALL pass seeded exact-recall, correction, restart, scope, distractor,
outage, resource and actual-delivery acceptance. Required integration jobs MUST
fail on absent dependencies. Final patch reviews SHALL record actual Sol/Astra,
Claude, Kimi and Grok participation and limits; all required quality gates MUST pass.

#### Scenario: Cross-session corrected action
Given exact seeded prior mistake, verified correction and distractors
When another available agent receives context and performs the paired task
Then observed context contains the exact correction and its action follows that correction
And results, limits, actual harness and reviewer identities are retained as evidence.

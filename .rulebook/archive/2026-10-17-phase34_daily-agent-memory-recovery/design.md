# Daily memory recovery design

## Decision and scope

Implement one compact local memory path in Cortex's existing SQLite storage.
Decisions and verified lessons must be captured, queried, and delivered without
GPU, embeddings, full-text services, queue workers, archive replay, or Jev. Optional
full-stack indexing mirrors evidence and never owns daily-read availability.

Authority: the operator's one-task priority override, recorded in
`docs/analysis/daily-agent-memory-2026-10/05-joint-decision.md`. Execution follows
this task's `tasks.md` exactly. This document finalizes §1.1; it is not code or a
claim that later sections have started. Ownership and per-item gates are in
`06-execution-plan.md` in the same analysis directory.

## Protect existing memory first

Disable automatic `retention.memory_consolidate` on fresh installs and reconcile
the existing scheduler row to disabled before a scheduler tick can launch it.
Persist the safety migration so default-job reseeding does not undo it. An
already enabled installation is covered, not just changed compile-time defaults.
Report an already running destructive rewrite rather than racing it. No automatic
native memory rewrite belongs to the daily profile. Preserve raw archives,
native memory files, their index files, and full-stack volumes. Repair of earlier
merged files is a separate explicit operation, not an automatic migration.

## Durable state and transactions

Add a small schema migration within `cortex-storage` and reuse the application's
file-backed metadata database. Check bundled SQLite FTS5 support in tests/build;
fail explicit startup validation if the required local search feature is missing.
Do not silently substitute the existing query-ignoring memory keyword lane.

Conceptual tables, with final names following repository storage conventions:

- A memory projection keyed by stable memory/decision ID, holding category
  (`decision` or `lesson`), canonical decision status, body, content hash,
  repository scope, optional branch/path applicability, origin, source references,
  capture time, version, and supersession linkage. Store a deterministic stale
  marker when a known referenced source revision/hash no longer matches.
- Canonical typed event evidence for the explicit write and lifecycle change,
  persisted in the same transaction. Reuse `Kind::Decision` and
  `DecisionPayload` for authoritative decision/correction lifecycle; lesson is a
  projection category/tag, not a second incompatible status machine. Preserve
  original legacy capture event identity and kind as source evidence when an
  explicit memory/knowledge/learning capture is projected.
- An FTS5 index of bounded searchable title/body data maintained transactionally
  with the projection. FTS selects candidates; scope, lifecycle, and source
  checks determine eligibility before ranking.
- Scope revision counters incremented transactionally by content/lifecycle/source
  validity changes. A permitted explicit global scope has its own revision;
  repository reads include both revisions only when global inclusion is enabled.
- An idempotency mapping from effective scope, caller namespace and supplied
  retry key to request hash and result. Equal retries return the same identity,
  timestamp, version, and revision without new evidence or authority. A reused
  key with different content is a conflict. A missing key retains legacy behavior
  and does not imply semantic deduplication of distinct events.
- Delivery rows keyed by delivery/query ID, storing effective scope, revision,
  exact memory IDs/versions included, byte count, time, channel, and outcome.
  Feedback references the delivered version when supplied; existing query-ID
  feedback stays compatible and uses the persistent store.

Transaction boundary: validate input and expected revision, persist canonical
evidence, update projection/status and FTS, increment affected revisions, and
save retry result atomically. A successful receipt means durable local commit and
immediate local read availability. Optional mirroring uses existing ingestion
contracts with duplicate-safe retry; it must not label acceptance `index_complete`
without an observed index receipt. Failure to mirror does not roll back memory.

## Provenance, identity, and scope

Minimum provenance: stable record and source event IDs, effective repository,
origin agent, source session/turn when known, captured-at time, status, and source
references. Retain a referenced file hash/revision when supplied and applicable.
Unknown legacy agent/session stays unknown. A correlation ID minted by Cortex is
labeled generated, never presented as an observed source session.

Trusted adapter/transport metadata takes precedence over arguments. MCP client
name and agent labels are attribution with a recorded trust/source field, not
authentication. A model-supplied label cannot grant broader scope or overwrite
another actor's record. Existing authorization checks still apply.

Default scope is the resolved repository. Global scope requires an explicit
authorized request; classifier suggestions never broaden scope. Branch/path
restrictions and known source validity are checked before ranking. Explicit
accepted general repo lessons can be pinned by an explicit applicability flag;
otherwise candidate content must match the task via deterministic lookup. Ranking
uses FTS relevance and stable tie-breaking, not repetition count or model prestige.
Superseded/deprecated/proposed/stale records are excluded from normal advice and
remain accessible through explicit history/source lookup.

## Capture, read, and delivery contracts

### Section 1.4 wire agreement

`POST /v1/ingest` retains required `kind`, `body`, `repo` and optional `topic`,
`severity`. Add `decision` kind and optional `title`, `memory_id`, `status`,
`agent`, `session_id`, `turn_id`, `retry_key`, `expected_revision`, `supersedes`,
`branch`, `path`, `pinned`, `sources`, and explicit `global` (default false).
Caller namespace comes from resolved transport/principal context, never a
request-supplied authority field. Labels supplied by MCP initialization or tool
arguments are client-reported attribution; they are not trusted authentication.
No wire `attribution=trusted_transport` claim is accepted from model arguments.

`POST /v1/memory/query` accepts required `repo`, `query`, optional `branch`,
`paths`, `include_global` (false), `limit` (5, at most 5 for delivery), and
`budget_bytes` (1536 maximum for the rendered memory context). Response includes
`context`, `query_id` equal to `delivery_id`, `revision`, `global_revision`,
and a bounded list of delivered `memories` with exact `id`, `version`, `event_id`
and `scope`. Context carries provenance/source citations; IDs include only
records actually rendered, not candidates discarded by formatting/budget.

`cortex_memory_read` forwards this contract. Existing `cortex_feedback_record`
accepts the returned `query_id`; joining it to the persistent delivery row links
feedback to the exact delivered versions. A receipt records `prepared` only,
not proof of client receipt or model consumption. Missing durable storage is an
explicit service error. Production memory and feedback use the same file-backed
metadata database; test-only router constructors may use explicit in-memory state.

The coordinator owns the storage delivery contract for this item; Claude owns
API wiring and Sol owns MCP. No Jev, pre-thinking hooks or runtime profiles are
implemented before their checklist items.

Keep `cortex_capture_memory` and `POST /v1/ingest` compatible for existing kind,
body, repo, topic, severity clients. Add optional provenance, retry key,
applicability, source refs, expected revision, and explicit supersession inputs;
support typed decision capture additively. Legacy explicit captures become
durable local lessons with honestly unknown origin where metadata is absent.
Do not reject existing clients for missing agent/session or invent Claude origin.

Explicit correction/supersession requires an authorized operation naming the
predecessor and matching expected revision. Reuse the typed decision transition.
Validate scope, self-reference and cycles; atomically expose successor and mark
predecessor superseded. Jev suggestions and negative feedback never perform this
operation. Existing canonical lifecycle statuses remain canonical; `active` in
daily context means accepted and eligible, not another stored lifecycle.

Expose local `POST /v1/memory/query` and MCP `cortex_memory_read`, returning a
small cited context plus exact memory IDs/versions, scope revisions, and delivery
ID. The existing pre-thinking path uses the same local query contract, regardless
of its primary content-search intent. No new independent service or speculative
CLI adapter is required.

Persist response preparation before reporting a delivery ID. Distinguish
`prepared`/`transport_failed` and an actual observed client delivery where the
harness can establish it. A server-prepared response is never labeled proof of
model consumption. If receipt persistence fails, return an explicit error or
degraded outcome rather than invent durable delivery evidence. Optional feedback
is durable and version-linked; no lane-weight learning or automatic authority
promotion is introduced.

## Bounded ordinary-prompt context

The daily-memory section is at most five records and 1,536 UTF-8 bytes including
IDs, provenance and citations. Whole records or body excerpts are trimmed
deterministically; never emit a body without its identifying source. No relevant
eligible records means no advice section. Excess candidate rows are bounded
before formatting. Preserve repository rules as higher authority than memory.

The total production pre-thinking deadline is 600 ms. Local scoped retrieval is
scheduled before optional full-stack work, with a latency target of 100 ms and
a hard deadline at 550 ms from pipeline entry for normal production budgets.
Scope discovery is bounded to 100 ms. Authentication is included in the local
deadline. No network model call occurs in the local memory lane; the legacy
optional search lane can use its configured query rewriter. Cancel/fail open
with an honest reason at the shared deadline and record that failure. Measure
the configured deadline with seeded and real installed clients; the 100 ms
target is not a measurement. Optional lane failure cannot discard a completed
local section. The lightweight runtime selects a deterministic query rewriter.

Cache identity includes effective repo/branch/applicability context, prompt,
principal where applicable, and current relevant scope revisions. A successful
capture, supersession, or stale-source update invalidates affected cached advice.
Do not cache a provider/transport failure as successful new memory knowledge.

## Candidate-only Jev

Jev is opt-in advisory enrichment after explicit content is committed. An
individual capture must set `classify=true`; absent/false remains local even
when the provider key is configured. Production sends only that candidate, not
other stored memories. Neighbor contradiction hints are deferred until a
separate consent contract exists, so a local-only record cannot leak through a
later candidate's classification. Pin tested `jev-1.13.0`; include resolved
model, question version, usage and fallback reason
in nonsecret metadata. Ask a few typed questions on one candidate: durability,
kind, suggested scope, and optional contradiction hints. Never apply the
49-question raw-event overlay.

Sanitize the entire transmitted state with existing redaction/security rules.
Allow only the explicitly authorized candidate text; exclude other stored
memories, raw transcripts, tool outputs, `.env` content and unrelated context. Use at most
8 KiB serialized sanitized state and 16 KiB response, one attempt with a 2 s
total deadline, one in-flight request, and at most 100 uncached calls per UTC
day persisted in SQLite. These are conservative initial limits; record usage,
latency, cache hits and actual spend before any adjustment. No retry storm or
silent parameter expansion. Overflow, budget exhaustion, missing key, timeout,
nonfinite/out-of-range probabilities and invalid typed answers select
deterministic `unavailable`/`uncertain` advisory outcomes.

Cache successful judgments by model, question/schema version, sanitized candidate
hash, candidate version and current scope revision. Reserve call budget
transactionally before network access and charge actual reported usage once.
Do not present per-call cent rounding as provider billing. Never log state,
secrets or raw error bodies. Explicit writes remain durable/eligible if Jev says
transient, suggests another scope, returns an error, or is disabled. Any proposed
candidate has no automatic promotion path. No source file or project rule is
written by classifier output. Injection-time Jev is outside this task.

## Integration and runtime

First inventory actual client executables, versions, loaded config, MCP transport,
hook capabilities and daemon state. Claude package SessionStart must await actual
context; test installed binary and shim, with adapter alive. Existing unsupported
governance calls must not add a mandatory nonexistent-route round trip to daily
memory. Do not invent Codex/Grok/Kimi hook support from upstream claims.

For each available Claude/Codex/Grok/Kimi harness, test real MCP capture/read and
observe returned context in its actual tool path. Record model-provider review
separately. If a client executable/session is unavailable, mark that path
unverified/unsupported and record the blocker; a request to a model API is not a
substitute harness test. Shared MCP availability does not imply automatic hook
injection. Document explicit session/prompt read instructions where only MCP is
supported.

Lightweight profile runs local API/storage and the adapters needed by installed
clients. GPU, reranker, bulk embedding/indexing, and archive tailing are optional.
Report missing optional capabilities honestly. No volume deletion, native-file
rewrite or OS scheduler is used to change profiles or sustain processes.

Daily mode performs no recurring complete archive decompression. Optional
archival mode uses append/capture events and durable file/event checkpoints,
bounded batches and duplicate-safe publication. Closed files are not repeatedly
decoded; current compressed tails must handle incomplete frames without advancing
past uncommitted evidence. A one-time explicitly requested historical backfill is
distinct from daily refresh. Expose lag, checkpoint and corruption/retry status.

## Verification

Storage tests run with temporary on-disk SQLite, not only in-memory connections.
Contract tests traverse actual capture/read/feedback routes. Seed exact decisions,
corrections, cross-repo records and distractors; check delivered IDs and versions.
Test simultaneous writers, retry conflicts, stale expected revision, scoped
supersession, source changes, restart, Jev/bulk outages, cache revision, byte/time
bounds and native-file preservation. Required integration jobs fail on absent
dependencies; optional live smoke tests are labeled separately.

Before activation, compare same-host idle CPU, RSS, IO, process count and query
p50/p95 over stated intervals for lightweight and full configurations. No
unmeasured resource reduction or model-compliance guarantee is claimed. Final
review includes actual Sol/Astra, Claude, Kimi and Grok patch responses, with
availability/limitations recorded; consensus never replaces required gates.

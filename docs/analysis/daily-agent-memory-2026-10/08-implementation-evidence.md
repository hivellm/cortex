# Recovery implementation evidence

Date: 2026-10-16. Task: `phase34_daily-agent-memory-recovery`.
This is an execution record, not a claim that the full recovery is complete.

## Section 1.2: stop automatic native-memory rewrites

The local `.cortex/metadata.sqlite` contained an enabled
`retention.memory_consolidate` job using `memory-consolidate --apply`.
Its recorded last successful run was 2026-10-09. A fresh check found zero
running retention sweeps and no host Cortex retention process. The API's
in-process retention loop is a separate automatic execution path.

The operator-authorized safety action changed only that row's `enabled` field
to zero. Schedule, command, outcomes and history were preserved. SHA-256
comparisons before and after the operation matched for all 535 top-level
native-memory Markdown files under the Claude project directories. No native
memory content was repaired, deleted or rewritten.

Claude Opus 5.5, invoked through local 9router, owns scheduler protection and
its regression tests. GPT Sol found that the real API daemon bypassed the
scheduler's `tick`; it owns the API wiring to the shared safe selection helper
and file-backed regression tests. Automatic selection disables and excludes
the retired job even if startup seeding failed or its row was re-enabled.
The existing explicit manual CLI remains available.

The first baseline workspace check failed because pre-existing phase31 config
changes referenced `LawsConfig` without importing it. The coordinator added
only the missing import, preserving those changes. The next
`cargo check --offline --workspace --all-targets -j 4` passed.

GPT Sol's `cargo test -p cortex-api --lib retention_daemon::tests --offline`
passed all six tests, including the two added file-backed daemon regressions.
Claude Opus's `cargo test -p cortex-workers --lib retention::scheduler` passed
all 27 scheduler tests, including fresh defaults, marked legacy rows, repeated
seeding, re-enable protection, explicit manual execution and restart. Final
targeted validation passed:
`cargo check --offline -p cortex-workers -p cortex-api --all-targets -j 4` and
`cargo clippy --offline -p cortex-workers -p cortex-api --all-targets -j 4 -- -D warnings`.
The three changed Rust files also pass `rustfmt --check`; the small config import
unblocks the previously incomplete foreign change without replacing it.

At this section's initial snapshot, live protection was the disabled database
row. The source upgrade still required validation and activation; an old binary's
default seeding could re-enable the job on restart. Section 1.11 below records
the subsequent validated installation and activation.

## Section 1.3: durable local memory

GPT Sol implemented the local projection in `cortex-storage::daily_memory`.
The existing metadata database applies an additive schema with FTS5. Capture,
canonical `DecisionPayload` evidence, scoped revision and retry receipt commit
in one immediate transaction. Repository identities remain exact; globals
require explicit scope and query opt-in. Supersession requires the current
scope revision and an accepted same-scope predecessor. Sources and unknown
origin are retained without synthetic session attribution.

The coordinator added independent public-API tests for simultaneous competing
corrections, retries after correction/restart, slug-colliding repository names
with identical record IDs, and branch/path eligibility before result limits.
Sol added seven unit tests for persistence, Unicode FTS, validation, lifecycle,
stale-state changes and injected transaction rollback. All 135 storage tests
passed, including the 11 new tests. Storage all-target checks and all-feature
clippy with `-D warnings` passed. Astra's independent review found a decomposed
Unicode query-tokenization defect and a loose concurrency assertion. Sol fixed
word extraction using complete Unicode letter/number/mark categories; the
coordinator now requires the losing correction to return `RevisionConflict`.
Portuguese NFC/NFD and vocalized Hebrew regressions pass. After both review
fixes, all-target check, all-feature/all-target clippy with `-D warnings` and all
136 storage tests pass (97 unit, 39 integration; 12 new tests in this item).

Kimi/Grok participated in the earlier architecture review. New requests for
storage tests/review encountered a rejected request, concatenated response
parsing failure, and provider timeouts. They are not counted as completed
implementation review. The four adversarial Rust tests were written and
validated by the coordinator. The router's model-list endpoint remained
available (54 ms); that does not establish inference availability.

## Section 1.4: capture, read and persistent delivery

The coordinator added file-backed prepared-delivery receipts with exact scope,
memory ID, version, source event, corpus revision and rendered byte count.
An immediate transaction rejects stale snapshots, wrong-scope records, duplicate
items and invalid byte/item budgets before creating a receipt. Preparation is
the only claimed stage; it does not prove client receipt or model consumption.
Three new storage integration tests pass, including feedback and receipt
persistence across correction/restart. Storage all-target check and all-feature,
all-target clippy with `-D warnings` pass after this addition. Astra independently
reviewed the receipt transaction without a confirmed storage defect.

Sol added the MCP read tool and compatible capture extensions. Observed MCP
`clientInfo.name` takes precedence over a supplied agent label but remains
client-reported attribution. Bearer authentication remains separate. Queries
default to repository-only scope, five records and 1,536 context bytes. Streaming
HTTP response bytes and the escaped MCP result are both bounded. All 195 MCP
tests pass, including seven new JSON-RPC/HTTP contract tests; all-target check,
all-feature/all-target clippy and targeted rustfmt checks pass.

Claude Opus researched the API integration but produced no patch during roughly
twenty minutes. The coordinator stopped only that task-owned helper process and
reassigned implementation to GPT. This is not counted as a delivered Claude API
implementation. Claude's earlier scheduler contribution and architecture review
remain separate completed contributions.

The API now commits capture locally, returns honest `local_committed` and
`index_complete=false` state, and uses the same configured file-backed database
for memory and feedback. Observed legacy capture kind is retained separately
from canonical Decision evidence; unknown origin/session stays unknown. Daily
feedback requires the receipt's resolved caller and repository access. Legacy
query-ID feedback remains compatible. With access control enabled, unscoped
feedback history requires admin access because its legacy rows carry no
repository authorization metadata. Local access-control-disabled history keeps
its existing anonymous behavior.

Astra found three integration defects and verified their corrections: router
credentials were not wired to the query service in the production constructor;
Argon2 verification ran synchronously on the async executor; MCP capture rejected
mixed-case exact repository identities accepted by the API. The constructor now
wires credentials and principal defaults, authentication runs on bounded blocking
workers, and MCP preserves exact scope identity. Access control disabled retains
local anonymous operation; a supplied invalid Bearer token is always rejected.
Global operations require an authenticated administrator. Authentication admission
is bounded at 100 ms with four concurrent workers and a 2 s operation deadline;
SQLite admission is 100 ms, work deadline 250 ms and lock wait 50 ms starting before
migrations. These endpoint bounds do not yet establish the later 600 ms total
pre-thinking deadline.

Seven API integration tests and four security tests pass, including the actual
production router constructor with separately supplied credentials. Six storage
receipt tests pass, covering independent global revision invalidation, lock wait
before migration, and observed legacy kind across restart/retry. Missing query
fields are rejected; exact record IDs are JSON-quoted, and truncated bodies carry
an explicit marker inside the byte budget. Sol's mandatory
real JSON-RPC-to-HTTP-to-file-backed-SQLite test also passes: exact mixed-case repo,
offline optional lanes, unknown legacy origin, two service restarts, correction,
original retry receipt and feedback linked to the previously prepared version.
It uses an actual ephemeral TCP listener and no environment-dependent skip.
It also verifies feedback history after restart in local mode. It does not
establish installed-harness delivery. The complete MCP suite passes 196 tests;
all-target/all-feature checks for API, MCP and storage pass. The complete API
suite passes 950 tests, with two existing ignored doctests. Legacy external
backend probes are opt-in and are not counted as exercised services. Final
all-target/all-feature check and clippy with `-D warnings` pass for API, MCP and
storage. After the final boxed-error lint fix, targeted API7/security4/storage6
and real MCP E2E1 tests pass again. Targeted rustfmt and diff checks pass. No
installed binary or daemon activation is claimed by these tests.

Kimi returned a concrete receipt review, independently confirming the feedback
authorization boundary. Its suggested extra repo bound was already enforced by
scope-key validation; its suggested rejection of empty global results was not
adopted because a global snapshot revision legitimately tracks an empty queried
corpus. Grok's new receipt-review request timed out after 100 s and is not counted
as a completed implementation review.

The later API review retries also failed to produce usable review: Grok timed
out after 180 s; Fable returned an SSE envelope without answer text and
`finish_reason=length` at an 800-token completion limit. Neither is counted as
a completed review.

## Section 1.5: bounded candidate-only Jev advice

The coordinator implemented the pinned TypeSafe transport and complete-state
sanitization; GPT implemented API orchestration and Sol implemented durable
admission/cache/accounting. Current live TypeSafe API, Choice, Noul, models and
memory-cookbook documentation was rechecked before implementation. The model
remains `jev-1.13.0`; the question/schema version is `daily-memory-advice-v1`.

The API commits explicit capture first, then optionally evaluates three base
questions. The initial provider schema supported up to three same-scope
comparisons; the current production path sends only the explicitly opted-in
candidate and an empty neighbor list. Provider answers can only add advisory
metadata to the response. They cannot change content, status,
scope, source evidence or revision. Ordinary reads and raw event traffic do not
invoke this service. Production startup explicitly enables credential lookup;
shared test/router constructors keep it disabled. The operator's supplied
`TYPESAFE_API_KEY` enables advice, first from process environment, otherwise from
the local ignored `.env`. An explicitly empty process variable disables advice
without falling back to the file. Configured credentials make the provider
available; each capture still requires explicit `classify=true`. Only the named
key is read; it is not part of printable config or state.

The total post-commit advice deadline is 2 s. Sanitized state is limited to
8 KiB, streamed responses to 16 KiB, with one attempt and no redirects/retries.
Reservations commit before network access, allow at most 100 attempts per UTC
day, and share one short lease across database connections. A timeout/crash keeps
its charge and unknown usage. The successful-advice cache is limited to 512
entries and keys model/schema/sanitized content/candidate identity and version/
scope revision. A final immediate transaction verifies candidate evidence and
revision before accepting results or publishing cache. Failure and stale results
retain measured usage when available; unknown usage never means zero billed cost.

Astra independently found gaps in the core redactor for modern OpenAI key
prefixes, Basic/Digest authorization and PGP private-key armor. The candidate
provider's supplemental redaction now covers those forms, TypeSafe tokens,
configured key literals, secret-bearing structured fields and incomplete PEM.
Current production sends only minimal constant-key candidate state; origin,
event/session IDs, references and repository names are omitted. No payload or
raw provider error is logged. Synthetic tests verify originals remain unchanged.
The final Astra review found no remaining concrete blocker in provider typing,
scope isolation, API wiring or transactional cache publication.

Eight file-backed storage admission/accounting tests and twenty hermetic API
provider/orchestration tests pass. API/storage all-target/all-feature checking
passes. The explicit live test also passes its preservation, bounded-call and
cache invariants. Seven API capture/read regressions and four authorization
regressions pass. All-target/all-feature clippy with `-D warnings`, targeted
rustfmt, diff checks and Rulebook task validation pass. Section 1.5 is complete.

### Capped live synthetic sample

Six labeled synthetic candidates exercised the real HTTPS provider through the
post-commit API orchestration and a temporary SQLite database. Each successful
capture was retried once, giving six verified cache hits and zero extra network
calls. No production memory or native agent files were used as sample inputs.

| Case | Expected kind | Returned kind | Durable probability | Capture latency |
| --- | --- | --- | --- | --- |
| English adopted decision | decision | decision | 0.82 | 711 ms |
| Portuguese observed correction | lesson | lesson | 0.85 | 288 ms |
| English build status | transient | transient | 0.03 | 282 ms |
| Portuguese speculation | transient | transient | 0.04 | 289 ms |
| Quoted adversarial instruction | transient | lesson | 0.05 | 298 ms |
| Portuguese adopted decision | decision | decision | 0.52 | 325 ms |

All six requests returned valid typed answers. Kind matched five of six labels;
the adversarial quotation was misclassified as a lesson. The Portuguese adopted
decision's durability was uncertain. This small sample does not calibrate
thresholds or establish general accuracy, and confirms why advice cannot control
capture or lifecycle. All six original captures remained intact and eligible.

Persisted accounting: six reservations, six finished/successful responses, six
known-usage records, zero unknown-usage records, six cache hits, no fallback or
budget denial; 3,775 input and 555 output tokens. At the inspected provider rate
of $0.042 per million input tokens and free output, measured sample cost is
$0.00015855. Sum of recorded enrichment latency was 2,010 ms; the table includes
full capture latency. These numbers are observations, not service guarantees.

## Section 1.6: ordinary-prompt local memory

The shared pre-thinking pipeline requests scoped daily memory before optional
search and before the optional circuit breaker. Completed local context survives
optional failure, an open breaker, and timeout. The local section reserves its
authority reminder before requesting the server's exact prepared context; it
does not truncate that context after its receipt has committed. The section is
limited to five records and 1,536 UTF-8 bytes. Prompt projection retains at most
32 distinct Unicode terms within 4,096 bytes.

The shared deadline is clamped to 600 ms, scope discovery to 100 ms, and local
lookup to 550 ms from entry under the production budget. The original 100 ms
local target is not a hard authentication deadline. Blocking scope discovery,
formatting and API credential verification have bounded admission and run away
from Tokio executor threads. Scope timeout skips retrieval instead of supplying
an accidentally empty scope. Repository identity uses bounded real TOML parsing;
inline comments and literal strings preserve the configured identity. Branch
metadata is bounded and does not spawn Git.

The adapter's prompt/cwd-only cache was removed. API local-context caching uses
effective caller, exact scope, applicability, query, limits and current scope
revisions, with 256 entries. Every hit is reauthorized and commits a new receipt.
Explicit correction and stale-source revision changes invalidate cached context.
The legacy optional query cache also needed principal, live policy, branch and
all response-shaping request fields in its key; authenticated ACL/rate authority
now comes from the resolved principal rather than a caller label. Invalid
supplied credentials fail closed when verification is configured. Typed access
control configuration, rather than a second environment lookup, controls the
HTTP missing-principal gate. Untrusted principal headers cannot satisfy it.

Adapter and MCP transports forward the existing configured API token to both
local memory and optional query. Independent review caught the MCP binary
bootstrap omitting this token despite its library-level E2E supplying one; the
bootstrap now loads and forwards the configured token. Preparation IDs and
revisions are separate from optional query IDs. No receipt asserts that the
model consumed its context. The local lane makes no model call; the legacy
optional lane retains its configured rewriter.

The authenticated debug E2E first delivered its exact memory in 571 ms with one
key. Adding three unrelated keys reproduced a lost-memory failure at 607 ms:
the old credential store ran Argon2 against every row while holding its database
mutex. The store now persists a SHA-256 lookup digest for newly issued random
256-bit tokens and selects the exact row before mandatory Argon2 verification.
Hashing runs outside the connection mutex. The final transaction rereads the
stored Argon2 hash, revocation and role before recording use and returning
authority. Lookup metadata alone cannot authenticate a key. Legacy NULL lookup
rows retain a first-use scan and receive a digest only after verification;
that initial legacy scan can still exceed the injection deadline. Schema
migration is serialized with an immediate transaction. File-backed tests cover
restart, other keys, legacy upgrade, concurrent open, fresh role/revocation from
another connection and deliberately poisoned lookup metadata.

Sol implemented the shared pipeline/transport/scope work. GPT implemented the
server cache and credential/cache isolation fixes. The coordinator implemented
adapter/MCP wiring and real TCP/SQLite scenarios. Astra reviewed scope parsing,
cache boundaries, deadlines, credential lookup and production wiring. Earlier
all-target/all-feature checking passed; final checking/lint/tests are running
after the credential fix. Client tests found a legacy health test expecting the
removed 60-second adapter cache; it now observes only validated server cache hits.
The old consolidation-continuity test used the production daemon, silently
skipped absent data and allowed 20 seconds. Its replacement uses a persisted
synthetic prior-session source, the real archive projection, real local HTTP API
and exact expected consolidation under 600 ms. It does not claim live harness
or model delivery. The new local pipeline's seven tests and two HTTP transport
tests pass. A temporary-directory fixture also needed correction because this
host's home directory is itself a Git repository.

The first parallel API run returned bounded 503 outcomes in cache-eviction and
feedback-authorization tests; both complete targets pass with isolated test
execution. Their initial CPU-heavy credential scans were subsequently removed
for indexed keys. The four-key cold E2E now delivers its exact memory in 586 ms.
One separate security fixture still returned 503 with four concurrent test cases
on this loaded host. Correctness gates therefore run serially, with the original
production deadlines and explicit concurrency tests still enabled. This does
not establish a concurrent latency guarantee; runtime-load acceptance remains
section 1.10. Final all-target/all-feature check and clippy pass. The combined
serial gate passes 1,121 tests across 14 targets; the opt-in real Jev sample is
the one ignored test. This gate includes four library suites, cache/auth/security
contracts, real MCP/TCP/SQLite scenarios, the bounded shared pipeline/transport,
adapter health and the repaired consolidation-continuity fixture. Section 1.6
is complete; installed client activation is not claimed.

### Claude peer checkpoint availability

The previously aligned `cortex-73` session still appeared idle in the installed
Claude CLI inventory. A follow-up through the actual `SendMessage` tool returned
`success: false`, with `No agent named 'cortex-73' is reachable.` Its earlier
agreement in analysis04 remains valid, but this new checkpoint was not delivered.
The earlier bare CLI attempt did not expose the messaging tool, and low-budget
attempts had no receipt; none is counted as communication. Local coordination
and required work continue independently. Claude provider review remains a
separate capability from a reachable peer session.

## Section 1.7: lifecycle and installed-client delivery

Packaged SessionStart now waits for the native hook response. The native shim
also overrides stale `--fire-forget` arguments for context-bearing events. Hook
stdin supplies the actual session and working directory before environment
fallbacks. Configuration, stdin, IPC and warning-marker work share a deadline;
input/configuration/reply sizes are bounded. Successful context is retained if
warning bookkeeping runs out of time. The fixed operator `systemMessage` is
deduplicated across SessionStart and UserPromptSubmit by actual session, including
resume/recovery. There is no notice in memory context. Unavailable/full marker
storage suppresses warnings; a delayed marker worker can suppress an unseen
warning. This is an at-most-once notice, not a guarantee of operator visibility.
The two packaged context shims require the native binary; missing binaries exit
zero with empty JSON rather than use the former unsafe shell transport fallback.

SessionStart reads only explicitly pinned scoped memories with the shared local
pipeline; it has no task query and does not search bulk history. Active-work
metadata runs in parallel with bounded scope discovery, authenticated HTTP,
64-KiB body admission and a 300-ms complete response deadline. Unknown repository
scope skips this metadata query. Whole task rows fit within the remaining total
bundle budget, capped at 4 KiB, without clipping prepared memory context. Ordinary
prompts retain query-matched advice. Structured MCP outcomes and empty prepared
receipts survive empty-bundle errors; integrated paths no longer inject legacy
outage comments. The unsupported PreToolUse governance HTTP call is a no-op,
preserving correlation without implying policy enforcement.

Review found a delivery-blocking publisher mutex: archival HTTP/retries held the
queue lock while a new prompt waited to publish after its memory was ready. A
separate flush lock now serializes bounded batches and retries; queue admission
does not wait for network I/O. Tests hold an ingestion request open while new
events enqueue, and verify FIFO, failed-batch WAL recovery and overflow retention.
No unbounded detached task is added per prompt.

The required subprocess fixtures execute the compiled MCP server and the real
adapter daemon/native hook through packaged Bash scripts, a loopback API and
file-backed SQLite. They check actual JSON transport, configured credentials,
scope/provenance, pinned startup advice, corrections, restart, exact prepared
IDs/versions/events, feedback, outage behavior and notice deduplication. These
are distinct from model-client consumption. A disposable API example and optional
live harness record actual MCP requests/results from installed clients in private
temporary artifacts; their scope is synthetic and classification is disabled.

Claude Code 2.1.291 completed a real MCP capture/read through the new server using
`cc/claude-fable-5-1`: exact ID `probe-618ffcad0bee4a47b3b85d7f5f5191e0`, version 1,
delivery/query ID `01M520H0Y52VA42PH1B5KPNHM2`, exit zero and reported cost
US$0.0894905. This bare invocation proves its explicit MCP tool path, not Claude
lifecycle hook loading. Artifacts: `%TEMP%/cortex-claude-memory-probe-2mhcjoy8`.
Codex's first sandboxed invocation could not resolve its home directory. Automatic
approval rejected a retry involving extraction of a router credential from local
SQLite; that path was stopped. A secret-free elevated inventory succeeded, and
native-client authentication is being exercised without manual credential access.

Codex also completed actual capture/read using its native configured provider
and authentication: ID `probe-bc62d4c5f87e428984435329c55ee369`, version 1,
delivery/query ID `01M520YN58V52J3TSXQP63HPRW`. Its per-tool approval configuration
was explicit for the two synthetic MCP tools. Native configuration merged other
MCP registrations despite an override; no other model tool call was observed.
Artifacts: `%TEMP%/cortex-codex-memory-probe-w2x9m_ru`. Reported usage was 67,598
input tokens (49,536 cached), 363 output tokens (40 reasoning); dollar cost was
unavailable. This is a real tool-path observation, not automatic prompt injection.

Kimi 0.29.2 connected and discovered the two advertised tools, but exited before
capture/read because `kimi/kimi-k3` was not configured in its native model table.
Artifacts: `%TEMP%/cortex-kimi-memory-probe-ykaabeze`. Its model path remains
unverified. Grok Build CLI was absent. At that snapshot, scoped authorization
for the rejected router credential source was pending. The operator subsequently
granted scoped 9router credential access; authorization alone does not establish
that access occurred. Later native source reviews are recorded in
[analysis19](19-final-validation.md), separately from native Kimi/Grok client
availability.

Final all-target/all-feature checking and Clippy with `-D warnings` pass for the
adapter, MCP server and shared pre-thinking crate. Serial Rust gates pass 354
tests across 16 targets, including actual authenticated daemon/package delivery,
stdio restart/receipt tests and stalled-publication regressions. The two Node
installer tests, formatting/diff checks and Rulebook validation pass. Initial
fixture failures (WSL Bash selection, updated sentinel assertions and a too-literal
shell drift anchor) were repaired and rerun successfully.

An actual non-bare Claude Code invocation then loaded explicit temporary
SessionStart settings and executed the packaged Bash hook against the real
adapter and disposable API. Its response included pinned memory
`lifecycle-61ae523a84be47778870cac55b70f623`. The model returned exact nonce
`hookcanary5e704cb0754a40c5b8207268a11eff15`, absent from the user/system prompt and
arguments. Built-in tools and MCP were disabled for that observation, eliminating
a separate retrieval path. Native canonical model `claude-fable-5-1` succeeded;
the earlier router-prefixed model was unavailable on this native route and made
no model turn. Successful run artifacts:
`%TEMP%/cortex-claude-lifecycle-probe-xv6cisys`. Exit zero, reported US$0.01909,
two input tokens plus 886 cache-creation tokens and 27 output tokens. This proves
observed lifecycle context use for the seeded case, not universal compliance.
All probe-owned processes stopped. Section 1.7 is complete, with Kimi's model
path explicitly unverified and Grok's standalone client unavailable. See
analysis09 for versions, capabilities, commands and evidence limits.

## Section 1.8: bounded optional archive processing

The daemon's boot-time whole-archive load and recurring full decompression were
removed. Archive indexing now requires explicit
`CORTEX_ARCHIVE_INCREMENTAL_ENABLED=true` (or the matching ingestion TOML field);
an archive root alone does not start it. Opt-in authorizes an initial bounded
historical baseline, reported separately from subsequent tailing. Daily memory
capture and retrieval do not depend on this worker.

Sol implemented the bounded raw-zstd reader; Astra implemented the durable
SQLite manifest/projection; the implementation agent updated both archive
writers and added API integration tests. The coordinator owns daemon wiring,
typed configuration, publication, status, metrics and forget/cache integration.
The prior Claude session was unreachable; this work does not claim Claude edits.

The reader keeps decoder state between polls and persists a frame boundary plus
complete-line ordinal. Restart may replay the current legacy frame in bounded
steps; completed historical frames are not decoded again. Cursor and projected
evidence commit together. The worker runs outside Tokio's async executor, with
no overlapping polls. Discovery, source reads, decoded bytes, lines, decoder
windows, retained readers and published snapshots have explicit limits. The
100 ms work budget is cooperative between synchronous operations, not a hard
filesystem deadline. The snapshot contains at most 2,000 recent logical events
and eight MiB; raw history remains unchanged.

Both producers now finish and sync an independent zstd frame for each envelope,
in the existing partition files. This bounds future restart replay to individual
events and surfaces finalization/write failures. Compression ratio and per-event
sync throughput can be worse; no throughput improvement is claimed. Open file
handles are bounded. Older unfinished frames remain supported. Complete lines
from legacy live frames can precede validation of the final checksum; later
corruption is reported without claiming that already exposed prefixes were
checksum-verified.

`/v1/status.archive` separates disabled, opening, historical backfill, tailing and
degraded states. Its latest report exposes actual read/decode work, known pending
and partial files, discovery completion and source errors. Metrics count actual
work rather than repeatedly counting the same historical corpus. The projection
is visible to dashboard endpoints and the memory keyword fallback; live Meili
search remains an independent index.

Integration review found two existing obstacles to scoped archive retrieval:
production strategies addressed repository-specific aliases while archive data
was seeded only under shared aliases, and code-anchor deduplication collapsed
distinct events from the same working directory. The archive fallback now maps
scoped aliases, filters repository and query text, and retains distinct event
identities. Snapshot revision changes invalidate optional search cache entries.
Confirmed forget operations persist suppression and serialize it with snapshot
publication, so replay cannot restore forgotten local archive projections.

Review also found decoder eviction could prevent progress when more than eight
legacy frames required replay. Active readers now retain scheduling priority;
tests exercise nine long frames. Discovery excludes quarantine directories and
symlinks/reparse points. Windows source identity uses creation/modified metadata,
length and sampled prefix/anchor hashes; arbitrary same-metadata changes outside
the samples are not claimed detectable. Deduplication guarantees one logical
event in the retained snapshot and checkpoint retry safety; a newly introduced
copy of an event already evicted from the bounded window may enter it again.
This is not an unbounded historical exactly-once ledger. Explicit daily-memory
idempotency remains separate and durable.

The first expanded configuration gate found the section 1.7 emergency-stop hook
read an environment variable outside the typed configuration crate. The narrow
pre-TOML stop check now belongs to `cortex-config`; hook behavior is preserved.
Expanded all-feature worker linting also exposed four pre-existing mechanical
Clippy failures in Claude archive mapping, tailing, walking and its CLI. These
were corrected without changing behavior. The optional search response cache
now retains at most 1,024 responses, prunes expired entries on insertion and
evicts the oldest insertion at capacity, so snapshot revision churn cannot
accumulate responses indefinitely.

All-target/all-feature `cargo check` and Clippy with `-D warnings` pass for the
five affected crates. Storage and API final changes received additional targeted
check/Clippy gates. Relevant Rust gates cover 1,112 distinct passing tests:
configuration 62, storage 169, archive worker unit/integration 85, API library
784 (including two final cache regressions), service security five, and actual
hook delivery seven. One optional live Jev API test remains ignored; it is not
an archive gate. The full API library run covered 782 tests before the final
cache change; 17 cache-focused tests then passed, including the two added tests
and archive publication/forget through the real QueryService. Final storage
index tests cover all 12 cases after the last review fixes.

Logs are under `%TEMP%/cortex-phase34-step18-*.log`. Formatting, diff whitespace
and Rulebook validation pass. Windows linker informational output and the existing
ts-rs attribute parser notices remain; source Clippy gates pass. All owned test
processes exited. Section 1.8 is complete; production activation and real resource
measurements remain in their ordered later sections.

## Section 1.9: lightweight runtime profile

The new typed `runtime.profile` accepts `full` (compatible default) and `daily`,
with normal TOML/environment/CLI precedence and `CORTEX_RUNTIME_PROFILE` support.
It preserves backend URLs, credentials, access control and storage paths. The
daily daemon gates factories before external probes, rather than clearing URLs
that legacy environment fallbacks could silently restore. It suppresses live
vector/Meili/Nexus lanes, Meili refresh, watchers, memory tailing, LLM query
rewriting, reranking, canaries, retention and recurring health monitors. Archive
incremental processing remains a separate explicit opt-in.

The adapter's daily publisher has no queue, HTTP client or WAL. It never opens,
replays or drains the existing overflow file. Bulk event construction, transcript
reads and tool correlation are skipped; session/turn identity and contextual
hooks remain. Adapter health reports the disabled publisher and untouched WAL
separately. Existing raw archives and native memory files are not rewritten.

The API's daily HTTP boundary exposes capture/read/feedback, local query, ACL,
audit, active-work and basic health/metrics. Legacy optional endpoints return
`503 disabled_by_profile` before their handlers can reload configuration or call
backends. This includes extended health and bulk dashboard endpoints. Basic
health/status observe bounded SQLite open/read and report unavailable storage;
disabled optional services are not probed. Readiness explicitly does not claim
that a client consumed prepared context. An unprobed adapter is labeled unprobed.

The manual Windows launcher owns only its two native child processes. It checks
ports, pipe, executable path, PID, creation time, listener ownership and actual
daily-profile health; stale installed binaries cannot pass on generic HTTP 200.
It preserves calling-directory path semantics and inherited configuration/auth,
uses hidden processes, creates no OS schedule and never stops Docker services.
The guide is `docs/operations/daily-memory-runtime.md`.

Astra implemented typed configuration and daemon gates; Sol implemented adapter
changes, and the coordinator implemented API capabilities and the HTTP boundary.
The runtime implementation agent owns the launcher. Native Claude Fable 5.1
reviewed the concrete launcher/profile design, with no credential extraction:
`%TEMP%/cortex-daily-native-review-final-_7tjj6yq/result.json`, reported list cost
US$0.27383. Findings about orphan cleanup, explicit stop order, readiness errors
and per-run log names were addressed. Earlier bare-auth and timed-out attempts
did not produce review evidence. This is a section review, not the final patch
review required by section 1.12.

Current gates pass 964 distinct Rust tests across configuration, API and adapter
library/binary/integration targets (one optional live Jev test ignored). The real
API binary test starts twice, captures and reads an exact decision, persists
feedback across restart, reaches zero configured backend connections, seeds zero
maintenance jobs and leaves an archive-root fixture unprocessed. The complete
test took 1.90 seconds; this is not a production latency measurement. The merged
dashboard test proves optional endpoints are blocked while active-work still
enforces real API-key authentication. The actual packaged hook/daemon test passes
in daily adapter mode and verifies a seeded dormant WAL remains byte-identical.

Check, Clippy `-D warnings`, formatting, workspace configuration audit and Rulebook
validation pass for the current changes. Logs: `%TEMP%/cortex-phase34-step19-*.log`.
The process-test gate first caught two SQLite count types using unsupported
`u64: FromSql`; these were corrected to `i64` before the passing rerun.

The actual isolated Windows PowerShell 5.1 launcher smoke passed in 4.85 seconds:
`%TEMP%/cortex-daily-launcher-smoke-iic12lsi/result.json`. API and adapter remained
alive after the launching shell returned, reported daily health and local SQLite
readiness, and stopped through a separate launcher invocation. Both owned PIDs
and listeners disappeared, the synthetic dormant WAL stayed byte-identical, and
an occupied-port rejection preserved the unrelated listener without spawning
children. The first sandboxed attempt lacked network/process inspection access
and failed safely; the authorized isolated retry exposed a real Windows issue:
`Get-Process.Path` could be empty after the parent shell exited. A limited-query
`QueryFullProcessImageName` fallback fixes identity lookup while retaining exact
PID/path/creation-time checks. All earlier smoke-owned processes were stopped.
PowerShell 5.1 and current-pwsh self-tests also pass. No installed runtime changed.

Daily MCP discovery now uses the same filtered registry for `tools/list`,
`cortex_capabilities` and dispatch: 12 supported tools, with the compatible full
profile retaining 42. Direct calls to unavailable tools receive the existing
JSON-RPC method-not-found response before HTTP. The binary uses the resolved
configuration snapshot; library `Server::new` retains its compatible full
registry. The real stdio test confirms all three surfaces agree and performs
authenticated capture/read against actual SQLite. MCP check, Clippy and 101 tests
pass, bringing this section's passing Rust gates to 1,065 distinct tests. All
owned test processes exited. Section 1.9 is complete. Resource and cross-model
behavioral acceptance remain 1.10; installed activation remains 1.11.

Formatting caveat for the final workspace gate: `cargo fmt --all -- --check`
also reports existing formatting drift in untouched graph, ACL, CLI, evaluation
and worker files. Targeted formatting and whitespace checks for this section pass.
The full-workspace formatting gate is therefore not claimed green; its log is
`%TEMP%/cortex-phase34-step19-fmt.log` and final gate reconciliation belongs to 1.12.

## Section 1.10: measured and behavioral acceptance

The strict concurrent acceptance opens persisted SQLite memory and API keys,
restarts the real HTTP router, and releases four distinct never-verified keys
through a barrier. Each caller traverses the real MCP pre-thinking, TCP,
authorization and storage paths. Optional search deliberately returns 503.
All four return the exact corrected decision, unique persisted prepared receipts,
matching event/version/scope and bounded cited context. Superseded, proposed,
wrong-repository, wrong-branch, wrong-path and unrelated records are excluded.
No failed sample is discarded and no retry or deadline tolerance is used.

Measured end-to-end times were 311.481, 315.511, 330.823 and 352.118 ms, each
below the strict 600 ms limit. All four local HTTP calls returned 200; all four
optional calls returned the injected 503 while preserving the local section.
Check, Clippy with `-D warnings`, and the test pass. Logs are
`%TEMP%/cortex-phase34-step110-acceptance-{check,clippy,test}.log`.
This verifies cold credential concurrency on this host, not a universal latency
guarantee under arbitrary resource contention.

The synthetic fixture now has a guarded `--resume`: an exact ownership marker,
canonical directory and database path must match before reopening. Ordinary
creation still refuses an existing database. Resume preserves repository/config
files and rejects a redirected database. Its test, check, Clippy and rebuild pass;
logs are `%TEMP%/cortex-phase34-step110-fixture-*.log`.

The isolated resource probe passes all 210 authenticated capture/read requests,
with complete cold/warm sequential/concurrent samples in analysis10. Its 60.026 s
idle window observed peak working sets of 20.000 MiB (API) and 13.676 MiB
(adapter), with no counter-visible CPU/IO increase. Optional backend connections,
cron jobs and archive index tables were zero. All 22 owned processes were cleaned
up. A later existing-Docker snapshot found the old API at 10.07 GiB; build,
corpus, workload and memory accounting differ, so no savings percentage or
controlled before/after comparison is claimed.

Analysis11 records two actual native sessions: Claude Fable captured, read and
explicitly superseded a synthetic policy; after a real API process restart,
Codex Sol retrieved only the successor and constructed the exact corrected JSON
action. Random path/header values absent from its prompt matched returned memory.
All observed model tool actions were permitted Cortex calls. Contexts were
457/465/465 bytes and RPC reads 18.142/13.117/24.876 ms. This establishes observed
use in that bounded scenario, separate from the receipts' honest `prepared`
stage. Kimi's missing native model and absent Grok CLI remain explicit limits.
Section 1.10 is complete; installed activation follows in 1.11.

## Section 1.11: local activation

Analysis12 records exact ownership checks and reversible stops of all 12 Cortex
Compose containers. All 29 unrelated containers retained their state, six named
Cortex volumes remain present, and restart policies were unchanged. Several old
services reached Docker's 30-second stop timeout and exited 137; no uniformly
graceful shutdown is claimed. Existing metadata passed `PRAGMA integrity_check`
after the transition. A consistent 22-table pre-activation backup is retained
under `.cortex-state/daily-runtime/metadata.before-activation-verified.sqlite`.
The first sandboxed backup attempt left a zero-byte artifact; it is not a usable
backup and was not overwritten or presented as successful.

Validated binary copies, private configuration and process ownership state now
live under `.cortex-state/daily-runtime/`. API and adapter reuse the operator's
existing `C:/Users/Bolado/.cortex/metadata.sqlite`; startup adds the daily schema
in place, giving 35 total tables including 13 daily-memory tables. A new explicit
local API-key store is configured separately. Its token stays in ignored private
TOML, with a Windows ACL granting the operator, SYSTEM and the creating sandbox
account. Provider authentication is unchanged. The first ACL omitted the actual
operator SID; the client updater stopped before writes, and access was corrected
before applying the reviewed manifest. Unauthenticated memory requests return 401.

Analysis13 records five narrowly updated client files with verified backups and
expected-state guards: Claude user MCP, existing project MCP, Claude user hooks,
Codex MCP and a new Kimi MCP entry. No duplicate Cortex server name or plugin was
added. Claude's contextual hooks are synchronous and explicitly pinned. Codex's
hook definitions/trust are unchanged. Native Claude and Codex report the new MCP
registration; Kimi doctor passes but its model remains unavailable.

Automatic approval review rejected sending three internal decision/lesson
summaries to TypeSafe without payload-specific authorization. No rejected command
ran. The safer capture contract now defaults to `classify=false`; only explicit
true invokes advice after commit. Production also excludes other stored records
from provider state, so a future opted-in candidate cannot disclose a prior
local-only neighbor. A real TCP/mock-provider regression verifies zero calls for
absent/false, one call for true, cached retries, and exclusion of a same-repository
private canary. API/MCP checks and Clippy pass; advice 21, MCP 8, capture 7, cache 6
and security 4 tests pass. One optional live provider test remains ignored.
The first parallel security run returned one 503; the complete serial suite
passed without deadline changes. The failing parallel observation is preserved
in analysis13; its exact internal timeout stage was not established.

After validated API/MCP binaries were recopied and the owned native pair was
restarted, three accepted project memories committed through the authenticated
local API with `classify=false`. Their IDs are `adr039-daily-runtime`,
`daily-value-priority` and `prepared-is-not-consumed`, all version 1. Provider
attempt counts stayed zero. All three installed client configurations then
performed real stdio reads returning exactly those IDs, revision 3 and 1,309 bytes.
Each separate prepared receipt was verified in SQLite. Actual descriptors expose
12 daily tools and the explicit classification flag. These are transport checks,
not Kimi/model-consumption claims.

Native-memory comparison before and after activation remains 535/535 identical
files, with zero additions/removals. Analysis14 distinguishes this retained
manifest from the earlier historical 535-file claim whose manifest was not
located. A real Claude execution with the two project decisions was separately
rejected by automatic approval review before launch. The operator subsequently
authorized those two summaries for native Claude and three named summaries for
Jev; [analysis22](22-authorized-live-validation.md) records the exact scope and
completed validation. Native Claude recalled exact copies of the two authorized
summaries in an isolated repository scope through the actual installed hooks.
Jev classified the three original candidates with cached retries and unchanged
capture receipts. These results do not establish general historical migration
or universal future compliance.
Installed public synthetic acceptance is recorded separately below.

Installed public acceptance now passes (analysis15). The first native run exposed
a real transport failure: Git Bash removed one leading backslash from the
explicit named-pipe argument. A native-argv reproduction established the cause;
the guarded installer now writes the equivalent `//./pipe/...` form only for
Claude hook command arguments. A fresh native Claude session then received the
987-byte context from actual installed user hooks and returned both exact canary
IDs, versions and random markers absent from its prompt. No model tools or MCP
servers were enabled in this check. Both failed and successful runs are retained.

The final native-memory comparison remains 535 identical files. All four pinned
binaries match their build hashes; API and adapter health/status report daily,
unauthenticated and invalid-token reads return 401, and bulk/archive processing
is disabled. Analysis10 separates the final installed 60-second observation from
earlier controlled fixtures. The operator guide highlights the installed paths
and manual restart commands before generic installation examples.

The verified pipe mistake was then saved locally as `windows-claude-hook-pipe`,
version 1, event `01M5287A2N3X9CBFHVH5NFY0RH`, repository revision 4. A matching
local query returned that lesson plus the two pinned decisions in 1,361 bytes,
delivery `01M5288FWVY13H09SD5W3F0AFH`. Classification remained `not_requested`.
Section 1.11 is complete. Independent final reviews are recorded in analyses16-21.
Final quality gates passed with 3,955 tests, zero failures and nine ignored;
[analysis19](19-final-validation.md) records the gates and
[analysis22](22-authorized-live-validation.md) records completed installed
verification, authorized model use and preservation of all 535 native hashes.

A subsequent sequential-hook check exposed intermittent failures in the debug
runtime. [Analysis23](23-release-runtime-validation.md) records the same-source
release correction, 40/40 delivered hooks, repeated native Claude acceptance,
persisted Jev cache and the current 32.031 MiB installed pair. Authentication and
deadlines stayed unchanged; prior debug observations remain historical evidence.

# Standalone audit implementation

## Prerequisite and record contract (section 1.1)

Phase36 is accepted in commits `49d5f63` and `5d3f339`. Installed executable
SHA-256 is `ce68013c14018cdd1df45878f29a2e5750590d6aefca1c33b82f68f023fedb8f`.
Phase28 is accepted in `1590e03` and archived in `467c097`: 131 standalone and
4,100 legacy Rust cases pass; 147 Python cases pass with one platform skip.
The frozen evaluation remains the regression contract. Its passing result does
not establish natural-language readiness or model use.

Repository memory was read through the installed standalone MCP in exact scope
`Cortex`; the older configured connector returned an internal fail-open timeout.
Relevant lessons distinguish receipts from use, semantic tests from wall-clock
deadlines, and direct hook execution from actual client behavior.

Audit is a distinct canonical type, never a memory `RecordKind` or searchable
historical event. It must not change memory/graph counts, FTS results, source
files, existing revisions or classification jobs. Files live in
`local/audit/<sha256(repository)>/<sha256(id)>.json`, using the existing store
lock, immutable publication, pending marker and SQLite recovery boundary.
The SQLite projection receives an explicit schema version upgrade. Hooks never
perform that migration; normal store initialization/repair must prepare it.

Every audit record has schema version 1, exact repository, local audit ID,
recorded UTC timestamp, a tagged payload and a SHA-256 over canonical JSON with
the hash field empty. IDs identify local occurrences, not invented native agent
sessions. Record ID is unique within exact repository scope. Same ID and same
semantic content is an idempotent append; different content conflicts. Generated
record timestamps are excluded from retry comparison; stored bytes never change.

Three payloads are required:

- Delivery: local query and delivery identity, origin, optional observed native
  attribution, query SHA-256, context SHA-256, context byte count, selected and
  omitted counts, exact rendered references, preparation outcome and latency.
  `prepared`, `empty` and `error` describe preparation only, never transport
  completion. Error records contain a bounded code, not raw exception text.
- Observation: optional delivery ID, optional independently captured event
  reference, observed attribution, linkage method, evidence references and
  uncertainty. Observation append never modifies the delivery. Retries can use
  an explicit observation ID; absent IDs generate a new local occurrence.
- Archive control: an explicit bounded list of delivery IDs, cutoff timestamp
  and archive time. It changes default audit visibility only. Canonical files,
  their linked observations and original memory/source data remain preserved.
  This retention operation does not promise disk reclamation.

Delivery and query IDs are local IDs generated per preparation attempt. Native
session, turn, event and agent IDs are separate optional fields, copied only
when actually supplied. CLI/MCP origin is transport-observed; caller-supplied
attribution remains reported attribution, not authentication. Hook origin is the
known client/event pair. No credential, API token or GUI bearer token becomes
an identity. Scope aliases use the existing exact-repository resolver.

Each delivered reference pins repository, ID, revision and content SHA-256 from
the actual `Record` rendered, plus source kind and canonical source SHA-256.
The exact historical memory reference resolves the original source on demand;
audit never duplicates arbitrary source payloads, locators, prompt text or tool
output. This keeps credentials out of the new summaries. Body clipping is
explicit per reference. A selected record omitted by the byte budget is not a
delivered reference. Do not re-resolve current revision while persisting receipt:
another writer may have revised it after selection.

Contract bounds:

| Field or operation | Bound |
| --- | --- |
| Canonical audit JSON | 64 KiB |
| Repository / local or reported ID | Existing scope rule / 256 UTF-8 bytes |
| Origin and method labels | Closed enums |
| Rendered references per delivery | 100, unique exact revision tuples |
| Context | Existing 256..6144-byte budget |
| Source kind | 128 UTF-8 bytes; source body remains outside audit |
| Error / uncertainty | Closed codes; no free-form prompt or error dump |
| Measured latency | Integer microseconds, at most 24 hours |
| List page | 1..100 records, exact scope and optional UTC bounds |
| Metrics population | At most 10,000 newest matching records, explicit truncation |
| Retention preview/apply | At most 100 delivery IDs per command |

Timestamps must parse as UTC RFC3339. Hashes are lowercase 64-character hex.
Reference scope must equal parent scope. Missing historical revisions fail
validation; a changed current revision does not invalidate its retained older
reference. Payload-specific requirements and evidence criteria are finalized
before persistence implementation in section 1.2.

The existing atomic publication guarantees are retained, including the current
Windows directory-sync limitation. Canonical publication precedes index commit
and acknowledgment. Index reconstruction validates audit and memory together
inside the existing transaction, so invalid input retains the working projection.
No background deletion, external service, classification call or OS schedule is
introduced.

## Evidence labels and proof criteria (section 1.2)

Labels describe evidence available in the local record, not causal productivity,
policy compliance, authenticated agent identity or a linear maturity sequence.
The engine derives labels; callers cannot choose a stronger label.

| Label | Required evidence |
| --- | --- |
| `prepared_delivery` | Successful preparation with at least one actually rendered reference; no transport/use claim |
| `observed_related_action` | A retained canonical lifecycle action and one scoped successfully prepared delivery, with matching nonempty observed client/session/turn IDs; linkage is correlation only |
| `captured_citation` | The related-action criteria plus exact full citation `[id@revision content_sha256]` in a retained `Stop` assistant-message body for each claimed delivered reference |
| `unknown` | Empty/error preparation, missing attribution, no delivery, ambiguous candidates, or a bare reported-use assertion |

An observation pins the action's immutable event revision/hash/source fingerprint.
The event must be in the same exact repository, have kind `event`, source kind
`lifecycle_hook`, and event metadata `PostToolUse`, `Stop` or `SubagentStop`.
Its recorded creation time cannot precede the delivery. Correlation requires
matching client, session and turn identifiers from the action metadata and
delivery attribution. Agent identity must also match exactly, including absence;
an observation's supplied event ID must match its canonical action, not the
delivery's different event. Native IDs only identify reported linkage; they are not
authorization credentials. No client/session-only fallback invents a turn.

Exact citation is verified against the canonical `Stop` body, not arbitrary
caller text, a prompt, a tool response or the audit append request. The full
revision/hash must match a reference actually rendered in that delivery. The
record reports method `exact_citation` and verification `canonical_event_citation`:
it proves a captured assistant message cited that revision, not that the memory
caused the answer, was correctly applied or was learned. A bare self-report uses
method `reported_use`, stays `unknown`, and remains counted separately.

Observation methods are `native_turn`, `exact_citation`, `reported_use` and
`unlinked`. Native-turn observations without a complete matching identity are
rejected rather than silently upgraded. Automatic hook linkage considers only
matching scope/client/session/turn/agent: exactly one eligible delivery permits a
correlation link; zero or multiple candidates produce an unlinked observation
with `missing_delivery` or `ambiguous_delivery`. Missing identifiers are retained
as absent and produce `missing_attribution`. Explicit linkage to a foreign
delivery or revision is rejected. Retry identity is scoped and immutable.

Metrics retain separate denominators for attempts, successful prepared receipts,
empty/error receipts, observations, correlations, exact-citation observations,
self-reports and unlinked/unknown observations. Distinct delivery counts with
citations are reported separately from observation counts, so multiple events
cannot inflate a delivery-use ratio. A receipt or adjacent action can never enter
the exact-citation numerator. Bounded-window truncation and omitted population
are explicit; unknowns and errors are never dropped to improve a rate.

Independent review approved these criteria with two explicit invariants:
`exact_citation` requires a nonempty set of unique evidence references, each a
subset of the rendered references and each actually cited. Retry comparison
includes linkage, method, evidence and outcome, excluding only the generated
record timestamp and its derived hash. Automatic observation retries must reuse
the already stored linkage, never recalculate it against later deliveries.

## Canonical persistence checkpoint (section 1.3)

`audit.rs` defines bounded typed requests, source fingerprints and derived labels.
`LocalStore::append_audit` publishes immutable canonical JSON under the existing
exclusive lock and pending marker before SQLite projection/commit; matching
requests return the original record. Audit remains outside FTS, memory counts,
graph edges and classification. `get_audit` requires exact repository scope.

The projection schema is now version 2. Normal opening rebuilds from original
canonical files; only a successful reconstruction commits the new version.
Hooks reject version 1 instead of migrating. Read-only SQLite inspection takes
an explicit expected version: local projection 2, import checkpoint ledger 1.

Six new storage tests pass, including three actual concurrent child processes,
same-ID retries, exact-scope collisions, restart and unchanged source bytes.
A failing SQLite insert trigger reproduces publication before index failure:
the canonical file and pending marker remain, then restart reconstructs it after
the injected failure is removed. This is fault injection, not a claim of testing
every OS power-loss outcome. The complete standalone suite passes 137 unique
cases; seven additional helper subprocess executions account for 144 printed
successes. Formatting, all-target/all-feature check and strict clippy pass.
Independent local review approved the persistence boundary. A source-only Claude
review under the user's existing authorization timed out after 240 seconds without
a result. A narrower source-only retry completed successfully. Claude identified
that native-turn linkage could accept empty/error receipts; both correlation and
exact-citation linkage now require a successfully prepared delivery. Conditional
transaction and missing-directory concerns were reconciled against existing callers
and fresh-store tests. Filesystem guarantees still assume cooperating local writers;
the check/open boundary is not protection against an adversarial local filesystem
race. Neither review input included memory, test logs or operator configuration.

## Reconstruction and validation (section 1.4)

Canonical audit reads now enforce a 64-KiB-plus-one-byte read cap in addition to
checking file metadata; growth between metadata inspection and reading cannot
cause an unbounded read. Errors identify the precise canonical path, including
JSON, path/identity, hash, reference and derived-label failures.

Two recovery tests pass alongside the six storage tests. A receipt for revision
1 retains its exact content/source fingerprints after revision 2, restart,
reindex and repair of a deliberately corrupted SQLite file. Nine invalid-input
forms (JSON, size, hash, label, foreign scope, missing revision, source fingerprint,
timestamp and filename) all fail reconstruction without changing the working
projection; explicit repair also leaves the original database bytes untouched.
The first check exposed a test-only WriteOutcome/Record mismatch, corrected
before the successful execution; its failed check log remains preserved.

## Bounded inspection and retention (section 1.5)

Retention means explicit archival of audit families from the default list, not
physical deletion or disk reclamation. A delivery and its linked observations
remain canonically present and inspectable by exact ID or `include_archived`.
No automatic hook, index rebuild or opening operation archives anything.

Listing requires exact scope, a 1..100 limit, optional inclusive `after` and
exclusive `before` UTC timestamps, optional payload kind and an optional cursor
record ID in the same scope. Order is UTC instant descending, then ID descending;
the next cursor exists only when another matching record exists. The projection
normalizes UTC timestamps to RFC3339 text with nine fractional digits; textual
timestamp formatting must not alter ordering, and sub-microsecond instants remain
distinct. Independent review caught silent truncation in the initial microsecond
projection before release. No unscoped all-repository query is added.

An explicit retention request selects at most 100 currently unarchived deliveries
strictly before its cutoff. Default execution is preview. The plan reports exact
selected IDs, current linked-observation count and distinct historical memory
references affected, while stating that all canonical/source files are retained.
Apply selects and persists its bounded archive control under the same store lock;
it never silently expands beyond its requested limit. New observations linked to
an archived delivery remain in that archived family. Preview and apply counts
describe their respective invocation, not a frozen transaction across commands.

Archive controls contain only exact selected delivery IDs and UTC cutoff. Their
derived projection records archived delivery membership; reindex reconstructs
that membership from immutable controls. Repeated archival cannot delete or
revise a receipt, observation, memory, import or control. Empty selections publish
no control. The generic audit append validator also rejects foreign, missing,
duplicate, non-delivery or too-new archive targets. CLI/MCP/GUI exposure follows
in the later checklist items; this item supplies engine operations and tests.

Seven retention tests pass together with the eight persistence/recovery tests.
They cover equivalent UTC spellings, exact nanosecond ordering/cutoffs, cursor
scope and bounds, default preview, source preservation, bounded archive controls,
family reconstruction and the empty/error linkage regression from Claude.
Formatting, all-target/all-feature check and strict clippy pass. Independent code
and test review approves section 1.5. Schema 2 has only been used in disposable
fixtures; the installed schema 1 store remains unchanged pending final acceptance.

## Actual preparation receipts (section 2.1)

`recall::prepare` serves MCP recall and all three local lifecycle clients. It
retains the previous context bytes, records references only after rendering a
complete citation, and marks a partially rendered body explicitly. Selected
records omitted by the byte budget remain omitted counts, not delivered evidence.
An empty receipt can contain the advisory header/clipping marker but has no
rendered references and always carries the unknown label. Errors have no context.

Successful MCP responses add local `query_id`, optional `delivery_id` and boolean
`audit_persisted` to the original repository/context/consumption fields. These IDs
are not session/turn identifiers. Optional MCP attribution is caller-reported;
JSON-RPC request IDs never become native attribution. Hooks copy bounded observed
IDs and their known client/event origin, preserving absent values. Prompt hashes
refer to the actual bounded/redacted query used by the adapter; oversized hook
tokens retain the prior recent-memory fallback. Audit contains no raw query,
source locator/payload or raw error message.

Latency measures preparation through the receipt draft, excluding receipt write
latency. Preparation failures attempt an error receipt when the scoped store is
available. Transport validation or unavailable stores cannot guarantee a receipt.
Audit persistence failure returns usable MCP context with a null delivery ID and
false acknowledgment; a canonical file may still exist after an interrupted index
commit. Hooks remain fail-open, and failure to capture the later prompt event
cannot retract prepared context. Preparation is not transport completion or use.

Ten new integration tests and six existing transport tests pass. They exercise
actual hooks, exact revisions/hashes, clipping versus omitted citations, missing
IDs, fake JSON-RPC identity, source privacy, preparation failure and publication
fault recovery. Check, formatting and strict clippy pass; independent review
approves. Evaluation validators retain exact original context comparisons and
validate the six-field envelope; 26 Python cases pass. Frozen corpus and policy
remain unchanged.

## Independently captured actions (section 2.2)

`LocalStore::observe_event` compares the complete event against its canonical
historical revision before using its body or attribution. A deterministic local
observation ID hashes scope, event ID/revision, content and source fingerprints.
Under the store lock, an existing observation returns unchanged before any new
candidate selection; later deliveries cannot rewrite its interpretation.

Candidate selection includes archived receipts, filters successfully prepared
deliveries in the exact repository/client/session/turn, excludes receipts newer
than the event, and reads at most two matches. Missing identity, no candidates
and ambiguous candidates produce distinct unknown reasons. It never uses a
recent-100 window that could hide another matching candidate. A unique match
allows correlation; only a canonical Stop body with complete citations permits
the captured-citation label. The evidence list is the exact cited subset of
delivered references, including their original body-clipping flags.

PostToolUse, Stop and SubagentStop hooks observe their event only after successful
canonical capture. Six linkage tests plus eleven existing hook/transport cases
pass, including scope, missing IDs, future/empty receipts, archived families,
historical revisions, ambiguity, immutable retries, forged event bodies and actual
hook invocation. Check, formatting, strict clippy and independent review pass.

## Evidence metrics (section 2.3)

Metrics use exact scope, inclusive/exclusive UTC bounds and explicit archive
visibility. A single SQLite snapshot counts matching records, then streams at
most 10,000 newest records ordered by timestamp/ID. Matching, sampled and omitted
counts and truncation are always explicit. Only the current bounded payload and
bounded ID sets are retained while counting.

Delivery attempts include prepared, empty and error outcomes; unknown deliveries
are empty plus error. Latency reports sample count, minimum, maximum, sum and
mean preparation microseconds. Observation classes are exclusive: related,
captured citation and unknown sum to the observation denominator. Reported
use and missing/ambiguous attribution are separate unknown subcounts.

Distinct cited delivery IDs are separate from citation-observation counts. The
citation-delivery numerator intersects cited IDs with prepared delivery IDs in
the same sampled population; its denominator is sampled prepared deliveries.
A zero denominator yields null, never a misleading zero or a rate based on
unseen receipts. Citation observations whose delivery is outside the sample are
counted separately. The rate measures captured citations, not causal use or
productivity. Three metrics tests, check, formatting, strict clippy and independent
review pass, including repeated citations, time windows without deliveries,
archive visibility and bounded truncation.

## Direct inspection (section 3.1)

CLI `audit list/get/metrics/retain` and MCP `cortex_audit_list/get/metrics/retain`
call the shared engine without HTTP or a provider. List defaults to 20 records,
metrics to 10,000 and retention to 100 deliveries. Exact scope, UTC bounds,
archived visibility, kind and cursor follow the engine contract. CLI get reports
not-found; MCP get returns null, matching its existing memory-get convention.
Retention defaults to preview and requires explicit `--apply` / `apply=true` to
publish an archive control. There is no generic remote audit-append tool.

Two new surface cases and fifteen existing CLI/transport cases pass without
starting services. Invalid fields, null/wrong types, scope leakage, numeric bounds
and preserved canonical bytes are exercised. Check, formatting, strict clippy
and independent review pass. Smoke and operator documentation now cover fifteen
implemented MCP tools; eight installed-smoke unit tests pass.

## Embedded inspection (section 3.2)

Authenticated POST routes `audit/list`, `audit/get` and `audit/metrics` reuse the
same host/origin/bearer guard and bounded engine operations. The GUI provides
evidence counts with denominators, kind/archive filters, 20-record pages,
immutable record details, linked deliveries and exact historical source/action
inspection. Source display requests the pinned revision and checks repository,
ID, revision and content hash before rendering. Text uses DOM textContent only;
source locators are never navigated or executed. Generation counters invalidate
stale scope, selection and source requests. Retention remains CLI/MCP-only.

Nine web integration cases, check, formatting, strict clippy and independent
review pass. Authorized Playwright testing uses only two fictional memories in
an isolated home. Desktop (1440x1000) and mobile (390x844) have no horizontal
overflow. Revision 1 displays after revision 2 exists, malicious markup stays
literal with no injected elements, switching case-sensitive repository clears
old evidence, and a 21-record fixture paginates 20+1 and back. Screenshots and
asserted JSON checks are retained for the final evidence backup. These adapter
fixtures do not establish live model use or productivity.

## Runtime acceptance (section 3.3)

An isolated acceptance run uses actual CLI capture, a separate MCP process for
recall, and a real subsequent CLI status operation. Its observed JSON result is
captured through a direct lifecycle-adapter invocation. No native session/turn
IDs are fabricated: the observation remains unknown with missing_attribution.
This verifies local operation and honest uncertainty, not automatic hook emission
by a client, model consumption or productivity.

Separate CLI processes inspect receipts/metrics, reconstruct the index and inspect
again. Every original source/canonical hash is unchanged, as are receipts and
metrics; historical revision/content fingerprints match the captured memory.
No provider call occurs. The fictional GUI server is stopped and its token is
invalidated. An internal PowerShell Stop-Process failure required verified-process
termination through the .NET process handle; no other process was terminated.

## Final review corrections (section 4.3)

The second authorized Claude review completed against audit, recall and hook
source only. Its attribution findings led to exact optional-agent matching
across delivery/action/observation and canonical validation of supplied action
event IDs. Unknown methods require compatible uncertainty codes. Independent
local review verified that metrics and reconstruction already use transactional
callers, so the conditional snapshot/partial-rebuild findings did not apply.

Before installation, the public label was renamed to `captured_citation` and its
observation counter to `captured_citations`. The previous name implied more than
the verified fact. A citation can originate elsewhere in a conversation; even
with matching native identity it proves neither causal use nor correct application.

Hooks drop redacted or invalid native IDs instead of treating redaction markers
as identity. An invalid supplied agent also removes turn linkage; absent/null
agents retain compatibility with clients that provide no agent identity. Stop
events using turn fallback include agent and body fingerprints, preserving
different responses in one turn while deduplicating identical retries. Explicit
native event IDs retain strict immutable retry semantics. Events without native
identity remain separate local occurrences and cannot acquire verified linkage.

A final authorized source-only Claude review of the corrected audit and hook
modules returned APPROVED with no concrete remaining findings. It specifically
confirmed absent/null versus invalid actors, exact optional-agent matching,
canonical supplied event IDs, citation-only semantics, retry identity and bounds.

## Final Rust and packaging checkpoint (section 4.3, still in progress)

The initial full workspace run recorded 4,279 passes, one failure and nine
pre-existing ignored checks/examples. The failure correctly exposed missing
audit-tool documentation in the embedded plugin memory skills. Both memory
skills and the Claude audit skill/command now document the fifteen-tool surface,
bounded inspection, exact historical references and citation-only evidence.
Plugin manifests, marketplaces, version-contract test and READMEs are aligned
at 0.4.1. Hook definitions did not change.

A later full-suite pass exposed a second version assertion in the legacy
`cortex-mcp-server/tests/hook_launcher.rs` still expecting 0.4.0. That current
plugin-contract assertion now expects 0.4.1. The package-local Claude development
marketplace was also aligned to 0.4.1. It is not an embedded installer asset:
the installer already embeds a separate root marketplace literal at 0.4.1.
These test/development-manifest corrections do not change the release binary
or its accepted hash. The full run's remaining results were still being collected
at this checkpoint; no all-green result is inferred from this correction.

The corrected full standalone rerun passes 180 unique cases; the legacy run
passes 4,100. Phase31 adds 49 Rust cases: 39 original audit/web cases, five
attribution regressions and five hook regressions. These results preserve the
failed initial run as evidence instead of describing it as green.

The final candidate release SHA-256 is
`3959770ba6cd594664993c210f1741f6bee203b93f5060b86693114d1e09ccaf`.
Release/installed acceptance and remaining final gates are still being collected.
Section 4.3 remains unchecked; this checkpoint does not claim installation,
client refresh, publication or task completion.
Independent local review also approved these fixes and their regression coverage.
These are code-review outcomes; runtime acceptance still depends on executed gates.

## Final Python and candidate-runtime checkpoint (section 4.3)

The final Python run executes 162 cases with real CLI/MCP integration enabled
through `CORTEX_EVAL_BINARY` and ResourceWarning treated as an error: 161 pass
and the Windows symlink-privilege case is skipped. CI Ruff checks pass for all
21 configured files. The fresh gate in `scripts/phase31-gate.tmp.json` passes
against release `3959770b`, with frozen corpus and policy unchanged. Natural-
language readiness remains false; all seven answerable held-out questions still
return zero results.

`scripts/phase31-runtime-acceptance-final.tmp.json` passes against the final
candidate: four canonical files, including event segments, retain hashes across
restart/reindex. The actual CLI follow-up captured through the direct adapter
remains unknown/missing_attribution; no native identifiers were invented.

The first final-GUI attempt timed out during initialization while heavy full
all-feature compilation was running. Its failure is preserved in
`scripts/phase31-gui-final-first-attempt.tmp.txt`; no live child remained afterward.
That attempt is not a passing GUI result. A final GUI rerun, installation and
the full workspace all-feature rerun are still pending. Section 4.3 remains open.

## Installed candidate and final GUI checkpoint (section 4.3)

The subsequent final GUI run passes on release `3959770b` at 1440x1000 desktop
and 390x844 mobile. It verifies captured-citation counts and disclaimer, exact
revision 1 inspection after revision 2 exists, literal imported markup with no
injected script/image DOM, and no horizontal overflow. The first report-writing
attempt failed because dynamic import was unavailable in the Playwright runner;
the assertions had passed and the result was repeated and saved through the tool
in `scripts/phase31-gui-final-report.tmp.json`. Desktop/mobile screenshots are
`scripts/phase31-gui-final-desktop.tmp.png` and
`scripts/phase31-gui-final-mobile.tmp.png`. Server PID 2236 was terminated through
CIM after Stop-Process failed; port 16464 is closed and its token invalidated.

The final release is installed at
`C:/Users/Bolado/.cortex/bin/cortex.exe`. Upgrade identity and schema-2 checks in
`scripts/phase31-upgrade-identity.tmp.json` verify the candidate and preserved
CE680 backup under `install-01M56Q4H7H711SGH205ZHRGC8N`. Doctor reports 106,960
records/revisions across 37 repositories: 106,946 events, 14 memories and 1,812
links; required_services is empty. Claude/Codex plugin caches contain 0.4.1, but
the clients still need restarting. Cache refresh does not prove active-client
reload or hook/model use.

`scripts/phase31-installed-historical.tmp.json` confirms the same two historical
events from May 5/May 9 and matching CLI/MCP graph results.
`scripts/phase31-installed-smoke.tmp.json` passes fourteen checks over fifteen
tools in an isolated fictional home, without a model-use claim.

The first all-feature workspace attempt failed a legacy Bash five-second timeout
under load; the identical test binary passed alone in 1.47 seconds. The complete
rerun with two build jobs remains in progress, as does source preservation over
69,634 files. Section 4.3 remains unchecked. Final evidence will be preserved
under `~/.cortex/backups/phase31-audit-3959770b/evidence/`; the paths above are
current staging locations, not claims that this move has already completed.

## Preservation and Windows test-helper correction (section 4.3)

The complete preservation pass now succeeds in
`scripts/phase31-preservation.tmp.json`: all 1,657 canonical files remain
unchanged, and all 69,634 original archive files (319,238,316 bytes) retain their
hashes. Its baseline SHA-256 is
`a47f60f89e099a263dc885b4c5ef73c079496f782a3af159476e6159d4a51173`.
This supersedes the earlier checkpoint's pending-preservation status.

The two-job full-suite rerun encountered NotFound in the audit-retention test's
filesystem snapshot helper. Repetition 2 reproduced it with the exact filename
`INDEX.SQLITE3-SHM.tmp`: an ephemeral Windows SQLite sidecar disappeared during
snapshot traversal. Only the test helper changed. It now excludes derived-index
files case-insensitively at the store root, while a mandatory imported nested
`index.sqlite3-source.jsonl` assertion ensures similarly named source data stays
within preservation checks. This is not a production retention/storage change.

Independent review approves the correction. Formatting, check and strict clippy
pass; thirty repeated retention suites pass. Seven retention cases and the
180-case standalone total are unchanged, as are product code and release hash.
Failed logs remain preserved. The third complete workspace all-feature run is
in progress in `scripts/phase31-workspace-verified.tmp.log`, using
`--workspace --all-features --no-fail-fast -j4`. Section 4.3 remains unchecked
until its complete result and final evidence archival are verified.

Legacy build limitation observed during these gates: `cortex-build/src/lib.rs`
lines 158-159 emit rerun-if-changed for `.git/HEAD` and `.git/refs/heads` relative
to crate directories where those paths do not exist. Cargo therefore repeatedly
marks that build input dirty; `CORTEX_BUILD_TS` also changes at lines 218-219.
This explains repeated compilation work, not a demonstrated standalone regression
or proven cause of a product failure. This task changes neither that module nor
its build behavior and creates no additional backlog from this observation.

## Fourth full-suite result and legacy retention diagnostic (section 4.3)

The fourth full workspace run, `scripts/phase31-workspace-accepted.tmp.log`,
exits 101 with 4,279 passes, one failure and nine ignored checks/examples across
263 targets. Its sole failure is the legacy `retention_canary_full_pipeline`
zero-orphan assertion: tmp_orphans=1. The exact original test binary passes in
isolation and through thirty repetitions. The root cause remains unresolved;
these reruns do not convert the failed full run into a pass.

A test-only diagnostic correction now treats previously ignored compaction/drop
Results as failures and reports temporary paths and metadata. The zero-orphan
assertion is unchanged. Independent review approves the stronger diagnostics;
formatting, check and strict clippy pass. No product fix or inferred root cause
is claimed. The next complete gate, `scripts/phase31-workspace-complete.tmp.log`,
is running in session 4369. Section 4.3 remains unchecked pending its actual
complete result and evidence preservation.

## Sandbox-only gate failures and approved rerun (section 4.3)

`scripts/phase31-workspace-complete.tmp.log` exits 101 with 4,275 passes, five
failures and nine ignored checks/examples across 263 targets. All five failures
are Windows ACL denial while testing fictional API tokens: four legacy setup
cases and one `cortex_cli_install_it` case. This run used the sandbox; it is not
evidence of a product regression. The strengthened legacy retention canary passes
in this full run in 0.77 seconds, which neither establishes the intermittent
orphan's cause nor proves a product correction.

The approved escalated full rerun starts only after the previous Cargo process
finishes, in session 13771, writing
`scripts/phase31-workspace-unsandboxed.tmp.log`. No concurrent Cargo run is used.
Its result is pending; no final acceptance or task closure is claimed.

## Current real-home evidence limit (section 4.3)

The installed CLI's post-upgrade inspection of exact repository `Cortex` reports
26 records/revisions: 16 events, 10 memories and one link. Audit metrics report
matching_records=0, with no prepared-delivery or captured-citation evidence.
`scripts/phase31-live-inspection.tmp.json` preserves this result. These exact-scope
counts are separate from the whole-home 106,960 records across 37 repositories.
The inspection provides no actual-client use evidence for the new audit feature;
fictional fixture acceptance and direct-adapter checks must not be presented as
client consumption in the real home.

## Cache fixture timing and serial full gate (section 4.3)

The approved unsandboxed run exits 101 with 4,278 passes, two failures and nine
ignored checks/examples across 263 targets. Only two `daily_memory_cache_it`
cases fail, returning HTTP 503. The exact original test binary then passes all
six cases in isolation in 2.61 seconds. No root cause is established by that
rerun.

Test-only stabilization applies the existing eviction test's frozen Tokio time
and polling pattern through shared post handling to all six cache cases, bounded
by a ten-second real-time guard. Real SQLite, authentication and assertions remain;
production code and deadline tests are unchanged. Independent review approves
the correction; formatting, check and strict clippy pass.

The current elevated full gate runs
`cargo test --workspace --all-features --no-fail-fast -j4 -- --test-threads=1`
in session 55812, writing `scripts/phase31-workspace-serial.tmp.log`. Serial test
scheduling preserves all tests, including their internal concurrency exercises.
Even a passing result would validate that execution mode, not default parallel
stability. No final result or section 4.3 completion is claimed yet.

## Final acceptance (authoritative section 4.3 closure)

This section supersedes pending/in-progress states in the historical checkpoints
above. Their failures and measurements remain evidence, not discarded attempts.
The elevated full workspace all-feature serial gate in session 55812 exits 0:
4,280 passes, zero failures and nine ignored checks/examples across 263 targets,
comprising 180 standalone and 4,100 legacy cases. The invocation uses
`--test-threads=1`; all cases and internal concurrency exercises remain enabled.
This result does not establish default parallel stability or resolve the cause
of earlier intermittent legacy retention/cache failures.

The parser and aggregate final acceptance pass. Python executes 162 cases with
real CLI/MCP integration and strict ResourceWarning handling: 161 pass and the
Windows symlink-privilege case is skipped. Formatting, type-check, strict clippy
and Ruff pass. Independent local reviews and authorized Claude source review
approve. Fresh evaluation, final desktop/mobile GUI, installed smoke and runtime
acceptance pass for installed release SHA-256
`3959770ba6cd594664993c210f1741f6bee203b93f5060b86693114d1e09ccaf`.

Preservation verifies 1,657 canonical files and 69,634 archive files totaling
319,238,316 bytes, all with unchanged hashes. The evidence move is complete:
`C:/Users/Bolado/.cortex/backups/phase31-audit-3959770b/evidence/` contains
198 files and 51 directories totaling 6,902,784 bytes. Before/after manifests
verify size and SHA-256 equality. All phase31 temporary reports, helpers and
previous failed runs are preserved there; historical `scripts/phase31-*` paths
above identify their former staging locations.

No test-target or Cargo processes remain. GUI test port 16464 is closed and its
temporary token invalidated. Claude/Codex caches contain 0.4.1, but active clients
still require restart; no automatic client reload is claimed. Real-home exact
`Cortex` scope has 26 records and zero audit records at inspection. Fixture and
direct-adapter acceptance do not demonstrate actual-client consumption. Frozen
retrieval readiness remains false with seven answerable held-out misses. Section
4.3 is complete on this evidence, with those operational/measurement limits intact.

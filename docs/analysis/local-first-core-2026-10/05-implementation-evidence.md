# Standalone implementation evidence

Date: 2026-10-17. Task: `phase36_local-first-core`.
This chronological record preserves implementation checkpoints and their limits.
Later entries supersede earlier pending states and installed binary hashes.
Installed Claude and Codex comparisons now show use of two specific memories;
they do not establish general productivity gains. Final acceptance passed:
128 unique standalone and 4,100 legacy tests, zero failures, nine pre-existing
ignored external checks/examples; formatting, check and strict clippy passed.
The final sections retain failed runs, diagnosis and fixture-only corrections.

At closure, temporary evidence moved out of the checkout into
`C:/Users/Bolado/.cortex/backups/phase36-acceptance-ce68013c/`: `evidence/`
preserves 395 files, including the final and failed test logs, migration probes
and model-use comparisons; `workspace-gates/` preserves eight earlier logs.
The one-off installed-acceptance script is retained alongside them. Paths to
`target/phase36-local-evidence/` below identify their original working location.
Source archives, native agent memory and the installed canonical store were
not part of this cleanup. Build outputs remain in their Cargo target directory.

## Section 1.2 - Canonical storage and derived index

Implemented in `crates/cortex-local`: immutable Markdown memory revisions,
bounded immutable NDJSON event segments, canonical JSON hashes, rebuildable
SQLite FTS5/current-revision/typed-edge projections, exact repository scoping,
idempotent captures and expected-revision updates. Raw events remain immutable
observations. Typed supersedes links do not silently change target authority.

An OS file lock serializes cross-process writers/recovery. A durable pending
marker precedes canonical publication; missing, empty or uninitialized indexes
reconstruct from canonical content. Rebuild validates hashes, revision continuity
and immutable provenance inside a transaction. Invalid input retains the prior
index. Newer index schema versions are rejected without rewriting them.

Independent review caught a zero-byte/existing-empty SQLite case that originally
returned an empty projection without rebuilding. The fix writes the recovery
marker before schema initialization and has a passing regression test. A second
review approved the correction. Existing-file collision comparison is bounded.

Verification, using `CARGO_TARGET_DIR=target/phase36-local` and four build jobs:

- `cargo check -p cortex-local` and all-target check passed.
- `cargo fmt -p cortex-local` and strict all-target/all-feature clippy passed.
- `cargo test -p cortex-local --all-features`: 25 cases reported successful
  (5 concurrency, 8 security, 12 storage; two concurrency cases are subprocess
  entry points also exercised by their parent tests).
- Real subprocess writers verified duplicate/conflicting identities and revision
  races. Killing an owned lock-holder verified bounded contention and automatic
  pending recovery without losing the original record.
- Windows junction fixtures executed without skipping and rejected redirects at
  the root, records and segments paths while preserving an external sentinel.
  The separate privileged symbolic-link fixture cannot execute on this host
  (Windows error 1314); its conditional return is not evidence of that coverage.

Known limits: the lock/SQLite timeout bounds contention, not a full automatic
rebuild. Hooks must not synchronously rebuild a large historical store. A corrupt
non-SQLite index errors explicitly; this section does not provide automatic
quarantine/repair of that cache. Original canonical content is never discarded.
Windows file contents are flushed before/after publication; directory fsync is
available through safe standard APIs only on Unix, so power-loss durability on
Windows is not claimed beyond the exercised process-crash recovery.

## Claude coordination

The existing interactive session was discovered as `cortex-80`; a coordination
message reached its inbox (receipt `be6cc3d4-11a4-48bd-9e6e-354122dc3c79`).
Claude subsequently acknowledged ownership in
`04-claude-coordination.md`: the adapter/plugin work remains owned by that
session; standalone core files are owned by this implementation team. The
acknowledgement records 480 adapter/core/MCP tests and the separately reported
broader legacy suite; these do not replace standalone acceptance. Codex live
hook trust remains an operator/harness requirement. A follow-up coordination
message was delivered with receipt `cbeb022a-572e-4af7-b5d5-8fd7b3082cbc`.

Automatic approval review initially rejected exporting the full new crate for
Claude review. The operator explicitly authorized that code disclosure, after
which a read-only `claude-fable-5-1` review was started. That helper produced no
result and was stopped; its replacement used explicit read-only file access to
the authorized crate. Session `6d0bfb2c-8ca9-450a-9e63-6982e91aa5c0` returned
APPROVED with five low-severity observations:

- Directory fsync previously walked above the store root. Publication now bounds
  that walk to the canonical root.
- A conditional integral-float concern does not apply to the existing canonical
  serializer, which preserves serde_json number representation; a regression
  exercises capture/reindex/retry with integral floats.
- Schema DDL and `user_version` assignment previously ran on every connection.
  They now run only during initialization; ordinary-read database-byte stability
  is covered by regression.
- Legitimate symlinked ancestors such as macOS `/var` were rejected. Root
  preparation now resolves those ancestors while still rejecting a redirected
  store root and redirects within its managed subtrees. Windows junction tests
  cover the unprivileged case; Unix-specific behavior remains platform coverage.
- Non-SQLite/corrupt cache headers still fail explicitly. This is the previously
  documented recovery limitation, not silent canonical data loss.

The review verified publication ordering, immutable versions, exact scope,
process locking and separation of advisory metadata from source authority.
Post-review changes passed the section 1.3 full local gates below.

## Section 1.3 - Source-preserving historical import

Daily import reads SQLite through a consistent read-only transaction, preserves
full evidence versions, and uses exact original-scope identities plus explicit
mapping. Destination checkpoints cannot substitute for canonical evidence.
Dry-run validates conflicts and revision chains without publishing or changing
destination files. Its immutable destination-index view is protected by the
existing store lock and rejects pending recovery or nonempty journals. This
mode is never used for a potentially live historical source database.

Eleven daily-import tests pass, including WAL source preservation, bounded
preparation, revision resume, scope mapping, lossless provenance, dry-run
conflicts and future-schema rejection.

Archive import streams zstd NDJSON through TailReader, batches immutable event
segments, preserves original payloads and line hashes, and tracks every source
occurrence/rejection in a rebuildable import ledger. Source fingerprints and
generations detect replacement and verified appends; canonical publication
precedes checkpoints. Only explicit parent-event links create derived-from
edges, within the original exact repository domain. Historical observations
never become accepted advice through import.

Fourteen archive tests pass, including recall across 2,205 events, bounded resume,
300 source files grouped into two segments, dry-run byte stability, changed
sources, corruption, incomplete tails, namespace conflicts and scoped parent
links. Discovery is capped at 100,000 entries per directory, 1,000,000 entries
per invocation and depth 64. `max_files` bounds files requiring decoding;
unchanged sources still undergo fingerprint checks. Hashing uses bounded memory
but has no wall-clock deadline. Daily preparation is capped at 128 MiB source
data and 64 MiB expanded requests; exceeding a cap fails before publication.

All-target check, formatting, strict all-target/all-feature clippy and the full
`cargo test -p cortex-local --all-features` pass: 57 reported cases (1 readonly,
5 concurrency, 14 archive, 11 daily, 10 security, 16 storage). The two subprocess
entry points and privilege-limited Windows symlink cases retain the section 1.2
coverage caveats; both Windows junction tests execute. Independent review
approved import behavior and the shared side-effect-free inspection helper.
The entire workspace gate and installed acceptance remain section 1.7 work.

A separate read-only compatibility check of the actual historical
`metadata.sqlite` found ten records/evidence rows across three scopes, including
six `repo:Cortex` memories. All ten were accepted version-one records (six
decisions, four lessons). Required schema, scope/ID agreement, initial sources,
payload hashes, snapshot/evidence agreement and contiguous history had zero
incompatibilities or orphans. Database and WAL hashes were identical before and
after. No real historical data was imported during this compatibility check.

## Section 1.4 - Advisory Jev classification

Classification uses the documented System One Choice and Noul primitives with
the pinned stable model `jev-1.13.0`. Confidence is validated independently from
the chosen probability. Prepared input contains bounded, redacted title/body and
source kind/authority only; raw provenance, payload, paths and repository identity
are not sent. Its hash binds model, questions and schema, while cache identity
also pins exact repository, record ID, revision and content hash.

Production transport fixes the HTTPS destination, rejects redirects, disables
proxy inheritance, bounds request/response bytes and total response time, and
never logs credentials or error bodies. The test-only constructor accepts literal
loopback endpoints. Typed validation preserves known token usage even when
answers are invalid. No live provider quality claim follows from these tests.

The atomic per-repository queue retries at most five times per explicit cycle,
with bounded backoff and drain budgets. Jobs persist independently of derived
SQLite state; canonical classification events survive rebuild and recover a
crash between result publication and queue acknowledgement. Usage distinguishes
known token totals from attempts with unknown usage. A source changed during
HTTP leaves historical advice pinned to its original revision and marks the job
stale. Missing credentials do not consume attempts. Advice never changes source
kind, authority or revision.

Independent review found and resolved global scan starvation/scope leakage,
missing parent-directory fsync, and outbound redaction gaps for modern API keys
and Basic authentication. Queue admission is explicitly capped at 10,000 entries
per repository, including finished jobs; saturation rejects new jobs instead of
accepting inaccessible work. Synchronous local filesystem work is not preempted
by the network deadline. Windows directory-fsync limitations remain unchanged.

All-target/all-feature check, formatting and strict clippy pass. The full local
suite passes 79 cases: the previous 57 plus four contract, nine provider and nine
queue tests. Provider tests use real loopback HTTP fixtures, including chunked
oversize, timeout, redirects, escaped credentials and malformed typed responses;
none contacted TypeSafe. Independent static review approved the corrected code.

## Section 1.5 - Standalone executable and transports

The new `cortex` executable exposes capture, revise, get, search, recent, graph,
import, reindex/repair, classify, status, doctor, MCP stdio, lifecycle hooks and
the embedded web interface. Home selection is explicit CLI, nonempty CORTEX_HOME,
then the user home. Exact repository identity comes from an explicit argument or
the nearest cortex.toml, stopping at a Git boundary; directory names are never
guessed as identities. Import dry-run neither initializes nor repairs the target.

Hooks have a separate open policy that never creates/rebuilds an index and caps
lock waits at 200 ms. Missing/pending/corrupt state fails open. Captured raw events
remain observations with native identifiers only when supplied; local event IDs
do not invent session/turn identity. Structured secret fields and inline tokens
are redacted before persistence. Replay identity uses native event/tool/turn IDs
only when sufficient; identical events without native identity remain distinct.
MCP advertises eleven implemented tools and bounds newline JSON-RPC input/output.
Prepared context carries exact revision, hash, repository, authority and source,
with an explicit not-observed consumption label.

Memory recall filters raw events and stale/branch/path-restricted candidates in
SQL before the result limit. Hook prompt lookup remains conservative literal AND
search with recent-memory fallback; quality for natural-language queries is a
retained evaluation obligation, not an established benefit. The 6 KiB context
budget preserves UTF-8 lines. Healthy hooks do not spawn children or services.

Explicit repair builds and validates a replacement projection first, copies and
syncs all original SQLite/sidecar/pending bytes before moving any originals, and
retains both forensic copies and moved originals. A simulated crash after the
first rename verifies those bytes remain recoverable. Hook/inspection openings
accept legitimate ancestor aliases while rejecting redirected managed roots.

The web process binds loopback and serves bundled assets without a frontend build
or CDN. All API requests require exact Host/Origin, a random 256-bit session token
and JSON POSTs. CSP, no-store and no-referrer headers apply. Imported content uses
textContent and safe SVG construction. Features cover search, capture, revision,
provenance, historical revisions, typed connections and classification queue
inspection/enqueue; provider calls and filesystem import/repair are not exposed.
Repository selection comes from CLI bootstrap or an explicit UI choice.

All 108 local cases pass: the previous 79 plus one repair-crash unit, five CLI,
seven hook-store, four recovery, six transport and six real-loopback web tests.
The CLI test executes the actual MCP child with a spaced home path and checks
record identity against CLI output with no HTTP service. Full check, format and
strict clippy pass. Independent reviews resolved credential redaction, event
flooding, applicability filtering, descriptor drift and stale UI selection.
Desktop (1440px) and mobile (390px) unauthenticated layouts were visually checked;
the sole favicon 404 was fixed with a bundled SVG and covered by the web tests.
Automatic approval review rejected sending the temporary fixture token to
Playwright; operator authorization was requested. Authenticated browser navigation
was unobserved at this checkpoint, although authenticated API integration tests
passed. The fixture server was shut down and its temporary directory cleaned,
invalidating that token.

Claude peer acknowledged coordination receipt
`73a7609e-7704-4a4f-aa6a-1f9118f3c76f` and released the Codex plugin for section
1.6 in `04-claude-coordination.md`. Its legacy adapter fixes and commits remain
preserved. Plugin trust must be renewed after the standalone switch; earlier
legacy hook delivery does not establish acceptance of the new executable.

## Section 1.6 - Packaging, installation and plugin switch

The default workspace member is cortex-local and its binary is `cortex`.
The old operator binary is now `cortex-legacy`. Its installer/update binary
list and explicit workspace build preserve this separation; old installation
manifests cannot retire the standalone executable or its recovery copies.
Legacy adapter tests continue exercising the retained adapter shims rather
than expecting the standalone plugin to launch a legacy daemon.

`install` copies its own executable and 33 embedded plugin assets, verifies
SHA-256, preserves previous artifacts before publication, and restores them
on an in-process publication failure. Each rename is atomic, but the complete
binary/plugin pair is not crash-atomic; originals remain in backups and rerun
completes publication. `update --binary PATH` validates standalone protocol 1
and runs that candidate's installer, keeping embedded assets with their binary.
Child output and runtime are bounded. `build-info` does not open a store.
Legitimate ancestor aliases work; redirected installation roots are rejected
before creating a store. Reinstallation repairs missing Unix execute bits.

Installed MCP manifests contain the absolute executable and exact CORTEX_HOME,
so they do not need PATH or the source checkout. Packaged source manifests use
PATH. Five lifecycle events use the same CLI; failed launchers emit only `{}`.
Plugin commands, agents and skills advertise the eleven implemented MCP tools.
No native agent memory, service, schedule or unrelated plugin is rewritten.

The release installed at `C:\Users\Bolado\.cortex\bin\cortex.exe` reports
standalone/release=true and no required services. Its SHA-256 is
`795585ea4b13956ba8d8aaba407a5cd63c02582cadcdb30fe89d925bbcf789e6`.
The prior binary remains under
`backups/install-01M5619GZ9TEF26G2M3HH3VB79/cortex.exe`. Official client CLIs
registered `hivellm-cortex-local` and installed Cortex 0.4.0. The old plugin
was disabled in both clients without deleting its cache. The single Codex
configuration change has backup
`backups/codex-config-d87d4af2ef384e16b36e99c7ef93bec7.toml`.

Both installed manifests successfully initialize their real stdio executable,
list eleven tools and answer exact-Cortex status without an HTTP service.
This is transport evidence, not observed model consumption. The current client
still needed to reload and grant trust at this checkpoint; operator confirmation
was pending.
The prior live-checkout/new-plugin versus old-executable mismatch reported by
Claude is resolved by installing the standalone binary and local bundle.

Before and after installation, legacy source hashes remained identical:
metadata.sqlite `FCDD9B2D8C2F18BCFED2A2809B4FC1CFDA768E57DCBB7C080639CF84125E5B02`,
WAL `FFDA117746C68118E2AB8CDD8F970B1D011210C30C59279E24E1D84D522E5CA9`, and
SHM `7BF8A45AE39230543C6B54D4551F4F2E7576924A1F61C686E1078C8F018A2963`.
No historical import has run at this checkpoint.

Validation so far: full local suite 120 cases, then the added CLI junction and
rendered-manifest regressions passed in focused CLI 9/9 and install 7/7 runs;
local all-target check and strict clippy pass. Legacy adapter/MCP focused gates
pass 123 cases (90 library, 16 lifecycle/delivery, 17 launcher/drift), with check,
strict clippy and formatting clean. Windows PATH launcher tests use the Cargo
target temp directory because the restricted runner denies where.exe enumeration
of its external TEMP; real CMD/POSIX discovery assertions remain enabled.
The standalone CI job runs check/clippy/tests on Linux and Windows without services.
Legacy CLI check/strict clippy also pass, with 40 unit and 16 installation tests.
Four Windows ACL cases require the normal user process rather than the restricted
runner; the approved rerun passes without changing ACL behavior. The regression
confirms old manifests preserve both standalone `cortex` and its `.old-*` copies.
The repository's OpenCode default now invokes `cortex mcp` directly and does not
activate the old HTTP lifecycle plugin. That adapter remains an explicit legacy
opt-in; automatic OpenCode lifecycle capture is not claimed for the new default.

Claude independently verified a fresh headless session connects the installed
plugin and calls status. Four observed lifecycle records (UserPromptSubmit,
two PostToolUse and Stop, client claude-code) reached the local store at
2026-10-17 23:00 UTC. Capture is observed; memory consumption is not yet observed
because the historical daily memories have not been imported at this checkpoint.

## Section 1.7 - Live migration and acceptance (in progress)

The real daily source passed a read-only preview and imported ten records with
zero rejection under namespace `legacy-daily-2026-10`. Original repository scopes
are preserved: six memories belong to exact `Cortex`. The installed SessionStart
then emitted 2,867 UTF-8 bytes with five citations. Each cited repository, revision,
hash and source was checked against the installed CLI's historical `get` result.

Claude subsequently ran a fresh-session comparison, documented in
`04-claude-coordination.md`: both sessions had one turn and zero tool calls, with
file/shell/web/agent/MCP retrieval tools denied. With the installed user plugin,
the model answered the Windows pipe-quoting and release-build questions using
two imported lessons and cited their memory IDs. Without user plugins it answered
"sem fonte" for both. This establishes observed use of those specific memories;
it is not a broad retrieval-quality benchmark or evidence of Codex hook acceptance.

The first real archive preview exposed a schema gap missing from synthetic
fixtures: older events have `Bootstrap` instead of `bootstrap`, and lack modern
context fields such as platform and repository. The corrected migration reader
accepts documented legacy variants without changing original JSON or line hashes.
Missing repository identity requires explicit destination `legacy:unscoped`;
cwd is retained as evidence, never silently treated as a repository ID. Both
quarantine directory spellings, including direct source selection inside them,
remain excluded. Counts distinguish quarantined archives from diagnostic files.
Seventeen archive tests, nine CLI tests, check and strict clippy pass; a new
10,000-record real preview has zero rejections. Bulk import was pending at this
checkpoint; the completed passes are recorded below.

Known ledger limitation: files previously imported with rejected records retain
those rejections when unchanged; parser changes do not automatically retry them.
The operator's archive attempts in this session were dry-run, so this limitation
does not affect its first real import. No rejected source bytes are deleted.

The accepted user direction is also captured as `phase36-local-first-direction`
and queued for advisory Jev classification. The existing repository credential
was detected without displaying it. Automatic approval review rejected the live
call because the specific decision payload and TypeSafe destination need explicit
operator authorization; the question was pending and no provider request had run
at this checkpoint. Local provider/queue integration tests were then the only
classification evidence; the later authorized live call is recorded below.

An isolated Codex 0.162.0-alpha.17.2 CLI experiment confirms `plugin add` refreshes
changed assets even at the same package version. Real caches match the installed
source (Codex 8/8 files, Claude 23/23). Update instructions explicitly call for
client cache refresh, restart and renewed trust when hooks change.

The archive compatibility fix was deployed through the installed executable's
own `update --binary` command. At this checkpoint the installed release SHA-256 was
`c494fe3520d33c855115bb7ae03eaca089008190f7bfc6ad50927c5f3fb5b15f`;
the previous standalone executable remains in
`backups/install-01M562K7QMY90QP0W9FBQ7TJY7/cortex.exe`. The complete current
local suite passes 126 cases with zero failures or ignored cases. Check, format
and strict clippy also pass.

Independent source verification compared every archive file against the
pre-import manifest: 69,634 files, 319,238,316 bytes, zero changed, missing,
added or unstable files. This includes quarantined archives and diagnostics;
their bytes remain preserved even though they are excluded from import.
The baseline manifest SHA-256 is
`a47f60f89e099a263dc885b4c5ef73c079496f782a3af159476e6159d4a51173`.
The comparison uses each source file's hash, not serialization equality between
the independently generated manifests.

Final independent static review found no new core blocker. Remaining limits
include literal FTS retrieval, best-effort redaction, preserved original import
payloads, client trust/restart, non-atomic publication of the complete binary
and plugin pair, and unchanged-file rejection checkpoints described above.
The legacy workspace suite initially passed 4,095 cases with four failures and
nine pre-existing ignored examples/live checks. Three auth fixtures included
cold SQLite migration inside the production 250 ms deadline; the scope fixture
walked global temporary-directory ancestry inside its 100 ms deadline. Fixture
corrections and a complete rerun are in progress. Production deadlines remain
unchanged. A raw first legacy API request can still fail open during slow cold
migration; its process-startup test waits for health before exercising capture.

The first full archive pass processed 68,573 active compressed files and 831,275
event occurrences in 439.47 seconds: 102,652 unique events imported and 728,623
duplicates recognized. No decoded record was rejected. The report remained
`complete=false`: three source files hit the reader's 1 MiB line ceiling, and
sixteen ended in an open compressed frame. Independent bounded decoding found
48 valid JSON lines above 1 MiB (largest 2,886,532 bytes), none above 4 MiB.
All sixteen open frames ended at a complete NDJSON line with zero pending line
bytes; all their readable lines were imported. These are different conditions,
not evidence that sixteen source files are corrupt. The explicit larger-line
import bound and clearer accounting are being corrected before final acceptance.
Quarantine remains excluded: 528 archives and 533 diagnostics.

Installed CLI and direct MCP returned identical complete records and provenance
for events from 5 May (exact scope `cortex`) and 9 May (`legacy:unscoped`). The
latter has 102,382 later records in its scope, proving retrieval beyond a recent
window. An imported two-node `derived_from` graph also matched between both
transports. Scope `cortex` remains distinct from current repository ID `Cortex`;
historical records without repository metadata remain explicitly unscoped.
The archive observed date range is 5 May through 16 October 2026. The local
corpus additionally contains native/daily records through 17 October.

An explicit installed `reindex` rebuilt 102,670 revisions/records from 1,591
canonical files in 31.391 seconds. Counts were unchanged; subsequent installed
CLI/MCP history and graph probes preserved the exact same records, source
hashes and relationships. Thirty links refer to parents absent from the imported
corpus; reconstruction reports those dangling links without inventing targets.

After bulk import, twenty sequential installed SessionStart invocations (ten
per Claude/Codex payload adapter) all prepared nonempty context within 6 KiB.
Observed process times were 27.399-243.279 ms, median 29.334 ms. These are local
acceptance samples under the current machine load, not a locked performance
baseline, renewed Codex hook trust, or proof of model use. Claude independently
observed a further cited answer in a fresh no-tools session during bulk import,
as recorded in `04-claude-coordination.md`.

Claude's remaining observability suggestion is tracked by the retained
pre-thinking audit task: empty fail-open hook responses currently do not
distinguish no applicable memory from store failure through local diagnostics.
The new task must preserve bounded, payload-free outcomes and avoid equating
delivery with consumption; section 1.8 documentation will not invent that feature.

### Final migration, provider and browser acceptance

The larger-line fix is installed as SHA-256
`ee513658aba5de54d4835afb7b213dd8bedffbe1eefcb2a38376b79ca8aab820`.
Its predecessor is preserved in
`backups/install-01M563SS5KXA5VM7ZMXK12BYZ6/cortex.exe`. Ordinary archive
readers retain their 1 MiB limit; migration explicitly opts into a 4 MiB ceiling.
Typed reports now distinguish open frames, pending line bytes, decoder errors
and line-limit failures. Independent review approved the change. Eight storage
tail tests and nineteen archive-import tests pass.

Resuming the same namespace imported 4,278 additional unique events from 35,799
occurrences, with 31,521 duplicates and zero rejected records or decoder/line-limit
failures. There are now 106,930 imported archive events across the original
repository identities plus explicit `legacy:unscoped`. A third full source scan
verified 68,573 active files and imported zero records: 68,557 unchanged completed
files and sixteen resumable open frames, zero pending line bytes. `complete=false`
honestly describes those unfinalized source frames; it does not mean recovered
complete lines are absent. No source was rewritten to manufacture completion.
The second and third passes took 49.142 and 28.137 seconds respectively.

After migration, all 69,634 original archive files and the three legacy SQLite
files still match their baseline hashes exactly. Thirty-four historical probes
pass; the old unscoped event now has 106,660 newer events in the same scope.
Full standalone tests pass 128 cases. A Windows test snapshot observed a
disappearing `INDEX.SQLITE3-SHM.tmp` entry; its creator is unknown. The test helper
now retries the entire snapshot only on NotFound, bounded to two seconds, with
no filename filtering and unchanged full-tree equality. Fifty repeated runs
inside and fifty outside the restricted runner pass. No production behavior
changed for that fixture correction.

The operator explicitly authorized the exact technical-decision payload and
TypeSafe endpoint. One real `jev-1.13.0` request completed, using 620 input and
80 output tokens with no unknown usage. Typed advice selected `decision`, with
confidence 1.0 and durable probability 0.8. The original accepted memory remained
byte-for-byte identical. This is one live integration example, not a measured
classification-accuracy benchmark. No credential value was printed or saved in
the evidence report.

The operator also authorized a fresh authenticated browser fixture. Desktop
1440x1000 and mobile 390x844 passed search, capture, revision, historical source
inspection and typed graph navigation through the UI. Two initial fictitious
memories became four memories/six revisions. API responses were HTTP 200, the
console had no errors/warnings, and page/editor layouts had no horizontal overflow.
The fixture server was stopped, its port closed, token invalidated and fixture
removed. No real-store browser session was used.

An environmental privacy finding remains: the browser's Kaspersky integration
sent its initialization endpoint the initial fixture URL, including its temporary
fragment token. Cortex removed the fragment, but browser-injected code had already
observed it. The token is now invalid. Do not claim this host's browser has zero
external traffic or that Cortex headers isolate data from privileged browser
extensions/security integrations. No token or encoded URL is retained here.

The operator confirmed trust in the new Codex hooks. A fresh CLI comparison still
received no memory, and inspection found no Cortex trust entries in that CLI's
configuration. The official interactive hook UI confirmed five new Cortex hooks.
Automatic review rejected global Trust All because it could affect other plugins;
each Cortex hook was instead reviewed and trusted individually through the UI,
leaving the OpenAI plugin unchanged. All five Cortex entries are now persisted.
No trust hash was fabricated and no bypass flag was used. A new controlled
model-use comparison was pending at this checkpoint; its diagnosis and eventual
successful result are recorded below.

### Installed acceptance and final workspace gates

The legacy workspace run before the subsequent shell correction passed 4,100
tests in 235 suites, with zero failures and nine pre-existing ignored live
checks/examples. All-target,
all-feature locked check, formatting and strict clippy passed. Together with
the standalone run this accounts for 4,228 passing cases; the independent
reviewer recounted the legacy log and found no further core blocker.
Generated GUI type files have no content or staged diff; their warnings only
describe future LF-to-CRLF conversion.

The installed historical acceptance probe was repeated against release
`ee513658aba5de54d4835afb7b213dd8bedffbe1eefcb2a38376b79ca8aab820`.
At that snapshot the store held 106,949 records/revisions, 106,938 events,
eleven memories, 1,812 links and 37 exact scopes. CLI/MCP historical get/search
results and the two-node, one-edge graph were identical. The previous report
was retained separately. The source archive's observed UTC range is
2026-05-05T02:02:50.951Z through 2026-10-16T09:29:44.620Z.

At this checkpoint Codex acceptance remained unresolved after persisted trust:
fresh `exec` and interactive TUI sessions answered "sem fonte" to both memory
questions with
zero model tools. The TUI additionally displayed `Hook failed` and
`hook exited with code 1`. Direct launcher execution succeeded, so the next
investigation targeted the actual client hook runtime rather than treating a
prepared response as observed model use. No conclusion about the failing
shell or command had yet been established.

The subsequent process inspection established the cause: Codex
0.162.0-alpha.17.2 launches `commandWindows` through
`pwsh.exe -NoProfile -Command` on this host. A quoted batch path followed by
the event is not a PowerShell invocation. Earlier CMD-only tests were valid
for CMD but did not represent this client's actual shell. The packaged command
now explicitly enters `cmd.exe /d /c call`; regression coverage runs all five
events through both PowerShell and CMD with a real binary and spaced paths,
checking both memory context and idempotent capture. Installed acceptance
still needed repetition after deployment and renewed hook trust at this
checkpoint; that later comparison is recorded below.

The cross-shell regressions passed, followed by the complete standalone suite:
128 unique tests, zero failed or ignored. Four child-process test summaries
are nested inside the concurrency suite and must not be counted again.
All-target/all-feature locked check, formatting and strict clippy pass.
The corrected release is installed as SHA-256
`ce68013c14018cdd1df45878f29a2e5750590d6aefca1c33b82f68f023fedb8f`.
The previous binary and bundle are preserved under
`backups/install-01M565JPPCX698YM9WYFAGHESP/`. Official `codex plugin add`
refreshed the installed 0.4.0 cache; all eight files match the rendered bundle.

### Codex observed use after the shell correction

After renewing only the five Cortex hooks through the official UI, a fresh
Codex session answered both historical questions with the complete expected
memory IDs at revision 1, the exact pipe syntax and the observed 4/12 versus
40/40 results with 550/600 ms limits. It made zero tool calls. Its natural
UserPromptSubmit and Stop events reached the standalone store. A fresh
session with the official invocation-only `--disable hooks` answered
"sem fonte" twice, made zero tool calls and produced zero Cortex events.
The control disables all hooks, including OpenAI's, for that invocation;
persistent plugin/trust settings were unchanged. This comparison supports
observed use of these two memories, not a general productivity improvement.

The attempted `plugins."cortex@hivellm-cortex-local".enabled=false` control
was contaminated: the runtime still listed five active Cortex hooks, wrote
two events and returned the memories. It is excluded from causal evidence.
The verified shell lesson was captured locally as
`phase36-codex-windows-hook-shell@1`, with this file as its source and no
classification request. No predecessor was silently superseded.

The full legacy rerun after the launcher correction exposed four failures:
cache eviction received HTTP 503, two ACL fixtures hit their 100 ms budget,
and a byte-determinism fixture compared a timed-out response with a normal
one. It passed 4,096 cases, failed four and ignored the same nine cases.
The cache response merges several error paths and does not by itself prove
a timeout. The earlier 4,100-case green run remains recorded, but it is not
substituted for this failure. Fixture diagnosis was then ongoing; no production
deadline or assertion has been relaxed to manufacture a passing gate.

Diagnosis reproduced the cache failure in eight concurrent probes using
separate databases. All eight timed out, with zero database-open or operation
errors. Queue wait stayed below 809 microseconds and opening below 10.1 ms;
operation and connection close reached 317.7 and 398.6 ms respectively. Thus
SQLite work, including warm operation and last-connection cleanup, can exceed
the legacy 250 ms wall-clock budget under this host's I/O load. The previous
cold-bootstrap observation is not the only way to reach that limit.

The corrected semantic fixtures control Tokio time while executing real
blocking I/O, with independent wall-clock guards against hangs. The cache
fixture still makes 259 real requests across 256 keys and preserves its
eviction assertions. ACL and byte-determinism fixtures retain their assertions
and now also reject fail-open responses. Production code and deadlines are
unchanged. Dedicated real-time deadline tests remain active, including the
locked-database 503/time-bound/recovery case. Temporary production diagnostics
were removed completely before validation.

Focused gates pass: twelve pre-thinking cases and thirteen API/cache cases,
check, strict clippy and formatting. Eight concurrent corrected cache probes
all pass with the same real database work. The full workspace gate is rerunning
with the corrected fixtures; the failed-run evidence is retained.

The corrected workspace formatting, all-target/all-feature locked check and
strict clippy gates subsequently passed. The full legacy test run is still
compiling and has not yet produced its final result. A read-only installed
`cortex status --all` snapshot at 2026-10-18T00:43:53Z reports 106,958 records
and revisions, 106,946 events, twelve memories, 1,812 links and 37 exact
repository scopes. Counts include ongoing authorized lifecycle capture and
the verified shell lesson; they are not additional imported source events.

That full rerun completed with 4,098 passed, two failed and nine ignored
cases in 235 suites. The corrected cache, ACL and determinism fixtures all
passed. Both remaining failures are in `daily_memory_security_it`: the
receipt-owner and retry-namespace tests received HTTP 503 from ingestion
before reaching their security assertions. They use the same bounded SQLite
execution path as the cache probes, but the generic response does not prove
the exact failure phase for these two requests. This run is retained as
`final-legacy-tests-after-fixtures.log`; it is not an accepted full gate.

Only those two functional security fixtures now control Tokio time, using a
ten-second real-time guard per request. Their real HTTP/SQLite work and all
authorization, receipt-owner and retry-identity assertions remain intact;
there are no retries of failed requests. Independent review approved this
fixture-only change. Formatting, check and strict clippy pass; all four
security cases and seven API cases pass, including the unchanged real-time
deadline test. Eight concurrent executions of the security suite also pass
(32 case executions). The next complete run uses the same workspace selection
and feature graph; the failed runs remain part of this evidence.

The final complete run exited successfully: 4,100 passed, zero failed and
nine pre-existing ignored live checks/examples, with 235 suites and no
filtered cases. Its log is `final-legacy-tests-security-fixture.log`. Together
with the 128 unique standalone tests, the accepted gate totals 4,228 passing
cases. Workspace check, formatting and strict clippy, plus the focused gates
for the final security-fixture change, all pass. Independent review found no
remaining core blocker. The earlier failures and the legacy real-time SQLite
limit remain documented; this result does not claim they never occurred.

The verified fixture lesson was captured through the installed standalone
`cortex_capture_memory` tool as `phase36-semantic-fixture-clock-isolation@1`
in exact scope `Cortex`, with this report as source. No classification was
requested. ADR 041, the rewritten standalone README and the operational
migration/recovery guide complete the documentation contract. Independent
documentation review corrected the queue/checkpoint file formats and limited
manual retry to failed classification jobs; no further blocker remained.

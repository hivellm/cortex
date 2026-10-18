# Standalone evaluation contract

## Accepted prerequisite (section 1.1)

`phase36_local-first-core` completed its implementation and acceptance in
`49d5f63`, then archived in `5d3f339`. Its canonical contract is
`.rulebook/specs/local-core/spec.md`; its checklist is preserved under
`.rulebook/archive/2026-10-18-phase36_local-first-core/`.

Accepted installed binary SHA-256:
`ce68013c14018cdd1df45878f29a2e5750590d6aefca1c33b82f68f023fedb8f`.
It reports standalone runtime, release build, install protocol 1, version 0.1.0
and no required services. Acceptance passed 128 unique standalone plus 4,100
legacy tests, with zero failures and nine pre-existing ignored external checks.
Formatting, locked check and strict clippy passed. Earlier failed runs remain
in `~/.cortex/backups/phase36-acceptance-ce68013c/`.

The evaluation measures the existing literal AND FTS5 query and bounded typed
graph traversal. It does not assume embeddings, semantic queries, shortest-path
tools, legacy database services or automatic feedback ranking. Raw events are
queryable observations; automatic recall accepts only applicable memories.
Repository identity is exact and case sensitive, including for indirect graph
and statistics observations.

The executable exposes `import daily`, `import archive`, `reindex`, `search`,
`get`, `graph`, `status`, stdio `mcp` and lifecycle `hook`. MCP exposes eleven
tools: cortex_search, cortex_query, cortex_get, cortex_graph, cortex_status,
cortex_pre_thinking, cortex_memory_read, cortex_capture_memory, cortex_revise,
cortex_classification_enqueue and cortex_classification_jobs. Fixture setup
may write only isolated temporary homes; installed smoke remains read-only
after that setup, and no evaluation touches the operator's real store.

The current memory pre-read used the actual repository cwd and this task. It
returned no matching memories and reported consumption as not observed; this
is not evidence of an unavailable store or successful model use.

## Corpus review policy (section 1.2)

The versioned corpus lives in `crates/cortex-local/tests/fixtures/eval-v1/`.
All text is authored fictional public fixture content under CC0-1.0, without
operator data, secrets or copied private history. An independent agent defines
relevance judgments before any retrieval is run. Development and held-out
queries are frozen together; held-out means excluded from iterative query or
ranking tuning, not independently sourced real-world validation. Difficult
natural-language queries remain even when literal FTS cannot answer them.

Daily source snapshots/evidence and archive envelopes are materialized as
readable JSON/NDJSON. The compressed archive is committed with its raw source
and exact hashes, so evaluation needs no Python compression dependency.
Corpus identities use namespace `local-eval-v1`. Daily import IDs use scope
`repo:<repository>`; archive IDs use `repo:<JSON-encoded repository>`, including
the quotes, matching the production import contract. Graph-only capture
fixtures use explicit IDs and sources, then the same production index rebuild.

The manifest pins file bytes, original IDs, imported IDs, source hashes and
expected text/authority/link projections. An expected source locator is derived
from the canonical absolute fixture path, not compared to a developer's machine
path. Cross-home equality excludes store-generated timestamps and whole-record
content hashes, which legitimately vary. Within one home, exact canonical
records and hashes must survive reindex unchanged. Raw source files must remain
byte-identical after import, retries and rebuild.

Coverage must include current and superseded decisions, proposed advice,
readable turns and tool output, multiple relevant answers, irrelevant records,
explicit empty expectations, exact/case-colliding repository names and supported
typed graph answers. Query judgments are not changed to make observed scores
pass. Baselines and justified enforcement floors are a separate later step.

Evaluation orchestration will use Python's standard library and the production
executable's CLI/MCP/hook surfaces. It requires no running HTTP service or legacy
backend. Production provider validation remains tested through its Rust API;
live Jev evaluation will be a separate explicitly opted-in path. Reports must
never imply that prepared context or synthetic provider responses prove model
use or real provider quality.

Pre-execution freeze: corpus SHA-256
`dd4c38d60a66f36004aa440a68279adb1043dd72c0723caeef13b510c0ad1521`;
manifest SHA-256
`a661af9be17b6148a19b65fd5568f4787c6b3681d24e2caa3c08005a47b90734`.
The corpus contains 18 memories, eight events, four graph records, twenty
queries (twelve development/eight held-out) and three graph questions.
Independent static review checked relevance, references and scopes without
querying the engine. Corrections before freeze clarified the fictional April
2025 chronology, superseded record types and bidirectional graph traversal.
Proposed records are active search results; irrelevance is distinct from an
inactive-record or scope violation. No result has informed these judgments.

## Construction acceptance (section 1.3)

The standard-library harness builds two independent homes using production daily
and archive imports, captures the four explicit graph records, repeats imports,
reads all thirty identities, then rebuilds indexes. Exact in-home records and
hashes remain identical. Across homes the normalized stable projection hashes to
`87f19e03c31848e09eb283a9c25f05a67c0d35287ada192ae5b9023891c0a543`.
Original source trees retain their bytes. The report records the accepted CE680
binary SHA and build-info, and rejects a changed executable during the run.

Independent review corrected a reread race by materializing validated archive
bytes, and generator publication now uses exclusive creation. Sixteen harness
tests pass, including real CLI construction, provenance corruption, subprocess
timeout, nonzero exit and stdout/stderr overflow. Two generator publication tests
pass; Windows denies symlink creation (WinError 1314), so that platform-specific
test remains explicitly skipped here and runnable on a capable host. No retrieval
results have been measured at this checkpoint.

## Retrieval scoring (section 2.1)

Recall@5 is the count of distinct, valid relevant identities in the first five
returned positions divided by all relevant identities for that query. Thus a
question with eight relevant records can score at most 5/8; the metric is not
silently normalized to the result limit. MRR@10 uses the reciprocal rank of the
first valid relevant result, or zero when none appears in the first ten positions.
Invalid or duplicate positions never shift later records forward. Exact source,
revision, scope and canonical hash validation precedes relevance credit.

Queries with no relevant records have null recall and reciprocal rank, and a
separate empty-result success numerator/denominator. They do not inflate recall
or MRR. Development and held-out aggregates remain separate, alongside the
combined nonempty-query aggregate and per-query intent/identity diagnostics.
Valid but irrelevant records lower answer quality; malformed provenance,
duplicates or foreign records are integrity failures independent of quality.
No threshold or expected result changes follow from observing these scores.

First observed CE680 results: development Recall@5 0.8625, MRR@10 1.0 over ten
answerable queries; held-out Recall@5 and MRR@10 both 0 over seven answerable
queries. Combined scores are 0.5073529411764706 and 0.5882352941176471. Three
empty expectations pass separately and all twenty responses pass integrity.
All seven answerable natural-language held-out questions returned no results.
This is a measured limitation of literal AND retrieval, not successful semantic
recall. Neither corpus nor expected identities changed after observation.

## Scope acceptance (section 2.2)

Sixty-one production CLI and persistent MCP checks alternate Atlas, atlas,
AtlasLab and missing AtlasProduction twice in one process. Full recall context
(including uncited text), identities, revisions, hashes, typed link targets and
six scoped counters are checked against validated fixture evidence. Foreign IDs
return null/empty graph within the requested scope. Prepared context remains
`delivery_consumption: not_observed`.

After quality measurement, five separate fictional controls exercise two ordered
primary hits, the same ID in two foreign repositories, and a dangling link whose
target exists only abroad. Foreign insertion and production reindex leave exact
primary records, scores, order, statistics and graph evidence unchanged. These
temporary controls are excluded from frozen corpus judgments and denominators.
The engine uses fixed title-match scores with scoped recency/ID sorting; no global
BM25 inputs or separate ranking-input inspection API are claimed.

Independent review added tests for foreign text without a citation and inversion
of the two primary results. Fifty-five focused Python tests ran: 54 passed and
one symlink-privilege case was skipped on Windows. Real MCP transport, subprocess
limits and source preservation are included; no Docker/provider request ran.

## Graph and historical evidence contract (section 2.3)

Graph answers require exact expected node identities, directed typed edge sets
and validated record provenance, not equal counts. Traversal expands both ways
while preserving each stored edge's direction. The depth frontier is not expanded
and does not itself imply truncation; node/edge caps can. CLI accepts depth 0..5
and limit 1..200; MCP accepts depth 0..5 and limit 1..100. No shortest-path API is
advertised or inferred. Controls beyond the three frozen graph questions remain
separate from corpus relevance judgments.

All eight historical events must retain readable payload text, original source
identity, line locator and hash when read through CLI and MCP. This proves
inspectability of retrieved evidence; it does not turn a missed natural-language
search into a successful answer. Earlier per-query misses remain in the report.

Acceptance: 49 production graph/history checks pass. Review exposed an evaluator
gap: a self-consistent hash alone did not reject extra foreign text in source,
metadata or historical body. The validator now checks exact Record/Source key
sets, complete metadata derived from frozen raw inputs, and exact deterministic
archive text projection. Optional typed MemoryOrigin fields normalize to null,
as production deserialization does; original raw evidence and hashes are intact.
Five independently authored attacks first reproduced this gap, then passed after
the correction. With real CLI/MCP enabled, the focused suite runs 75 tests: 74
pass and only the Windows symlink-privilege case is skipped. No relevance judgment,
fixture byte or observed quality score changed.

## Classification evaluation policy (section 2.4)

Offline fixtures exercise the production typed provider decoder and durable
queue using loopback HTTP. Their labels and synthetic responses measure the
scorer/authority contract, not Jev accuracy. Unknown, transient failures and
invalid responses remain explicit. Classification advice cannot change source
authority, content or provenance.

Live evaluation is separate and opt-in. It constructs public cases in fresh
temporary homes, previews the exact production request, verifies its hash against
the prepared job and requires explicit acknowledgement of the payload hash before
using a credential. Each case gets at most one drain attempt, with at most six
cases, ten-second provider timeout and bounded process lifetime. Live reports
retain failures and missing usage without substituting synthetic responses.

Official TypeSafe docs reviewed on 2026-10-18:
[API](https://docs.typesafe.ai/api),
[Choice](https://docs.typesafe.ai/primitives/choice),
[memory cookbook](https://docs.typesafe.ai/cookbooks/basic_memory), and
[models/pricing](https://docs.typesafe.ai/models).
The pinned `jev-1.13.0` model costs USD 0.042 per million input tokens, with free
output and default 64k per-request context. Using 65,536 tokens conservatively
reserves USD 0.002752512 per request, or USD 0.016515072 for six. No long-context
header is sent. The live guard checks the full reservation against the explicit
budget before sending; missing usage retains the reservation. Price assumptions
expire after thirty days and must be reviewed again. This is a bound under the
documented model/pricing contract, not a provider-native billing cap or a claim
that a timed-out request costs zero. Model/schema/usage and policy assumptions
must accompany any observed quality result. No new live call is authorized or
required by offline task acceptance.

Pre-live classification fixture freeze (LF normalization before publication,
without changing JSON values or judgments): cases SHA-256
`271b987d355e90fec20d38cb6068b70badae65c67998364d51c92dc608c453c6`;
questions SHA-256
`d5d0b53ea0e8a00cd4b333b1e0e3a25096de6e720daa1f79ddf5ff46a7e2edf3`.
Six CC0 cases include one expected unknown and two deliberate synthetic errors;
mocked accuracy is 4/6, with two unknown outputs and synthetic usage of 78 input
and 30 output tokens. Three production Rust tests pass through loopback transport
and queue persistence, including malformed responses, 429 and timeout. Check,
strict clippy and formatting pass. These observations establish contract behavior
only; no external provider quality measurement occurred in this evaluation.

Independent final static review approved the offline/live separation, authority
checks, request identity, bounded attempts and failure accounting. The integrated
Python suite executed 87 tests: 86 passed and one Windows symlink privilege case
was skipped. Strict Python lint passed. The real six-case production preview
performed zero provider attempts; its reviewed-payload candidate SHA-256 is
`76a18d81367ecc6a83b5a2a256638a748cfd73a77e06c3f3be3d90a4047fc094`.
That preview is not authorization to send it or evidence of provider accuracy.

## Latency measurement contract (section 3.1)

Latency is end-to-end elapsed wall time from the evaluator, using Python's
monotonic high-resolution counter. CLI and hooks start a subprocess for each
invocation. Direct MCP cold measurements include process start, initialize and
the first tool response; warm measurements use one initialized process after an
excluded warmup. The corpus is already imported in every case. Process-cold
never means a flushed OS page cache, cold disk, or first installation.

Samples run sequentially with concurrency one, bounded sample counts and process
deadlines. Reports preserve individual successful and failed samples, warmup
failures, successful-sample p50/p95 using nearest rank, and total attempted versus
successful counts. Failed/time-out samples cannot be converted into fast success
or silently removed from denominators. Semantic checks must reject hooks that
fail open to an empty object when fixture memories should be delivered.

CLI/MCP search results must retain exact scope and provenance. Hook preparation
must match scoped fixture context, and is reported as prepared delivery with
consumption not observed. SessionStart and UserPromptSubmit are distinct: the
latter writes a fictional event in the isolated home, so that write belongs to
its end-to-end duration. All source files remain unchanged. Reports identify the
binary hash/build, frozen corpus, OS, CPU, Python version and UTC run time. These
small-fixture measurements are not throughput or historical-store scale claims.

Acceptance on Windows 11 build 26200, AMD Ryzen 9 7950X3D (32 logical CPUs),
Python 3.13.9, accepted CE680 release: 20 samples per series, concurrency one,
100 successes and zero failures/unmeasured. Final p50/p95 milliseconds: CLI
26.58/34.09; new-process MCP 24.89/26.12; persistent MCP 3.85/4.47; SessionStart
29.58/38.64; UserPromptSubmit 53.16/55.02. A first full measurement before CPU
metadata fallback is also retained, with p95 37.46/32.13/5.16/39.90/58.08.
Metadata reads CPU model/architecture only, never hostname or device serial.

Fifteen unit/adversarial tests pass and independent review approved the sample
recount. Review first reproduced loss of completed persistent-MCP samples after
cleanup failure. Cleanup now runs outside the timer and preserves completed
samples while failing the series. Fail-open hook output, warmup failures and
canonical-record changes also fail acceptance without discarding prior series.

## Baseline and enforcement policy (section 3.2)

`crates/cortex-local/baselines/local-eval-v1.json` freezes the observed query
scores, split denominators, exact check names, fixture identities and measured
latency before enforcement is connected. It is a reviewed policy, never a
downloaded latest-success artifact or automatically refreshed baseline. Candidate
binary hashes may differ from the historical CE680 baseline, but the quality and
latency reports must identify the same candidate build and frozen corpus.

The blocking quality rule is no loss on any fixed query or split, plus zero
integrity, scope, graph, provenance or empty-expectation failures. Development
floors are Recall@5 0.8625 and MRR@10 1.0; combined floors are
0.5073529411764706 and 0.5882352941176471. All seven natural-language held-out
questions currently have zero credit. Their zero baseline cannot detect further
loss or establish useful natural-language retrieval. Reports therefore separately
expose `natural_language_readiness`, with initial targets of held-out Recall@5
and MRR@10 at least 0.5. These targets are not met. A green contract/non-regression
gate does not mean those targets or observed agent-use acceptance passed.

Latency enforcement requires all five named series, at least twenty sequential
samples each, no failed or missing attempts, no setup/cleanup errors and valid
nearest-rank calculations. P95 must not exceed two seconds for subprocess-based
CLI/MCP/hooks, or 500 milliseconds for persistent MCP. These broad bounds guard
against hangs and severe responsiveness regressions on heterogeneous CI hosts;
they do not detect modest slowdowns relative to the 4-55ms local observations.
They are explicit standalone limits, not inherited service-stack thresholds,
ratios against a developer workstation, production SLOs or large-store targets.

Enforcement recomputes scores, denominators, sample counts and percentiles from
individual evidence rather than trusting summary success flags. Missing,
duplicate or malformed questions/checks/series fail closed. Failed reports remain
available with their diagnostics. Live provider quality, price and credentials
remain outside this offline policy; synthetic classification accuracy is not a
provider-quality floor. Future floor changes need review, justification and
retention of prior failure evidence.

Policy SHA-256 at acceptance:
`7a82c121075cf0f0c2a6be1ebc7115044fc77c80055e76af7e563efb6a65dcc5`.
Independent review reproduced six false-success gaps involving absent source
snapshots, wrong archive hashes, altered latency scope/query and explicit errors
masked by summary flags. Corrections also reject normalized source-path
collisions. Twenty gate tests pass, including partial-report preservation when a
later evaluator fails. A fresh production run passes all three gate checks and
explicitly reports `natural_language_ready=false`. A deliberate historical-report
mutation to 2.1-second CLI samples retains valid counts/percentiles but exits 1
for the latency ceiling, preserving the quality report and every sample in the
failed report. This is a fault-injection proof, not an observed engine slowdown.

## Explicit and CI execution (section 3.3)

`python scripts/gate_local_memory.py --binary <release-cortex> --report
scripts/eval-report.json` runs fresh isolated construction, quality, scope, graph
and twenty-sample latency evaluation. CLI output prints contract/non-regression
and natural-language readiness separately. Supplied historical reports require an
explicit pair of flags, are labeled assessed claims rather than fresh execution,
and cannot serve as CI defaults or silently update policy.

`.github/workflows/eval.yml` replaces its legacy scheduled service-stack jobs
with push, pull-request and manual execution on Ubuntu and Windows. It builds the
standalone release, runs typed classification tests against an ephemeral loopback
fixture only, enables real-binary Python integration tests, then runs the fresh
gate. No Docker service, running application HTTP server, provider credential or
live classification command is configured. Build tools/dependency acquisition
still need normal CI package access. Produced reports upload with `always()` and
ninety-day retention, including gate failures; a failed build may produce no
evaluation report and remains visible in job logs. No successful artifact becomes
the next baseline automatically.

Eight configuration/mutation tests pass and independent review approved trigger,
platform, release, default twenty-sample policy, fresh execution, no-bypass and
failed-artifact contracts. Local Python and real-binary execution validate the
commands; GitHub-hosted runs are not claimed before these changes are pushed.
The initial workflow-test parser mistook a lint file argument for an executable
gate command; exact command-position parsing corrected that test failure.

## Installed surface acceptance (section 3.4)

`scripts/smoke_installed_local.py` compares explicitly supplied installed and
reference executable hashes/build-info before and after an isolated fixture run.
It checks all eleven tool names against the operations table, actual Rust dispatch
arms, CLI descriptors and persistent MCP descriptors. The eight read-only tools
run across Atlas, atlas, AtlasLab and absent AtlasProduction, comparing scoped
counts, complete search/get provenance, typed graph results and cited recall.
The three mutation tools are inspected but never invoked during the read phase.

Only setup imports/captures fictional data in a new temporary home. After setup,
SessionStart is the only invoked hook because UserPromptSubmit publishes an event
and was measured separately in section 3.1. Canonical records/segments, queue JSON
and fixture source bytes must remain identical; derived SQLite/lock files are
excluded from byte equality because opening readers may update those files.

Final installed CE680 smoke passes fourteen checks, including matching executable
hashes, eight exercised read-only tools and unchanged twenty-three canonical
files. Independent review corrected graph equality to validate both CLI and MCP
separately, rejecting duplicate nodes and integer zero in place of Boolean false.
Thirty-one installed/graph/workflow tests pass. No real home was opened, no
provider request ran; prepared hook context does not establish observed model use.

## Final coverage review (section 4.2)

The coverage audit added five real-child-process hook tests for excessive stderr,
nonzero exit despite valid JSON, malformed/multiple/duplicate-key JSON, oversized
input rejected before spawn and removal of TYPESAFE_API_KEY from the child
environment. Existing tests already cover timeout and stdout overflow. Child
processes are reaped in every tested outcome.

Nine malformed benchmark metadata cases initially passed report assessment.
Admission now requires a named OS/Python, explicit CPU/architecture availability,
and ordered UTC timestamps; unavailable hardware may be null. Boolean CPU counts
and Boolean/NaN/infinite metrics are rejected. Twenty-eight focused gate and
transport tests pass, all evaluation Python lint passes, and reassessment of the
unchanged actual measurements still passes non-regression with natural-language
readiness false. No metric floor, fixture judgment or observed sample changed.

## Final acceptance history (section 4.3)

Workspace formatting, locked all-target/all-feature check and strict clippy pass.
The standalone all-feature suite passes 131 unique tests (135 printed successes
include four additional child executions of the subprocess-writer helper).

The first complete Python run executed 146 tests with one Windows symlink
privilege skip and one cleanup error: WinError 145 removing the real-MCP test's
temporary directory after its functional assertions completed. The failed log
is retained. Inspection found the transport unconditionally killed even a
healthy server at close. A deterministic regression first demonstrated that this
interrupts a child's cleanup after stdin EOF. Close now sends EOF, permits up to
one second of graceful shutdown, and retains bounded kill/wait fallback. Reader
EOF is tolerated only during expected closure; unexpected EOF and output limits
remain errors. Twenty-two transport/latency-adversarial tests pass after the fix.

Independent review approved the lifecycle correction. It is a plausible
contributor to the Windows cleanup failure, not proof that delayed filesystem
deletion or external filtering cannot also cause WinError 145. No cleanup error
is ignored, no retry hides it, and the original full-run failure remains evidence.
The complete Python rerun executes 148 cases: 147 pass, with one Windows symlink
privilege skip and no errors. A fresh gate using the final transport passes all
contract/non-regression checks while natural-language readiness remains false.
The repeated installed smoke passes all fourteen checks in an isolated home.

The first legacy run used default features and completed with 4,085 passing,
five failing and eight ignored cases. All five failures concern Windows
current-user credential ACLs: the restricted sandbox loses access after the
fixture grants access only to the actual user. No permission policy was relaxed.
The failed log is retained. The complete locked all-feature rerun outside the
sandbox exits zero: 4,100 passing cases, zero failures and nine pre-existing
ignored live checks/examples across 235 targets. All five credential ACL tests
pass without code or permission-policy changes. Together with 131 unique
standalone cases, final Rust acceptance is 4,231 passes. Python acceptance is
separate: 147 passes and one Windows symlink privilege skip. Formatting, workspace
all-target/all-feature check, strict clippy and evaluation Ruff check/format pass.

Final standalone evidence is preserved under
`~/.cortex/backups/phase28-evaluation-ce68013c/`; batch manifests verify each
move without overwriting older reports. Final gate SHA-256 is
`8a5ebf313e7016896a40f11824b9c6384a3d8f40dafd2db84a2a2062c29c2197`;
final installed smoke SHA-256 is
`ec535e3d635d06e702eb9b4fc5e4d0c31fbd9bde70a4920e048f99699da88db7`.
Independent review confirmed both reports and the complete Python result match
this record. No additional provider request was made.

Implementation checkpoint: `1590e03`. The accepted task is archived at
`.rulebook/archive/2026-10-18-phase28_retrieval-eval-gate-live/`; its reviewed
requirements are preserved as `.rulebook/specs/eval/spec.md`. Rulebook moved the
task but did not emit that live spec, so the archive tail copied the accepted
requirements unchanged. All twenty-seven machine-specific reports/logs remain
in the verified backup across three batch manifests, including failed runs.

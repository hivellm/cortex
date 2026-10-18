# Local Feedback and Ranking Design

## 1.1 Verified prerequisites

Verified on 2026-10-18 before implementation. Phase36, phase28 and phase31
are completed and archived; their checklists contain no unchecked items.
Phase31 implementation is `5689d8d`, archive is `e5f3ca8`.
Installed standalone executable SHA256 is
`3959770ba6cd594664993c210f1741f6bee203b93f5060b86693114d1e09ccaf`.
It exposes fifteen MCP tools, including four audit inspection/control tools.
CLI, MCP and embedded web use the same local engine without required services.

Canonical JSON memory revisions and audit records are authoritative; SQLite
schema 2 is disposable. Audit records are limited to 64 KiB. Recall returns
repository, context, query_id, delivery_id, audit_persisted and
delivery_consumption. Consumption remains `not_observed`. Delivery references
contain exact repository, ID, revision, content/source hashes and clipping state.
Query text and prepared context are represented by hashes in delivery receipts.

Current search uses quoted-token AND matching, not BM25/RRF. Full query text in
the title scores 2.0, otherwise 1.0. Ties use observed time descending, then ID.
Limits are 1..100; recall excludes events and respects active/applicable state.

Frozen evaluation inputs (must not change):

| File under crates/cortex-local | SHA256 |
| --- | --- |
| tests/fixtures/eval-v1/corpus.json | dd4c38d60a66f36004aa440a68279adb1043dd72c0723caeef13b510c0ad1521 |
| tests/fixtures/eval-v1/manifest.json | a661af9be17b6148a19b65fd5568f4787c6b3681d24e2caa3c08005a47b90734 |
| baselines/local-eval-v1.json | 7a82c121075cf0f0c2a6be1ebc7115044fc77c80055e76af7e563efb6a65dcc5 |

Corpus contains 18 memories, eight fictional events, four graph records and
20 queries (12 development/eight held-out). Development Recall@5 is 0.8625,
MRR@10 is 1. Seven answerable natural-language held-out queries return zero.
`natural_language_ready=false`; no feedback-count or synthetic-test result may
be described as real productivity evidence. Held-out judgments are never used
to create feedback or tune policy.

Legacy metadata.sqlite has schema version 1 with additive
`pre_thinking_feedback(query_id PRIMARY KEY, intent, helpful, files_cited,
rating, free_text, implicit_score, recorded_at)`. Upsert overwrites prior values.
Rows lack repository, exact record revision/hash, separate delivery ID, actor,
source, native action/session/turn and explicit-versus-inferred provenance.
`files_cited` are paths, not memory references. Optional matching
daily_memory_deliveries can establish prepared bundle correlation only.
Standalone import currently preserves daily memories/evidence, not feedback.
Legacy detect_citation has no production callers; caller-supplied implicit_score
is not observed model use. Unknown attribution must remain unknown.

Sources: crates/cortex-storage/src/metadata.rs, crates/cortex-api/src/feedback.rs,
crates/cortex-storage/src/daily_memory/delivery.rs,
crates/cortex-local/src/{audit,index,recall,model}.rs and import/daily.rs.
Verification artifact: scripts/phase32-prerequisites.tmp.json (sealed with
acceptance evidence). Phase32 memory read confirmed accepted lesson
phase31-citation-evidence-is-not-causal-use@1; capture and read receipts are in
scripts/phase32-memory.tmp.json. No provider call was needed.

## 1.2 Attribution and immutable identity

Feedback has its own canonical JSON files and rebuildable index projection.
An online request requires a stable caller-selected ID, exact repository,
source kind/reference/actor, evidence (`explicit` or `inferred`), judgment
(`helpful`, `unhelpful`, `unknown`), query ID and delivery ID. An optional exact
AuditReference makes it record-specific. Source actor is declared provenance,
not authentication or proof of independent human votes. Source strings are
bounded, retained unchanged and never interpreted as instructions.

The stored delivery must belong to the exact scope and its query ID must match.
A record-specific reference must match all rendered reference fields, including
clipping, and the canonical historical revision; delivery must be Prepared.
Bundle feedback, unknown judgments, inferred evidence, absent citations and
temporal correlation are never eligible record judgments. Jev is not consulted.
Archived receipts still exist and validate: archival affects visibility only.

Server generates recorded_at once. Same ID and identical request returns the
original record; changed request conflicts. Immutable content hash covers the
whole record except the hash itself. No revisions, overwrite, silent latest-win
policy or feedback deletion. Maximum canonical feedback size is 64 KiB.
Logical groups use repository + query_id + exact record revision/hash. Changing
feedback IDs, delivery IDs or declared actor does not amplify a query's vote.
Conflicting helpful/unhelpful judgments invalidate that group while retaining
all sources. Multiple explicit queries are samples, not authenticated independent
people. Future record revisions do not inherit older revision feedback.

Legacy import reads original database bytes without opening it in SQLite; SQLite
opens only a private copied snapshot with READ_ONLY and query_only. Preserve all
eight original row fields, source path and canonical raw-row hash; deterministic
identity hashes canonical source path + raw row, so unrelated database changes
do not duplicate the occurrence. A changed legacy row is a new preserved snapshot.
Its evidence is always `legacy_unknown`, under reserved repository
`legacy-feedback-unattributed`; online requests reject that scope and evidence.
Query IDs and cited file paths in legacy raw data are never promoted to exact
delivery or memory attribution, even on apparent collisions. Original database
is unchanged. Row preservation does not claim a full immutable SQLite backup.
Oversized/invalid rows remain in source and are reported explicitly, never silently
truncated. Legacy data can only be inspected, never contribute to ranking.

Independent review corrected caller-actor amplification, contradictory judgments,
namespace collision, mutable source-file identity and archival semantics before
implementation. Claude design review is retained with phase32 evidence.

Claude identified additional ambiguities, resolved here: the logical key includes
record ID, revision, content hash and source hash; clipping variants share the
same group but must match their own delivery exactly. Only otherwise eligible
explicit helpful/unhelpful judgments participate in contradiction detection.
Unknown/inferred/legacy judgments cannot veto an explicit group. A query ID
with inconsistent query hashes across deliveries is ineligible. Query IDs and
actors are locally declared; duplicate protection does not authenticate people
or prevent a writer fabricating new deliveries. Repeated query text can yield
distinct samples, never a claim of independent human evidence.

Resolve deliveries only by exact scoped delivery ID, never query-ID search.
Feedback timestamp cannot precede delivery timestamp. Rebuild first validates
memory revisions, then all audit records, then feedback. Eligibility is derived
from these immutable inputs, never trusted from stored labels. Legacy reader
rejects pending source WAL/journals and verifies source bytes and sidecars before
and after reading; no SQLite immutable URI is used. Raw values encode SQLite storage classes;
feedback query_id is the table primary key, while row content hash distinguishes
changed snapshots. Normalize canonical Windows source path separators/case.
Report oversized rows as rejected and preserve their source, never truncated
stubs. Reserved scope applies to the feedback API; legacy discriminator prevents
attribution to any real memory/audit sharing that string. No feedback crosses
record revisions, even if new content happens to be equal.

Public contracts for implementation: FeedbackRequest contains id, repository,
source (kind/reference/actor), evidence, judgment, query_id, delivery_id,
reference (optional AuditReference) and comment (optional). FeedbackRecord holds
schema_version, request, recorded_at, optional legacy payload and content_sha256.
FeedbackEntry adds derived eligibility; FeedbackWriteOutcome has entry/created.
LocalStore exposes record_feedback, get_feedback and list_feedback. Paginated
FeedbackListQuery has repository, limit and cursor. Legacy import is an explicit
CLI operation; web and MCP never accept filesystem paths or legacy provenance.

## 1.3 Ranking policy local-feedback-v1

Opt-in per search request, disabled by default. Disabled mode calls the unchanged
baseline search path and returns identical identities, order and scores without
reading feedback. Enabled mode reranks the first 100 baseline candidates, then
applies the requested limit; it cannot recover absent matches. Only exact current
memory revisions with feedback for the exact UTF-8 query text hash participate.
The policy cannot cross baseline score classes 1 and 2 and does not affect events,
authority, source, applicability, supersession or graph links.

Capture one UTC as_of for the request (explicit RFC3339 UTC or current time).
Read candidates, deliveries and feedback in one snapshot. Filter feedback and
delivery timestamps strictly after as_of before grouping, contradiction detection
or query-hash consistency checks. as_of controls evidence age, not historical
reconstruction of the corpus. Query IDs with different delivery query hashes are
ineligible. Group by the section 1.2 key; opposing eligible judgments exclude the
group. Earliest eligible feedback time fixes age so later duplicate retries cannot
refresh a vote. Sort groups by their stable key before floating-point sums.

For each nonconflicting group, weight = 2^(-age_milliseconds / 2592000000), a
30-day half-life. Require at least three groups and total weight >= 1.5.
Otherwise delta is zero. With sufficient evidence:
delta = clamp(0.25 * sum(sign * weight) / group_count, -0.25, 0.25),
where helpful sign is +1 and unhelpful sign is -1. Use finite f64 arithmetic;
stable sort by adjusted score preserves baseline order on ties. No tuning against
held-out labels. Numeric constants fixed before evaluation.

Read at most 10000 scoped feedback records and 10000 scoped delivery records.
If either collection exceeds its bound, return baseline with an explicit
evidence_limit reason; never score a silently truncated set. Explanation includes
policy, as_of, baseline score, delta, final score, sample/conflict counts, effective
weight, reason and candidate limit. Inspectable eligibility means attributed
evidence only; it does not promise the threshold is met or model use was observed.
Changing actors or feedback IDs never increases group count. Untrusted local
writers can fabricate distinct query IDs: these are declared samples, not votes
from authenticated independent people.

Rollback is omission/false of the per-request flag; existing hooks and recall
remain baseline. No persistent enable switch, data deletion or migration is
needed to disable ranking. Independent review approved this policy with temporal
filtering, snapshot consistency, explicit read limits and stable summation.

## 2.1 Persistence and surfaces acceptance

Canonical feedback, schema-3 projection/rebuild, CLI record/get/list/import,
MCP record/get/list and authenticated embedded GUI inspection/capture implemented.
Legacy import reads only a private copied database and verifies original bytes,
mtime and sidecars before publication. Pagination cursor pins exact normalized
source path, full snapshot SHA256 and offset; replay is idempotent and changed
sources invalidate continuation. Oversized rows remain explicit rejections.

`cargo check -p cortex-local --all-features --all-targets -j 4` passed.
Focused tests: 16 passed across feedback_storage (6), feedback_import (5),
feedback_surfaces (4) and feedback_upgrade (1). Tests include corruption/recovery,
scope/hash/revision rejection, immutable retry, conflicts, source preservation,
typed SQLite values, continuation and schema-2-to-3 upgrade without source changes.
Independent review corrected clock-regression retries and bounded-import
continuation before acceptance. Final browser acceptance is recorded in section 3.3.

Actual legacy metadata.sqlite had a nonempty WAL. A separate private copy of
database plus WAL was verified byte-for-byte against original before/after opening
only that copy; SQLite backup produced a checkpointed snapshot. It contains zero
pre_thinking_feedback rows. This is measured absence of legacy feedback, not loss
of the separately recovered 106930 historical events. Evidence:
scripts/phase32-legacy-snapshot.tmp.json and phase32-tests-21.tmp.log.

## 2.2 Opt-in adjustment verification

`search --feedback [--as-of UTC]`, MCP search/query and web search share the
same snapshot-based engine. GUI starts unchecked and displays score/eligibility
explanations. as_of requires feedback on transport surfaces. Disabled searches
return the original JSON shape and skip feedback reads. Existing recall/hooks
remain baseline. Index schema 3 and feedback stay local.

Check passed; nine independent ranking tests and five transport tests passed.
Tests cover byte-equivalent disabled results, signed weights, thresholds/decay,
positive/negative bounds, replay and late duplicate age, future evidence before
conflict/query-hash checks, exact query/scope/revision isolation, event exclusion,
overflow fallback before payload loading, baseline top100 candidates/stable ties,
reindex reproducibility and rollback preserving canonical bytes. Source-only
Claude implementation review and tested dispositions are recorded below.
Evidence: scripts/phase32-check-22.tmp.log and phase32-tests-22.tmp.log.

## 2.3 Locked evaluation

Implemented scripts/evaluate_local_feedback.py with six passing harness tests.
Real CLI/MCP evaluation passed on all 20 frozen queries with 36 explicitly
fictional feedback records seeded only from development delivery references.
Held-out labels never seed feedback. Five paired trials alternate enabled and
disabled MCP requests; CLI results cross-check both modes outside timed trials.
Nearest-rank p95 uses ceil(0.95*n)-1; independent review corrected the initial
percentile off-by-one and the real report was regenerated.

Development Recall@5 remains 0.8625 and MRR@10 remains 1 in both modes. Held-out
Recall/MRR remain zero across seven answerable queries; all seven are unanswered.
Only development query d05 changes order, with no metric gain or regression.
Initial debug-build paired median latency was 4.58 ms disabled / 5.72 ms enabled,
p95 6.76 / 7.99 ms (100 samples per mode, environment-specific). Release acceptance
records its own hash/timings in section 3.3. No-feedback equality, exact rollback,
reindex reproducibility, 95 canonical files and fixture sources passed preservation checks.
Benefit remains `inconclusive_synthetic_only`; default remains disabled and
natural_language_ready=false. Report: scripts/phase32-feedback-eval.tmp.json.

Claude implementation review found four importer/validation issues and no concrete
ranking bugs. Corrections bind continuation mode, preserve cumulative rejections,
verify skipped-prefix publication before claiming completion (including forged
cursors), report absolute row positions/key hashes, reject extra legacy columns
and require exact typed-value keys. Check and 20 importer/storage/surface tests
passed after fixes; no earlier failure was represented as acceptance.

Authenticated fictional GUI checks verified initial disabled state, actual
alpha/beta reordering and rollback, exact-record capture, stable retry ID after
rejected provenance, new target/new ID, literal script text, exact lowercase scope
isolation and mobile layout without horizontal overflow. Temporary server exited;
its token is invalid. Final embedded release received fresh visual inspection.

## 3.3 Final acceptance

Accepted on 2026-10-18. Workspace check, formatting and strict Clippy passed.
The complete locked all-feature workspace test run passed 4324 tests with zero
failures and nine ignored across 275 targets, using `--test-threads=1` outside
the sandbox for Windows credential ACL tests. Default parallel stability is not
established. This includes 36 feedback integration tests and the additional
module-level regressions. Python ran 171 tests: 170 passed and one Windows
symlink-privilege skip. An initial stale fifteen-tool assertion failed; the
contract now requires eighteen unique tools including all three feedback tools,
and the entire Python suite passed again against the final release.

Release and installed binary SHA256:
`f30b1593cbeb252530df75e29796c4d7fd6d67ceb99d92831bfbd1c4ff86bc41`.
Both identify as standalone release builds with no required services. The real
derived index is schema 3. Plugin 0.4.2 is verified in both client caches; all
seven hook files are unchanged and the user already confirmed hook trust.
Reloading current clients and their actual use of the new plugin are not observed.
The previous binary/plugin pair remains in
`~/.cortex/backups/install-01M56YQKJCXNZH0SEZVJ6M2WKW`.

Fresh installed retrieval gate passed all three checks; installed smoke passed
fourteen checks, advertising eighteen tools and exercising thirteen read-only
tools. Feedback evaluation independently exercises recording and ranking. All
reports bind the same final binary SHA and unchanged frozen input hashes.
Final release feedback latency was median 3.38 ms disabled / 3.62 ms enabled,
p95 4.49 / 4.94 ms, across 100 samples per mode. Development Recall@5/MRR@10
remain 0.8625/1; seven answerable held-out queries remain unanswered. Only d05
changes order. All 95 fictional canonical files and fixture sources survive rollback/reindex.
No quality or productivity gain is established; default remains disabled and
natural_language_ready=false.

Fresh authenticated release GUI acceptance covered default-off ranking,
alpha/beta reordering and rollback, four-decimal score snippets, exact-record
unknown feedback capture, literal script text and exact-scope reset. Desktop
1360x900 and mobile 390x844 screenshots were visually inspected; no horizontal
overflow (mobile document width 375). The owned fixture server is stopped and
its token invalidated.

Claude source-only review findings were corrected with regression coverage:
cursor mode/cumulative failures, forged completion, typed NULL keys, generated
columns and NULL-row pagination. Its subsequent private-copy WAL objection was
refuted by an actual passing Rust regression. Independent final review approved
the corrections, conditioned on the now-passing gates; this does not relabel
Claude's raw NOT APPROVED reports as approval.

Preservation verified all 1659 original canonical files and all 69634 archive
files (319238316 bytes) unchanged. A subsequent local observation lesson adds
one canonical file; all originals were rechecked, giving 1660 canonical files.
The lesson is `phase32-feedback-ranking-has-no-proven-quality-gain@1`; no
external classification was requested. Final inventory: 106962 records/revisions,
106946 events, 16 memories, 37 repositories and 1812 links. Exact Cortex scope:
28 records, 16 events, 12 memories and one link. Its one prepared audit delivery
has zero observations or feedback; consumption remains not_observed. The actual
legacy feedback table contains zero rows. Previously reported sixteen incomplete
compressed frames remain explicit recovery limitations.

All phase32 temporary evidence is sealed under
`C:/Users/Bolado/.cortex/backups/phase32-feedback-f30b1593/evidence/`:
92 files, 29 directories, 4764605 bytes, with equal before/after manifests.
`phase32-final-acceptance.tmp.json` binds the reports, inventory and preservation.
Earlier `scripts/phase32*.tmp*` references in this design identify filenames in
that sealed directory. Private historical snapshots were never sent to Claude.

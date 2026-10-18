# Local declared feedback

Feedback records a declared judgment about an exact prepared delivery, optionally
one rendered memory revision. It does not prove consumption, causal usefulness,
independent human votes or productivity. Capture, inspection and optional search
ranking stay local; Jev is never consulted. External classification authorization
requirements are unchanged.

## Capture and inspect

Recall through `cortex_memory_read` or `cortex_pre_thinking`, then inspect the
returned delivery with `cortex_audit_get`. Use its exact query ID and delivery ID.
For a record judgment copy the complete rendered AuditReference, including
repository, ID, revision, content/source hashes, source kind and body_clipped.
Do not reconstruct a reference from the current revision or a path.

The online JSON request contains:

```json
{
  "id": "my-stable-feedback-id",
  "repository": "Cortex",
  "source": {
    "kind": "user_statement",
    "reference": "local-session:explicit-judgment",
    "actor": "declared-operator"
  },
  "evidence": "explicit",
  "judgment": "unknown",
  "query_id": "COPY_EXACT_QUERY_ID",
  "delivery_id": "COPY_EXACT_DELIVERY_ID",
  "comment": "Optional statement, maximum 4096 UTF-8 bytes"
}
```

Replace the placeholders with an existing scoped delivery. Omitting `reference`
makes this a bundle judgment; it cannot rank individual records. Evidence is
`explicit` or `inferred`; judgment is `helpful`, `unhelpful` or `unknown`.
Source kind/reference/actor are required and remain declared provenance, not
authentication. Unknown, inferred and legacy feedback are never eligible ranking
votes. Source strings are retained as data, not executed as instructions.

```powershell
cortex --repository Cortex feedback record --input judgment.json
# --input - reads the complete JSON request from stdin.
cortex --repository Cortex feedback get my-stable-feedback-id
cortex --repository Cortex feedback list --limit 20
cortex --repository Cortex feedback list --limit 20 --cursor 'COPY_NEXT_CURSOR'
```

The input repository must equal the selected exact CLI scope. MCP provides
`cortex_feedback_record`, `cortex_feedback_get` and `cortex_feedback_list` with
the same engine rules and standard repository aliases. List limit is 1..100,
default 20. Online surfaces reject legacy provenance and the reserved legacy
scope. Neither MCP nor web accepts filesystem import paths.

Server time is recorded once. Repeating the identical request and stable ID
returns the original record; changing it conflicts. Retry a failed/uncertain
save with the same ID and payload rather than inventing a new vote. Feedback has
no overwrite/revision/delete operation. Canonical JSON is bounded to 64 KiB,
stored under `local/feedback/`, and indexed by schema 3. Back up the whole home.
Normal open upgrades the index; hooks and dry-run inspection never migrate.

GUI Audit inspection offers bundle and exact-record judgment buttons on a
selected delivery. The feedback form records explicit declarations, keeps its
ID on failed retries and rotates it for a newly selected target. Feedback list
and detail show provenance and derived eligibility. Imported unknown feedback
is visible only after selecting exact scope `legacy-feedback-unattributed`.

## Optional search ranking

```powershell
cortex --repository Cortex search "release build"
cortex --repository Cortex search "release build" --feedback
cortex --repository Cortex search "release build" --feedback --as-of 2026-10-18T12:00:00Z
```

MCP search/query and HTTP search accept `feedback: true` and optional UTC `as_of`.
An as-of time requires feedback enabled. GUI's search checkbox starts unchecked.
Rollback is omission/false of that flag or unchecking it: baseline identities,
scores and order return without feedback fields or evidence reads. No persistent
switch, data deletion or migration is needed. Recall/hooks remain unchanged.

Policy `local-feedback-v1` is fixed before evaluation:

- Rerank only the first 100 baseline candidates, then apply the requested limit.
  Missing literal-AND matches cannot be recovered. Events and baseline score
  classes are preserved; adjustments cannot cross score classes 1 and 2.
- Require explicit helpful/unhelpful feedback for the exact current record ID,
  revision, content/source hashes and exact UTF-8 search-query hash. Rendered
  reference validation includes clipping. New revisions inherit no old votes.
- Group by repository, query ID and exact record identity. Changing feedback ID,
  delivery ID or declared actor cannot amplify a query group. Opposing otherwise
  eligible judgments invalidate that group; unknown/inferred judgments cannot
  veto it. Inconsistent query hashes for a query ID exclude its evidence.
- Filter feedback and delivery timestamps after one UTC as_of before grouping.
  Earliest eligible group time fixes age; retries cannot refresh weight. Weight
  is `2^(-age_milliseconds / 2592000000)`, a 30-day half-life.
- Require at least three groups and effective weight at least 1.5. Adjustment
  is `clamp(0.25 * sum(sign * weight) / group_count, -0.25, 0.25)` with helpful
  sign +1 and unhelpful -1. Stable ordering preserves baseline ties.
- Read at most 10,000 scoped feedback and 10,000 scoped delivery records. Exceeding
  either bound returns baseline with explicit `evidence_limit`, not partial
  scoring. Candidates, deliveries and feedback share one read snapshot.

Each enabled hit includes `feedback` explanation: policy, as_of, baseline_score,
adjustment, final_score, samples, conflicting_groups, effective_weight, reason,
candidate_limit, evidence_records and deliveries. GUI list scores are rounded
for readability; selected explanation JSON retains full precision. As-of limits
evidence age, not historical reconstruction of the memory corpus. Declared query
groups are samples, not authenticated independent people; local writers can
fabricate distinct queries. Eligibility alone does not promise threshold success.

## Legacy import

```powershell
cortex feedback import 'C:\history\metadata.sqlite' --dry-run --max-records 1000
cortex feedback import 'C:\history\metadata.sqlite' --max-records 1000
cortex feedback import 'C:\history\metadata.sqlite' --max-records 1000 --cursor 'COPY_NEXT_CURSOR'
```

The importer reads source bytes; SQLite opens only a private copied snapshot.
No source migration occurs; pending WAL/journals are rejected.
The immutable snapshot preserves all eight original fields with SQLite storage
classes, source path and canonical raw-row hash. Deterministic source-path/row
identity deduplicates unchanged occurrences; changed rows become new snapshots.
It is row preservation, not a full historical database backup. The source and
sidecars remain unchanged. Invalid/oversized rows stay in source and are reported,
never truncated into placeholders.

All legacy evidence is `legacy_unknown` in `legacy-feedback-unattributed`.
Neither legacy query IDs, file paths nor apparent receipt matches establish exact
attribution. These records are inspectable and permanently excluded from ranking.

Import pages accept max-records 1..10000, default 10000. Continue with the returned
next_cursor until remaining is zero. Cursors bind canonical source path, snapshot
hash, dry-run/apply mode and cumulative rejection counts (including store-side
failures separately). Do not reuse preview
cursors for apply or edit a cursor. Source changes require a new traversal;
unchanged rows remain idempotent. Final apply verifies the imported prefix before
claiming completion. Inspect rejected/cumulative_rejected, prior_missing,
prior_rejected, prior_progress_verified and complete; no last-page success hides
earlier rejected or unpersisted rows. A schema-2 destination needs normal open
or explicit reindex before dry-run inspection; dry-run cannot upgrade it.

## Evidence and limits

Final installed-release evaluation creates 36 fictional feedback records using
development questions only across twenty frozen queries and five paired trials. Only d05 changes
order; development Recall@5 remains 0.8625 and MRR@10 remains 1. All seven
answerable held-out questions still return zero; readiness is false. No measured
quality or productivity gain is established. Frozen corpus/policy stay unchanged.

The actual historical metadata source contains zero feedback rows. This is
separate from the 106,930 historical events previously recovered. Final release
GUI passes capture, ranking, rollback, literal rendering and scope reset on
desktop 1360x900 and mobile 390x844 (375 CSS-pixel width), without horizontal
overflow. Its server stopped and temporary token is invalid.

Installed SHA-256:
`f30b1593cbeb252530df75e29796c4d7fd6d67ceb99d92831bfbd1c4ff86bc41`.
Schema 3, eighteen tools and both plugin 0.4.2 caches are verified; seven hook
files remain unchanged. Active-client reload and model consumption are unobserved.
Fourteen installed smoke checks pass, including thirteen read-only operations.
Full Rust validation passes 4,324 tests with zero failures and nine ignored across
275 targets, serially outside the sandbox; check, format and strict clippy pass.
Python runs 171 cases: 170 pass and one Windows symlink check skips. An initial
stale fifteen-tool expectation was corrected before the successful full rerun.

Release MCP latency median is 3.38 ms disabled / 3.62 ms enabled; nearest-rank
p95 is 4.49 / 4.94 ms, environment-specific. Evaluation preserves 95 canonical
files. Installation acceptance preserves 1,659 original canonical files and
69,634 archive files (319,238,316 bytes); one local lesson is additive. Sixteen
previously truncated source frames remain incomplete. No productivity or complete
recovery claim follows from these checks. Sealed evidence:
`C:/Users/Bolado/.cortex/backups/phase32-feedback-f30b1593/evidence`.

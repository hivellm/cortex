# Phase34 section 1.12 - Astra final review

Date: 2026-10-16. Independent review of the working tree after sections 1.1-1.11.

## Findings and fixes

### P1 - Archive projection removed access classification

Trigger: enable incremental archive retrieval, archive a supported envelope with
`class_level=2` and `class_compartments=["financial"]`, then query its repository
with an authenticated principal at level zero. The projection constructed by
`crates/cortex-api/src/lanes/archive_loader.rs` in `envelope_to_hit` omitted both
classification fields. `poll_and_publish` persisted that projection, and the
orchestrator's `apply_acl_wedge` interpreted the missing fields as public. The
same leak applied to compartment-only restrictions.

Fix: preserve both optional envelope fields in `LaneHit.extras`. Before publishing
archive hits, require the `lane-hit-acl-v1` projection contract. The new
`ArchiveIndex::require_projection_version` transaction invalidates old derived
hits and their source references, resets decode checkpoints and records the new
contract. Existing permanent suppressions and archive bytes survive. No old
unlabeled snapshot is published while bounded reprojection proceeds.

Regression: `archive_reprojection_enforces_classification_and_exact_repository_for_real_keys`
builds an old database without access labels, reopens it, asserts immediate
invalidation, rebuilds through the production projection/publication path, and
queries through `QueryService::handle_with_headers` with three actual API keys.
Public and level-two/no-compartment principals see only public content; the
level-two/financial principal sees authorized content. Both fresh and cached
queries are checked.

### P1 - Archive slug matching crossed exact repository authorization

Trigger: caller is authorized only for `foo-bar`; archive contains an event in
`foo_bar`. `QueryService` checks the exact requested repository, while
`scoped_archive_hits` compared both source/request repository slugs. Both names
produce `foo-bar`, allowing the second repository's content into the first query.

Fix: retain slugs for physical index names, but require exact equality between
the requested repository identity and each projected hit's repository. The real-key
regression above includes a public `foo_bar` event and verifies that even the
financial principal authorized for `foo-bar` cannot retrieve it.

Compatibility consequence: archive callers must provide the actual repository
identity. Case variants and punctuation variants do not confer access through a
shared index slug. This matches the exact repository ACL and daily-memory scope.

## Transaction and restart verification added

`archive_index_projection_upgrade_is_bounded_durable_and_preserves_suppression`
covers an existing 600-event projection, explicit suppression, contract upgrade,
immediate empty snapshot, bounded first poll, restart during reprojection,
599 surviving events, unchanged archive bytes, and no further decode for the
same contract after completion.

The one-time contract change performs SQLite cache deletion and manifest reset
in one transaction. Manifest SQL work scales with known file count; it is not
claimed to satisfy the cooperative 100 ms decode budget. Subsequent discovery,
decoding and publication use the existing per-poll limits. Contract changes do
not enable recurring full archive decompression. Corrupt or unavailable sources
remain unavailable/degraded rather than restoring unlabeled old projections.

## Review scope and limits

Inspected daily-memory capture, correction, idempotency, scoped revision/search,
stale marking, delivery transaction, API cache reconstruction, daily authorization,
ingest mapping, feedback authorization, API-key authority revalidation, archive
cursor/decoder transactions, source replacement/deletion, publication/suppression,
and selected daily runtime route/factory gates. No additional concrete P0-P2
finding was established in those paths during this review.

Capture/correction evidence and scope revisions share their write transaction.
Retry receipts are caller/scope-bound and precede optimistic revision checking.
Daily query delivery validates current revisions and record versions before
returning cached context. Timed-out blocking work retains admission permits.
These observations are static code evidence, not substitutes for execution gates.

No Cargo command, provider/model call, active-runtime mutation, credential dump,
or real-memory probe was performed by this reviewer. Source fixes and the tests
above were formatted with rustfmt. Root coordinator owns type-check, Clippy and
test execution after concurrent section 1.12 fixes settle. Until those results
are recorded, these fixes are code-complete but execution validation is pending.
Existing phase31 `LawsConfig` work and unrelated dirty files were preserved.

## Cross-review of section 1.12 peer fixes

The new `memory_repo` gate was reviewed in `scope.rs`, `pipeline.rs` and the
adapter dispatcher. It requires a valid explicit `cortex.id` at the nearest Git
root; missing/invalid identity does not issue a local-memory request. Scope work
and response formatting retain admission permits after an awaiting timeout.
The dispatcher reports the new identity-required outcome. No additional P0-P2
finding was established in those changes themselves.

The canonical archive emitter, tail retry and bootstrap retry changes were also
reviewed. Successful archive emits finish and sync their own frames before IDs
are marked delivered. Failed tail emits leave the source eligible without a new
append; failed bootstrap files lose their completed-file checkpoint and cause
a nonzero result, allowing explicit resume.

### P2 - Imported calls without outcomes were still omitted from projections

Evidence: `claude_archive/mapper.rs` creates ToolCall and AgentCall payloads from
observed `tool_use` blocks without an `outcome`; the corrected emitter preserves
that unknown result. `archive_loader.rs` previously deserialized the complete
core `ToolCall`/`AgentCall` types, whose `outcome: String` is required. Projection
therefore returned `None` for these valid partial observations, while the index
committed its cursor. The emitter's new integration regression covered Turn only.

Fix: archive projection now uses narrow typed payload views containing only the
fields it displays. Tool output still uses the existing `ToolCallOutput` type.
Missing outcome stays unknown; neither source payload nor core schema is changed.
Repository identity and access classification continue through the same projection.

Regression `imported_calls_without_outcome_survive_mapper_emitter_and_projection`
reads synthetic native JSONL, maps its user/assistant/tool/agent observations,
emits actual archive frames, and checks both calls survive the production loader.
It also verifies absent outcome, unknown source repository, exact explicit
repository and level/compartment propagation. rustfmt and diff-check passed;
Cargo remains owned by the coordinator.

Preexisting limit: the legacy mapper synthesizes random IDs when native UUIDs or
tool IDs are absent. Its retry deduplication therefore assumes source IDs exist;
this is separate from daily capture's caller-scoped durable retry contract.
No source-identity redesign was included in this review fix.

Additional integration concern sent to the coordinator: adapter `sync_paths.rs`
uses `cwd.unwrap_or(".")`, so an incomplete hook frame can derive the daemon's
repository. Native shim normally supplies cwd; disposition of missing/empty cwd
belongs to the integration owner. This review did not modify that path.

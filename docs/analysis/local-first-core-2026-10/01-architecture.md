# Standalone core architecture and Claude review

Date: 2026-10-17. Task: `phase36_local-first-core`, section 1.1.

This planning checkpoint is retained as written. The accepted implementation,
final migration accounting and measured limits are recorded in
[05-implementation-evidence.md](05-implementation-evidence.md) and
[ADR 041](../../../.rulebook/decisions/041-standalone-local-memory-core-and-explicit-advisory-classification.md).

The operator requested a single local Cortex CLI with embedded web GUI, Jev
classification, queryable local indexes and simple graph relationships, replacing
the Docker-dependent core while recovering months of historical data.

## Research and review

Sources: https://github.com/akitaonrails/ai-memory,
https://raw.githubusercontent.com/akitaonrails/ai-memory/main/docs/ARCHITECTURE.md,
https://docs.typesafe.ai/api.md and
https://docs.typesafe.ai/primitives/choice.md, read on the date above.
The reference architecture informs the file/index boundary; its code is not copied.

Claude Code (`claude-fable-5-1`, read-only planning, no enabled tools) reviewed the
proposal. Its substantive corrections were accepted: raw history must be batched
NDJSON instead of one Markdown file per event; canonical serialization and immutable
version rules must be explicit; cross-process locking must cover file publication
and index rebuild; tests belong in every implementation section. Existing GUI bulk
views cannot be embedded unchanged because their services are absent.

Inventory: `C:/Users/Bolado/.cortex/archive` contains 69,101 `.parquet` files,
318,898,627 compressed bytes. These are zstd NDJSON, decoded by the existing bounded
`TailReader`. There are also existing quarantine/reason files. Presence and byte
counts are not assertions that every archived record is valid or complete.

## Record contract

- Stable `id` and exact `repository`; version starts at one and increases only
  through explicit expected-revision writes. Import IDs derive from source identity.
- Kind distinguishes events, decisions, lessons, preferences and notes. Authority
  distinguishes observation, proposed, accepted, deprecated and superseded.
- Title/body plus source kind, locator, original ID/time and original payload.
- Typed links carry target ID and relation: related, supports, contradicts,
  derived-from or supersedes. Missing targets remain inspectable; traversal never
  broadens repository scope.
- Metadata and content hash travel with canonical content. JSON keys are ordered,
  encoding is UTF-8 without BOM and generated line endings are LF.
- Memory revisions are immutable Markdown with JSON frontmatter. Event batches
  are immutable NDJSON segments. Source identifiers never become raw OS filenames;
  hash-based paths avoid traversal, Windows reserved names and case collisions.
- Jev judgments carry input identity/version, actual model, schema, probabilities,
  confidence and token usage separately from source authority. Reindex preserves them.

## Invariants

Capture acknowledges only durable canonical content. Retries of identical input
do not duplicate records; conflicting content under the same identity is rejected.
No existing source record is overwritten. The OS file lock spans canonical
publication and index transactions; SQLite uses WAL and bounded busy waits.
Reindex validates sources before replacing indexed content and preserves the
working index on failure. A missing index can be reconstructed from files.

Imports stream bounded batches, commit checkpoints after canonical persistence,
detect changed/truncated sources and report invalid records explicitly. Sources
are read-only. Search covers the full imported corpus and uses deterministic
ranking with repository filters. No snapshot eviction silently removes history.

CLI/MCP/hooks operate directly on this engine. Only `serve` runs a foreground
HTTP process, serving bundled assets and the same storage API. No separate
adapter, index server, GPU or Docker process is required. Hooks do not wait on
Jev or an unbounded import/reindex. Local HTTP rejects unrelated origins/hosts
and authenticates API access; untrusted record text is never executable HTML.

## Validation obligations

Each implementation step includes focused tests. Final acceptance covers crash
recovery, simultaneous writers, index reconstruction, repeated historical import,
cross-repository isolation, source preservation, Jev failure/retry and real
retrieval from imported history. Release hook latency is measured on imported
data; healthy processes or prepared receipts do not prove useful model recall.

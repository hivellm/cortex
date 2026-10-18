# Standalone local Cortex

## Authorization and scope

On 2026-10-17 the operator requested replacing the complex Docker stack with a
self-hosted CLI, embedded web GUI, local files and queryable indexes, Jev
classification and simple graph interconnections, planned with Claude and executed.
This supersedes the daily-only architecture as the target; it does not authorize
discarding historical data or deleting source stores. Implementation follows the
task checklist in order. Existing unrelated backlog is not part of this change.

## Evidence

- https://github.com/akitaonrails/ai-memory and its architecture document describe
  canonical Markdown, rebuildable SQLite indexes and typed links in one binary.
  Cortex adopts that architectural pattern; no upstream code is copied.
- Local inventory found 69,101 compressed archive files totaling 318,898,627 bytes.
  Their `.parquet` extension is historical: `cortex-storage::archive_tail` reads
  zstd-compressed NDJSON. Existing sources remain untouched.
- Current daily memory stores six explicit project memories; it does not replace
  or automatically index the historical corpus.
- TypeSafe's current HTTP contract is `POST /v1/systemone`, with `state`, `model`
  and typed `questions`, returning `answers`, model identity and token usage.

## Boundaries

`cortex-local` owns the new engine, with no dependency on the bulk API, worker
crate, Synap, Meilisearch, Vectorizer or Nexus. Rust `cortex-core` and
`cortex-storage` provide compatible event types and archive decoding. A single
`cortex` executable exposes CLI operations, local MCP, hooks, and an optional
foreground HTTP server with embedded web assets. Commands use the engine directly;
capture and search do not require a running server. No OS scheduler is installed.

The old runtime remains buildable for recovery and legacy tests, under an explicit
legacy entry point. It is excluded from the default standalone build/install.
No command implicitly starts Docker or rewrites native agent memory.

## Persistence

The new store is isolated under `<CORTEX_HOME>/local`; old SQLite, archives, CAS,
Docker volumes and client transcripts are read-only migration sources.

Canonical memory records are readable Markdown files with machine-readable JSON
frontmatter containing identity, exact repository scope, kind, source, timestamps,
version and typed links. Raw events use immutable bounded NDJSON segments rather
than one Markdown file per event; segments retain full original source payloads.
Both layers are canonical and participate in reindex. Claude's architecture
review specifically required this split to avoid filesystem amplification across
months of tool events. JSON metadata is canonically ordered, UTF-8/LF without BOM.
Filesystem names are derived from validated identities/hashes, never raw paths
supplied by clients. Revisions are immutable; corrections create a new revision
or explicit supersession. Raw observations never become accepted decisions merely
because a classifier labels them as decisions.

SQLite stores a derived full-text index, current record projections, graph edges,
classification work and migration checkpoints. Search is bounded, parameterized,
literalized for FTS, and always scoped to an explicit repository unless the local
operator explicitly asks for an all-repository inventory. Current recall excludes
superseded/deprecated evidence by default; historical inspection remains possible.
Graph traversal is bounded by depth and result count and cannot cross scopes.

Writes must persist canonical content before acknowledging success. Concurrent
writers coordinate through an OS file lock, SQLite WAL/busy timeout and immutable
atomic publication; the lock remains valid even when an index needs rebuilding.
Recovery can index committed files after an interrupted index update. Reindex
must validate every source and retain the working index on failure. Durable
classification results and links must survive a complete index rebuild.

## Historical import

Import explicitly selects existing daily SQLite or an archive directory. Dry-run
inventory is available. Source identifiers, provenance and original payloads are
retained; retries are idempotent. Streaming archive decoding uses `TailReader`
with bounded batches and a persisted checkpoint only after records are durable.
Failures identify source and reason, remain retryable, and never advance beyond
uncommitted data. Truncated/replaced sources are detected instead of silently
trusting an obsolete offset. A corrupt file does not erase successful imports.
Reports distinguish files scanned, imported records, duplicates, failures and
incomplete tails. No provider call runs as a side effect of historical import.

## Jev

New classifications use typed Choice and Noul judgments for category and durable
relevance, with explicit unknown/transient outcomes. Store full validated
probabilities, confidence, provider model/schema identity and actual token usage.
Classification adds advisory metadata; it cannot change accepted status, repository
scope, source content or supersession. Redact sensitive material, bound request
size, concurrency and timeout, reject redirects and malformed responses, and
retry provider failures with bounded backoff. Credential values never appear in
files, tool output, browser assets or logs. Explicit classify commands drain a
bounded durable queue; missing credentials leave work pending and do not break
capture/search. Bulk historical classification is separately bounded.

## User and agent surfaces

CLI: capture, search, get, graph, import, reindex, status, classify, serve, mcp,
hook and single-executable installation/doctor. JSON output supports automation.
MCP retains the useful memory tool names while querying the local engine directly;
it advertises only implemented operations. Hooks capture sanitized real events,
resolve repository identity from `cortex.toml`, and return bounded cited context
only through supported harness lifecycle contracts. No fabricated session IDs.

Embedded GUI provides local corpus counts, search, record/provenance inspection,
typed relationships, import outcomes and classification state. Assets ship inside
the executable and use same-origin API calls. The server binds loopback by default,
rejects untrusted Host/Origin headers, requires authenticated mutations, and never
renders imported text as executable markup. Provider credentials remain server-side.

## Acceptance

- Standalone dependency tree and execution require no Docker/service SDKs.
- Canonical files survive index loss and reconstruct search, versions and links.
- Crash/retry and simultaneous writers cannot silently lose or overwrite records.
- Import resumes without duplication and retrieves events outside the old 2,000
  record snapshot limit; source hashes remain unchanged.
- Jev unavailable/malformed/rate-limited responses retain durable pending work.
- One executable serves GUI and API; direct MCP/hooks work with HTTP server stopped.
- Real historical search is validated with Docker stopped; API reachability and
  prepared delivery receipts alone do not count as observed agent use.
- Rust check, formatting, strict lint, relevant tests, full suite and independent
  review precede task completion and archival.

## Claude review

Claude Code `claude-fable-5-1` reviewed this design on 2026-10-17 in a read-only
planning invocation. Accepted corrections: separate batched raw events from human
Markdown memories; specify immutable versions and canonical serialization; lock
across processes, including index reconstruction; keep per-step tests; embed assets
without requiring Node at runtime or installation. Source preservation, exact
scope, durable classification metadata, bounded hooks, and full-corpus recall are
explicit acceptance gates. A summary and record contract are retained in
`docs/analysis/local-first-core-2026-10/01-architecture.md`.

## Acceptance retained from the old backlog review

The 2026-10-17 review in
`docs/analysis/local-first-core-2026-10/02-backlog-review.md` retires obsolete
service-stack tasks while retaining these requirements within the existing
checklist order:

- Section 1.2: repository isolation covers graph traversal, aggregate counts,
  caches and ranking as well as returned records. Linked endpoints must be
  inspectable; graph population alone is not evidence of useful retrieval.
- Section 1.3: historical events expose readable searchable text while retaining
  source payloads and exact scope. Test recall beyond the old snapshot window.
- Section 1.5: implemented, advertised and documented MCP surfaces agree; only
  supported harness contracts carry context. Prepared delivery is distinct from
  observed agent use, with unknown outcomes reported honestly.
- Section 1.7: functionally smoke-test every advertised read-only tool using an
  isolated corpus, including direct CLI/MCP with HTTP and Docker stopped. Do not
  invoke mutating smoke operations against real historical data. Check current
  standalone dependencies and record actual quality/latency/link results;
  historical checked boxes and old service measurements do not satisfy gates.

The adapted retrieval evaluation and durable-use audit are follow-ups to the
completed core; feedback-based ranking additionally depends on both. Their
retained lower phase numbers do not authorize running them before phase36.

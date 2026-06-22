# phase0 — live ingestion staleness (no current events reach the indexes)

Source: system-state analysis (2026-06-21). The `cortex-cortex-turns`
index froze at 2026-06-18T20:43; the graph carried 0 confidence edges and
consolidation surfaces were empty — every retrieval/consolidation/graph
feature was operating on a frozen 3-day-old corpus.

## Why

Two independent ingestion paths feed Cortex, and both were broken:

1. **Cold archive path** (`cortex-claude-archive` tail watcher → parquet
   under `~/.cortex/archive`). **FIXED in commit 4706f08.** Root cause: the
   watcher's `root` is `/data/claude-projects` and `walk()` appends
   `/projects` (root must be the dir CONTAINING `projects/`), but the
   compose mount put the host's `~/.claude/projects` directly at
   `/data/claude-projects`. So `walk()` scanned the nonexistent
   `/data/claude-projects/projects` → `files_watched=0`, silently halting
   archival of new session transcripts. Fixed by mounting at
   `/data/claude-projects/projects`. Verified: `files_watched` 0→203,
   15 052 envelopes emitted.

2. **Live indexing path** (host adapter daemon → `cortex-ingestion`
   `POST /v1/events` → Synap raw → classifier → enriched → embedder/
   fulltext/graph → the live Meili indexes + Nexus). **STILL BROKEN.**
   `cortex-ingestion` and `cortex-classifier-worker` are idle (no activity)
   and `~/.cortex/adapter-daemon.log` last wrote 2026-06-20 13:13 — the
   host adapter daemon (Claude Code hook → POST /v1/events) is not running
   / not posting, so nothing reaches Synap raw and the live turn/tool_call
   indexing never fires. This is the path that keeps the turns index fresh
   and (now, on Nexus 2.3.4) would let phase27a confidence land on live
   edges.

## What Changes

- Confirm whether the cold-archive fix (path 1) alone re-feeds the live
  indexes (does `cortex-api`'s archive_loader / a backfill pick up the new
  parquet into Meili/Nexus?). If yes, path 2 is only needed for real-time
  freshness; if no, path 2 is required for any indexing.
- Diagnose path 2: is the host adapter daemon installed + running on this
  machine? Is the Claude Code hook configured to POST to
  `cortex-ingestion:17010 /v1/events`? Restart/repair it; verify
  `cortex-ingestion` receives POSTs and publishes to Synap raw, the
  classifier enriches, and the turns index advances past 2026-06-18.
- Add a watchdog: the `cortex-claude-archive` health already exposes
  `files_watched` — surface a coverage alarm when `files_watched==0` while
  the mount is non-empty (would have caught path 1 in minutes). Add an
  equivalent freshness alarm on `cortex-ingestion` (no POSTs in N min).

## Impact
- Affected code: `docker-compose.yml` (path-1 mount, DONE); host adapter
  daemon config; `crates/cortex-workers/src/claude_archive/` (watchdog);
  coverage/health alarms.
- Breaking change: NO.
- User benefit: the system ingests current work again — retrieval,
  consolidation, and the graph reflect today, not a 3-day-old snapshot.
  This is the single biggest gap between "where we are" and a useful system.

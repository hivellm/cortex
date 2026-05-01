# `cortex-claude-archive`

Phase11i §1 — ingest the Claude Code (and Codex) JSONL conversation
archive into Cortex. Walks `~/.claude/projects/<project>/*.jsonl`,
parses each record, projects them into canonical
`cortex_core::events::Envelope` shapes, and ships them through one
of two sinks. Phase11i §5 added a long-running watcher daemon and a
health endpoint so an operator can run the indexer alongside the
rest of the Cortex stack.

The crate ships a library + a single binary
(`cortex-claude-archive`). The library is what the live worker
pipeline depends on; the binary is the operator surface.

## Subcommands

```text
cortex-claude-archive estimate    # count files / projected envelopes; no writes.
cortex-claude-archive bootstrap   # one-shot full ingest, with checkpointing + --resume.
cortex-claude-archive tail        # long-running watcher; emits new envelopes as sessions append.
```

### `estimate`

Walks the configured root and reports:

- session-file count
- sidecar-file count
- total bytes
- projected envelope count (≈ 30 % of records, the corpus-wide ratio
  of Turn / ToolCall / AgentCall vs. attachment / system / file-history
  records).

Use this before a bootstrap pass to size the run.

### `bootstrap`

Reads every selected file front-to-back. Each session runs through
`reader::read_records` → `mapper::map_session` → the chosen sink.
Honours `--resume` against the checkpoint file (`.cortex-claude-archive.checkpoint.json`
under the root by default), so a crashed run picks up at the next
unprocessed file.

```bash
cortex-claude-archive bootstrap \
    --root ~/.claude \
    --projects-only \
    --sink archive \
    --archive-root ~/.cortex/archive \
    --resume
```

### `tail`

Phase11i §5.2 long-running watcher. Polls the configured root every
`CORTEX_CLAUDE_ARCHIVE_POLL_MS` (default 1 s). For every session
file whose `(mtime, len)` advanced since the last tick, the watcher
re-runs `reader::read_records` + `mapper::map_session` and emits
envelopes whose `event_id` is not in the in-memory dedupe set. The
watcher exposes a `:17030/healthz` endpoint (axum) returning a
JSON snapshot the dashboard surfaces under
`/v1/health/coverage.archive_watchers`:

```json
{
  "last_flush_ts": 1712345678901,
  "files_watched": 42,
  "envelope_rate": 12.5,
  "rss_bytes": 18874368,
  "envelopes_emitted": 42_113,
  "envelopes_dropped": 7,
  "uptime_ms": 3_600_000,
  "status": "healthy"
}
```

```bash
cortex-claude-archive tail \
    --root ~/.claude \
    --projects-only \
    --sink archive \
    --archive-root ~/.cortex/archive
```

The docker-compose `cortex-claude-archive` service ships exactly
this command (read-only mount of the host's `~/.claude/projects`,
archive bind-mount onto `~/.cortex/archive` so cortex-api's
archive_loader re-reads it at boot).

## Sinks

- `--sink stdout` — one canonical-JSON line per envelope on stdout.
  Cheap, ideal for piping through `jq` during development.
- `--sink archive` — zstd-compressed NDJSON shards under
  `<archive_root>/events/year=YYYY/month=MM/day=DD/hour=HH/bootstrap-claude-NNNNN.parquet`.
  cortex-api's `archive_loader` re-reads the same path tree at boot
  so the watcher's output drives the keyword + vector lanes without
  needing the live worker pipeline to be up.

## Checkpoint format

Bootstrap writes an atomic JSON file under
`.cortex-claude-archive.checkpoint.json`:

```json
{
  "version": 1,
  "sessions": {
    "<project_dir>/<session_filename>": {
      "session_id": "01HFIXED...",
      "last_record_uuid": "u1234",
      "last_byte_offset": 0,
      "written_at_ms": 1712345678901
    }
  }
}
```

Today the checkpoint is at file granularity (`last_byte_offset = 0`).
A future iteration will track byte-offset granularity so a partially
-processed file resumes mid-stream rather than re-reading from the
top.

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `CORTEX_CLAUDE_ARCHIVE_BIND` | `0.0.0.0:17030` | Bind address for the `tail` health endpoint. Parse error aborts boot. |
| `CORTEX_CLAUDE_ARCHIVE_POLL_MS` | `1000` | Polling cadence in ms; clamped to `[100, 60_000]`. |
| `CORTEX_ARCHIVE_ROOT` | `~/.cortex/archive` | Archive sink root when `--archive-root` is unset. |
| `CORTEX_ARCHIVE_WATCHER_URLS` | _(unset)_ | Comma-separated list of `<host>:<port>` watchers cortex-api probes for the `archive_watchers` block under `/v1/health/coverage`. |

## Resource footprint

Measured on the §5.3 IT against a synthetic 100k-event session
(50 k user/assistant pairs, ~50 MB of JSONL):

| Metric | Value |
| --- | --- |
| Single-tick wall time | sub-second |
| Per-file mtime stat | O(N) in file count |
| Peak RSS after 50 k Turn emits | 14.9 MiB |
| §5.2 spec ceiling | 512 MiB |

The dedupe set holds one ULID-shaped string per emitted envelope; at
50 k entries that's ≈ 1.3 MB. The mapper-side allocations are
short-lived (records + envelopes drop out of scope at the end of the
tick), so a tick over an unchanged corpus is effectively allocation
-free.

## Failure modes

| Symptom | Behaviour |
| --- | --- |
| Malformed JSONL line | Reader warns + drops; counted in `envelopes_dropped`. Watcher tick stays `healthy`. |
| Typeless record (no `type` field) | Same as malformed — counted in `envelopes_dropped`. |
| Unreadable file | Tick records the IO error in `report.errors`; watcher status flips to `degraded: <reason>`. |
| Crash mid-bootstrap | Checkpoint at file granularity — `--resume` skips every file already past the checkpoint. |
| Sink failure (e.g. archive root unwritable) | Per-envelope failure counted in `envelopes_dropped`; tick continues so a transient failure does not stall the watcher. |

## Tests

```bash
# Unit tests + non-gated ITs.
cargo test -p cortex-claude-archive

# §5.3 RSS cap IT (synthesises 50 k pairs; ~4 minutes).
CORTEX_ARCHIVE_MEMORY_IT=1 cargo test -p cortex-claude-archive --test memory_it -- --nocapture

# §5.4 corrupt-line IT runs unconditionally — no gate, sub-second.
cargo test -p cortex-claude-archive --test corrupt_line_it
```

## Related specs

- `docs/specs/01-event-schema.md` — canonical envelope shape this
  crate emits.
- `docs/specs/12-pre-thinking-injection.md` — how the rendered
  bundle surfaces watcher data (past sessions, similar turns).
- `docs/cortex/relevance-tuning.md` — operator handbook for the
  relevance signals the watcher feeds.
- `.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/` —
  task tree this crate lives under.

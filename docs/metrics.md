# Cortex metrics catalogue

Phase8b — every long-running Cortex binary exposes a Prometheus-text
`/metrics` endpoint on the same listener as `/healthz`. The catalogue
below documents the canonical metric names, labels, and the freshness
aggregator pairs the cortex-api endpoints derive from them.

All counter metrics are monotonic across the lifetime of the process.
On restart they reset to zero; the binary's `since` timestamp on
`/healthz` lets the aggregator distinguish "never observed activity"
from "process restarted recently".

## cortex-adapter-claude-code (port 17011)

### IPC / dispatcher

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_adapter_ipc_frames_received_total` | counter | `hook` | Frames the IPC handler accepted and parsed enough to identify the hook label. |
| `cortex_adapter_ipc_frames_parsed_total` | counter | `hook` | Frames the dispatcher fully parsed into a `HookFrame`. Pairs 1:1 with `frames_received_total` on healthy paths. |
| `cortex_adapter_ipc_frames_parse_error_total` | counter | — | Unlabelled count of frames that failed JSON parsing. The hook label is unknown at this point. |
| `cortex_adapter_envelopes_built_total` | counter | `kind` | Envelopes the dispatcher handed to the publisher, per canonical kind. |
| `cortex_adapter_last_frame_ts_ms` | gauge | `hook` | Unix-epoch ms of the most recent successfully parsed frame. |
| `cortex_adapter_last_envelope_ts_ms` | gauge | `kind` | Unix-epoch ms of the most recent envelope built. |

### Publisher

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_adapter_envelopes_publish_ok_total` | counter | `kind` | Envelopes the ingestion side accepted in the 202 response body. |
| `cortex_adapter_envelopes_publish_fail_total` | counter | `kind` | Envelopes the publisher couldn't deliver — network failure or per-envelope ingestion rejection. |
| `cortex_adapter_publisher_accepted_total` | counter | — | Sum across kinds of the BatchReport.accepted values. |
| `cortex_adapter_publisher_queue_depth` | gauge | — | In-process bounded queue depth (best-effort). |
| `cortex_adapter_overflow_wal_bytes` | gauge | — | Bytes spilled to the on-disk WAL. |
| `cortex_adapter_last_publish_ok_ts_ms` | gauge | `kind` | Unix-epoch ms of the most recent successful ingestion publish per kind. |
| `cortex_adapter_ipc_pipe_alive` | gauge | — | `1` while the named pipe / Unix socket is bound, `0` otherwise. |

## cortex-ingestion (port 17010)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_events_received` | counter | — | Aggregate post-validation accept counter (phase8a). |
| `cortex_events_rejected` | counter | — | Aggregate validation-rejected counter. |
| `cortex_ingestion_events_received_total` | counter | `kind` | Per-kind durability counter — bumps before the archive write. |
| `cortex_ingestion_events_archived_total` | counter | `kind` | Per-kind archived counter — bumps after the archive writer accepts the envelope. |
| `cortex_ingestion_events_rejected_total` | counter | `reason` | Per-reason validation/archive/publisher rejection counter. Reason buckets: `invalid_json`, `archive_failure`, `publisher_failure`, `validation`. |
| `cortex_ingestion_last_archive_write_ts_ms` | gauge | `kind` | Unix-epoch ms of the most recent archive write per kind. |
| `cortex_last_batch_accepted_ts_ms` | gauge | — | Unix-epoch ms of the most recent successful batch accept. |
| `cortex_archive_errors` | counter | — | Archive write failures. |
| `cortex_publisher_errors` | counter | — | Synap publisher failures. |

## cortex-api (port 17000)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_archive_loader_last_refresh_ts_ms` | gauge | — | Unix-epoch ms of the most recent archive scan completion. |
| `cortex_archive_loader_envelopes_seeded_total` | counter | `kind` | Cumulative envelopes the archive loader has surfaced to the keyword lane, by canonical kind. |
| `cortex_meili_loader_last_refresh_ts_ms` | gauge | — | Unix-epoch ms of the most recent meili scan completion. |
| `cortex_meili_loader_docs_seeded_total` | counter | `family` | Cumulative docs surfaced from each meili family. Families: `decisions`, `violations`, `memories`, `analyses`, `turns`. |

## cortex-classifier-worker (port 17021)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_classifier_jobs_processed_total` | counter | — | Synap messages classified end-to-end. |
| `cortex_classifier_last_job_ts_ms` | gauge | — | Unix-epoch ms of the most recent successful job. |

## cortex-embedder-worker (port 17022)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_embedder_jobs_processed_total` | counter | — | Synap messages embedded end-to-end. |
| `cortex_embedder_last_job_ts_ms` | gauge | — | Unix-epoch ms of the most recent successful job. |
| `cortex_embedder_chunks_written_total` | counter | — | Sum of every per-source chunk counter. |
| `cortex_embedder_vectorizer_errors_total` | counter | — | HTTP-error count from the Vectorizer SDK. |
| `cortex_embedder_dedup_hits_total` | counter | — | Dedup short-circuits. |

## cortex-fulltext-worker (port 17023)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_fulltext_jobs_processed_total` | counter | — | Synap messages indexed end-to-end. |
| `cortex_fulltext_last_job_ts_ms` | gauge | — | Unix-epoch ms of the most recent successful job. |
| `cortex_fulltext_documents_total` | counter | — | Sum of every per-index document counter. |
| `cortex_fulltext_skipped_empty_total` | counter | — | Events dropped because body selection produced an empty string. |

## cortex-graph-worker (port 17024)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cortex_graph_jobs_processed_total` | counter | — | Synap messages written to the graph end-to-end. |
| `cortex_graph_last_job_ts_ms` | gauge | — | Unix-epoch ms of the most recent successful job. |
| `cortex_graph_edges_dropped_total` | counter | — | Sum of every per-type edges_dropped counter. |

## Aggregator endpoints (cortex-api)

| Endpoint | Returns |
|----------|---------|
| `GET /v1/health/freshness` | Flat array of `{ key, last_event_ts_ms, gap_seconds, severity }` rows keyed by `<stage>.<kind>`. |
| `GET /v1/health/divergence` | Adjacent-stage counter pairs as `{ pair, upstream, downstream, delta, delta_growth, severity }` rows. |

### Severity buckets

Both aggregator endpoints colour-code rows the GUI consumes:

- **Freshness** — `gap_seconds > 60` → `warn`; `gap_seconds > 300` → `critical`. `last_event_ts_ms == 0` (never observed) emits `gap_seconds = -1` and `severity = warn`.
- **Divergence** — `delta_growth > 10` → `warn`; `delta_growth > 50` → `critical`. The aggregator caches the previous probe's delta in process memory and only emits non-zero growth when ≥ 30 s have elapsed since the prior probe — that gives the operator a stable signal even on noisy scrape cadences.

### Canonical divergence pairs

| Pair | Detection |
|------|-----------|
| `adapter.frames_parsed -> adapter.envelopes_built` | Hooks fired but the dispatcher couldn't build an envelope (unknown hook kind / non-publishable). |
| `adapter.envelopes_built -> adapter.envelopes_publish_ok` | Queue drops, WAL spills, or BatchReport rejections. |
| `adapter.publish_ok.<kind> -> ingestion.archived.<kind>` | Per-kind silent drop between publish and archive durability. |

### Operator workflow

1. `scripts/health.sh` (or `.bat`) for the alive/dead picture (phase8a).
2. `curl http://127.0.0.1:17000/v1/health/freshness | jq` for "where did the data stop moving?".
3. `curl http://127.0.0.1:17000/v1/health/divergence | jq` for "which boundary is silently dropping?".

The 2026-04-28 JSON-truncation incident manifests as
`adapter.frames_parsed -> adapter.envelopes_built` carrying a
non-zero `delta_growth` within seconds of the truncation hitting,
instead of the ~2 h grep-the-logs trace that actually found it.

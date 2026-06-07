# Tool-call archive loss after `cortex-ingestion` hard-kill

**Date**: 2026-04-28
**Reporter**: operator (live timeline stopped showing this session's
`tool_call` rows mid-conversation)
**Severity**: high — silent data loss; surface looked healthy
(hooks `-> ok`, WAL=0, all PIDs alive, partition counts in Nexus
growing) but the dashboard stopped capturing real activity.

## TL;DR

Hard-killing `cortex-ingestion` with `taskkill /F` (or any signal that
bypasses graceful shutdown) leaves the tail of
`raw-00000.parquet` (zstd-NDJSON, despite the extension) in a
half-flushed state. When the next `cortex-ingestion` boots, it opens
the SAME file in append mode (`OpenOptions::create(true).append(true)`,
[`crates/cortex-ingestion/src/archive.rs:84`](../../../crates/cortex-ingestion/src/archive.rs#L84))
and starts a brand-new zstd stream concatenated onto the broken one.

The two consumers of the archive cannot both read past the corruption:

- `cortex-api`'s archive loader bails on the first
  `Data corruption detected` and only seeds the events that landed
  before the kill (recovery count `125 / 251` in this incident).
- `cortex-fulltext`'s `boot_replay` and `cortex-ops`'s archive probe
  use the same `zstd::stream::read::Decoder` and short-circuit
  identically.

End user effect: the timeline freezes at the last successful refresh's
high-water mark even though new events keep landing in the file —
they're just past the corruption boundary that the reader can't cross.

## Reproduction (observed sequence)

| t | event |
|---|---|
| 17:59 | `cortex-api` started. |
| 21:25 | operator killed `cortex-adapter-claude` (PID 29812) via `taskkill /F` to swap binaries. New adapter came up at 21:29:55. |
| 21:30:06 | last `tool_call` envelope from session `1E8BYVCG…` lands in `raw-00000.parquet`. |
| 21:51:15 | operator killed `cortex-ingestion` (PID 105988) and `cortex-api` (PID 143784) for a stack-wide rebuild. Both were sent `SIGKILL`-equivalent. |
| 21:51:22 | `cortex-api` restarted; archive loader logged `recovered_envelopes=125 error=io: Data corruption detected` against `hour=21/raw-00000.parquet`. |
| 21:51:51+ | `cortex-ingestion` restarted; new envelopes appended to the same `raw-00000.parquet`. Hooks kept firing `-> ok`, adapter publisher kept POSTing 202s, but `archive_loader::scan_file` could not parse past the boundary. |
| 22:01–22:07 | every new `tool_call` from this session landed in the file but stayed invisible to the dashboard. Operator manually rotated the file (`mv … raw-00000.parquet.corrupted-1907`); on the next ingestion-restart, a fresh `raw-00000.parquet` was created and tool_calls reappeared in the timeline within one refresh window (≤ 30s). |

## Why every layer's "I'm healthy" signal lied

The incident took ~2 hours to root-cause because every component looked
fine in isolation:

- **Plugin hooks** logged `-> ok` to `~/.cortex/hook-invocations.log`
  for every `PreToolUse` / `PostToolUse`. The pipe write+read
  round-tripped successfully — the adapter accepted the frame and
  responded `{}`.
- **Adapter `cortex-adapter-claude` publisher** spilled to
  `~/.cortex/overflow.wal` only on transport-level failure. Once
  `cortex-ingestion` was reachable again, the WAL drained to `0` and
  every subsequent batch returned `202 Accepted`.
- **`cortex-ingestion` validates per envelope and returns
  `{"accepted":N,"rejected":M,"errors":[…]}` inside a `202 Accepted`
  response.** The publisher's `post_batch`
  ([`crates/cortex-adapter-claude-code/src/publisher.rs:181`](../../../crates/cortex-adapter-claude-code/src/publisher.rs#L181))
  treats *any* 2xx as success. So an envelope that was rejected at
  the schema layer was indistinguishable from one that landed.
- **`cortex-api`** kept refreshing the archive every 30s and looked
  alive on `/v1/status`; the `archive_loader` `partial frame (live
  file or trailing corruption)` log line is `DEBUG` (silenced under
  the default `RUST_LOG=info`). `recovered_envelopes` flat-lined but
  no operator-visible signal fired.
- **Dashboard timeline** rendered the cached lane (no SSE failure,
  no 5xx, no banner). The user only noticed because subjectively
  the latest timestamp stopped advancing.

Three independent fail-quietly seams stacked: zstd corruption silent
beyond the first occurrence, ingestion's per-envelope rejection
hidden inside a 202 body, and the archive loader's debug-only
diagnostics.

## Proximate code path

```
cortex-adapter-claude (publisher)
  POST /v1/events/batch  ─►  cortex-ingestion router
                              redact + validate (cortex-core::events)
                              archive.write(&envelope)
                                ensure_open(stream_tag, ts)
                                  OpenOptions::create(true).append(true)
                                  zstd::Encoder::new(BufWriter::new(file)).auto_finish()
                                encoder.write_all(canonical_ndjson_bytes)
```

[`crates/cortex-ingestion/src/archive.rs::ensure_open`](../../../crates/cortex-ingestion/src/archive.rs#L66-L88):

```rust
let filename = archive_filename(stream_tag, 0);
let path = dir.join(&filename);
let file = OpenOptions::new().create(true).append(true).open(&path)?;
let encoder = zstd::Encoder::new(BufWriter::new(file), self.level)?.auto_finish();
```

`archive_filename` is hard-coded to sequence `0`, so there is exactly
one file per `(stream_tag, hour_bucket)` and the hour file is reused
across process restarts. Append mode + abrupt termination = the
trailing zstd frame of the previous run is the leading bytes of the
next reader's stream.

## Why the readers can't recover

`zstd` framing supports concatenated streams in principle, but the
Rust binding's `zstd::stream::read::Decoder` and the upstream Python
`zstandard` library both surface `Data corruption detected` on a
truncated frame and stop. There is no API for "skip this frame, try
the next stream header". The Rust archive loader could in theory
walk the file looking for the next `28 B5 2F FD` magic, but today it
simply returns the partial-recovery counter and moves on.

## Fix

Two complementary changes — implement both:

### 1. Rotate archive sequence on writer startup (cortex-ingestion)

Make `ensure_open` pick the **next-free** sequence number when it
materialises a fresh `(stream_tag, hour_bucket)` writer, instead of
always picking `0`. The first run lands in `raw-00000.parquet`; every
subsequent restart that wants to write into the same hour starts a
new `raw-NNNNN.parquet`. Existing readers already glob every
`*.parquet` in the partition directory, so the dashboard simply sees
`raw-00000.parquet` (frozen, possibly corrupt-tail) **and**
`raw-00001.parquet` (clean, current) side by side.

Pseudocode (replaces lines 80-86 in
[archive.rs](../../../crates/cortex-ingestion/src/archive.rs)):

```rust
let dir = archive_partition(&self.root, ts);
std::fs::create_dir_all(&dir)?;
let sequence = next_free_sequence(&dir, stream_tag)?;
let filename = archive_filename(stream_tag, sequence);
let path = dir.join(&filename);
let file = OpenOptions::new().create(true).append(true).open(&path)?;
```

`next_free_sequence` walks `dir` for `*-NNNNN.parquet` files matching
`stream_tag`, parses the sequence numbers, and returns
`max(existing) + 1` (or `0` when none exist). Effect: graceful
shutdown can stay in-place (encoder.flush completes the frame
cleanly), but a hard kill never poisons the file the next process
opens.

This is also the right wire shape for the spec-04 "rotates when files
reach a size cap" semantics already documented at
[`crates/cortex-storage/src/archive.rs:53`](../../../crates/cortex-storage/src/archive.rs#L53)
— that doc-comment already promises sequence rotation; the writer
just never honoured it.

### 2. Tolerant archive reader (cortex-api / cortex-ops / cortex-fulltext)

Even with the writer fix, in-place corruption from older runs
(everyone's existing `hour=*/raw-00000.parquet` files) still blocks
the loader. Patch `scan_file` (and its three siblings) to:

1. Drain the current `Decoder<'_, BufReader<File>>` until first
   error.
2. On error, log at `WARN` (not `DEBUG`) with `recovered_envelopes`
   and `bytes_skipped`.
3. **Do not** continue past the first corrupt frame. Multi-stream
   zstd recovery is fragile; the writer-side fix is what we lean on.

Bumping the log level to `WARN` is the only behaviour change the
dashboard operator needs — corruption stops being silent. The
`recovered_envelopes` counter is already there.

### 3. Wire a health probe (phase8a)

The phase8a `cortex-health` shared crate (already in flight at
[`crates/cortex-health/`](../../../crates/cortex-health/)) gives us
the right contract for surfacing this immediately. Two specific
extras that would have caught this in seconds:

- `cortex-ingestion /v1/healthz`:
  - `extras.last_archive_corruption_warn_ts` — bumped whenever
    `scan_file` (or the writer's flush path) hits an error.
  - `extras.archive_open_paths` — the currently-open
    `raw-NNNNN.parquet` per `(stream_tag, hour)`.
- `cortex-api /healthz`:
  - `extras.archive_loader_corrupted_files` — list of hour
    partitions whose last refresh recovered fewer envelopes than the
    file size implies.
  - `extras.last_archive_loader_refresh` — Δ since the last refresh
    that loaded ≥ 1 new envelope. When a hot session is producing
    tool_calls but no new hits land, this Δ grows past the configured
    refresh interval and the subsystem flips to `degraded`.

The aggregator at `GET /v1/health` rolls these into one
`HealthReport` so a single curl + grep tells the operator "ingestion
is healthy but the api side stopped seeing fresh envelopes" — which
is exactly the diagnostic that would have cut today's investigation
from two hours to thirty seconds.

## Recurrence prevention

| Action | Owner | Status |
|---|---|---|
| Implement #1 (rotate-on-open) in `cortex-ingestion` | this PR | ✅ |
| Implement #2 (loud-warn on corruption) in `archive_loader` | this PR | ✅ |
| Phase8a health endpoints + aggregator + script | tracked at [`phase8a_health_endpoints_per_crate`](../../../.rulebook/tasks/phase8a_health_endpoints_per_crate/) | in progress |
| Move all existing `*.corrupted*` files into a quarantine directory under `~/.cortex/archive/.quarantine/` so the loader stops scanning them on every refresh | follow-up | not started |
| Adapter publisher: parse the `accepted/rejected` body of the 202 response, surface `cortex_adapter.publisher.rejected_total{reason}` metric | follow-up | not started |
| Hook-side `Pipe is broken` counter (see §Sibling failure mode) | follow-up | not started |

## Sibling failure mode — `Pipe is broken` on the hook side

Observed 2026-04-28 22:20 (same session, post-fix):

```
2026-04-28T22:20:19.428Z PostToolUse env_sid= payload_sid=d80f15ce-…
2026-04-28T22:20:19.428Z PostToolUse  -> err: Exception calling "Flush" with "0" argument(s): "Pipe is broken."
```

The pwsh hook in
[`packages/cortex-claude-plugin/hooks/cortex-post-tool.ps1`](../../../packages/cortex-claude-plugin/hooks/cortex-post-tool.ps1)
opens a fresh `NamedPipeClientStream` per invocation, writes the frame,
calls `Flush()`. When two hooks fire concurrently (e.g. PreToolUse +
PostToolUse for back-to-back tool calls), the
single-instance named pipe contention drops one of the connections —
the surviving hook writes `-> err: Pipe is broken` and the envelope is
**lost**. There is no WAL on the hook side: the WAL only protects
events that already reached the adapter's queue.

Failure rate observed during the 2026-04-28 incident: roughly 1 in 30
PostToolUse invocations. Drops are individually small but cumulative
loss is real and silent against the live timeline.

Mitigation candidates (each is its own follow-up):

1. **Hook retry loop.** Catch the `IOException` in the pwsh hook, sleep
   25–100 ms, reconnect, write once more. Bounded retry (≤ 2) keeps the
   5 s hook timeout intact while collapsing the contention window.
2. **Multi-instance pipe.** Bind the adapter's pipe with
   `MaxNumberOfServerInstances > 1` (the d434701 commit notes already
   call this out as the suspected silent-fail cause). Doesn't fix the
   write-side flush race but reduces the connect-side collisions.
3. **Hook-side mirror to a local rolling log.** Frame goes to the pipe
   AND to `~/.cortex/hook-events.ndjson` synchronously; an adapter
   side-task drains the rolling log on startup. This makes hook-side
   loss zero — at the cost of a per-hook disk write.

Only option 3 closes the silent-loss class entirely. Options 1 + 2 are
both worth applying in the meantime; they're cheap and they cut the
observed failure rate.

Today's signal for the operator is `grep "-> err" ~/.cortex/hook-invocations.log`.
Phase8a's adapter `/healthz` should expose `extras.hook_pipe_broken_5m`
so the dashboard can flag the rate without manual log greps.

## Lessons

1. **Append-mode archive writers + abrupt termination = corrupt
   tails.** Choose: rotate on every open (cheap, always safe) or
   write a footer the reader can skip past (complex, fragile). We
   went with rotate.
2. **A 202 body that carries a rejection list is not a 2xx for
   "everything landed."** The publisher should parse the accepted
   counter; treating "the request hit the wire" as success masks
   schema drift.
3. **Debug-level diagnostics for hot-path corruption is the wrong
   default.** Promote anything that signals data loss to WARN and
   surface a counter. The whole point of having archives is so future
   queries land — silently dropping rows defeats it.
4. **Health monitoring needs a freshness signal, not just liveness.**
   Every component in this chain answered "I'm alive." None
   answered "I'm currently making forward progress." Phase8a's
   `last_*_ts` extras are the right shape.

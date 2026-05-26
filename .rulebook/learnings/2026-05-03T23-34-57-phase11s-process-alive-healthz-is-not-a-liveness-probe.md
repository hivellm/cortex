# phase11s — process-alive /healthz is not a liveness probe
**Source**: manual
**Date**: 2026-05-03
**Related Task**: phase11s_pipeline_drainage_recovery
**Tags**: phase11s, liveness, healthz, supervisor, consumer, watchdog
During phase11s §1, the classifier-worker stayed `/healthz`-green for 26 hours while its consume loop was DEAD. The worker's process was alive (TCP socket bound, readiness probe passing) but the inner loop had been retrying a single `synap consume` call with a 500ms back-off + `continue` since `2026-05-02T01:59:02Z`. Operators only noticed when zero `cortex.events.classified` envelopes shipped for a day.

The structural lesson: **a `/healthz` endpoint that reports "process alive" is NOT a liveness probe**. It must observe **forward progress**:

- For consumers: the timestamp of the last successful `consume` return (NOT the last successful job, because empty polls still represent forward progress).
- For producers: the timestamp of the last successful publish.
- For workers with both: the LATER of the two — whichever lane is most idle.

The §1.3 fix adds `last_consume_ts_ms` distinct from the existing `last_job_ts_ms`. The freshness signal driving the `/healthz` `state` field flips to the consume timestamp because: a worker draining empty batches all day still reports green via `last_consume_ts_ms`, but if Synap dies the consume timestamp ages immediately and the doctor flags it.

Pattern that works:

```rust
// Inside run_forever's main loop:
match self.run_once().await {
    Ok(_) => {
        self.consume_errors_consecutive.store(0, Ordering::Relaxed);
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_consume_ts_ms.store(now_ms, Ordering::Relaxed);
    }
    Err(e) => {
        let consecutive = self.consume_errors_consecutive.fetch_add(1, Ordering::Relaxed) + 1;
        if consecutive >= self.max_consume_errors {
            return Err(anyhow!("consume loop stuck: {consecutive} errors"));
        }
        // back-off + continue
    }
}
```

Anti-pattern (the regression this caught): Look at `process::is_alive()` → green → assume the worker is processing events.

The same lesson generalises beyond Synap consumers: any long-running async loop where `Err → continue` is the only error-handling path will silently survive an unrecoverable upstream failure. The supervisor exit-on-N-consecutive-errors pattern is the load-bearing fix; the `last_*_ts` liveness probe is the diagnostic surface that lets operators see the failure BEFORE the supervisor trips.

Worth applying to: graph-worker (already adds `last_job_ts_ms` but misses `last_consume_ts_ms` for the same reason; future phase), fulltext indexer (same consume loop shape), embedder (the JWT path has a similar dormancy class — every embed call returning 401 isn't a "loop crash" but it's effectively dead).
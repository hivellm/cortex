//! `cortex.adapter.*` counters / histograms (spec 10 §Observability).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Adapter-side metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.adapter.events.total{kind}`.
    pub events_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.events.dropped{reason}`.
    pub events_dropped: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.sync.latency_ms{hook}` — histogram observations.
    pub sync_latency_ms: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.adapter.sync.timeouts{hook}`.
    pub sync_timeouts: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.publisher.errors{status}`.
    pub publisher_errors: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.publisher.rejected{reason}` — events the
    /// ingestion side returned in the 202 response body's `errors`
    /// list. Tracked per error class so silent schema drift surfaces
    /// in the metrics.
    pub publisher_rejected: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.publisher.accepted_total` — envelopes the
    /// ingestion side reported as accepted in the 202 body. Bumps per
    /// successful batch only after we parse the response shape.
    pub publisher_accepted: AtomicU64,
    /// `cortex.adapter.pre_thinking.bundle_bytes` — histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.adapter.laws.blocks{law_id}`.
    pub law_blocks: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.overflow.wal_bytes` — gauge sample.
    pub wal_bytes: AtomicU64,
    /// Phase8a — Unix-epoch ms of the most recent successful publish.
    /// `0` until the first envelope ships. `/healthz` reads this to
    /// detect publisher stall and downgrade to `degraded`.
    pub last_publish_ok_ts_ms: AtomicU64,
    /// Phase8a — current publisher queue depth (in-memory bounded
    /// channel between hook callbacks and the HTTP publisher).
    pub publisher_queue_depth: AtomicU64,
    /// Phase8a — `1` while the IPC pipe (named pipe / Unix socket)
    /// is bound + accepting connections, `0` when the listener has
    /// torn down.
    pub ipc_pipe_alive: AtomicU64,
    /// Phase8b — `cortex_adapter_ipc_frames_received_total{hook}`.
    /// Bumped on every `handle_line` entry, before the JSON parses.
    /// Pair with `frames_parsed_total` to detect malformed inputs.
    pub frames_received_total: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_adapter_ipc_frames_parsed_total{hook}`. The
    /// frame's hook string is only known once the JSON parses, so a
    /// frames-received vs frames-parsed gap localises parse drops.
    pub frames_parsed_total: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_adapter_ipc_frames_parse_error_total`. Bumps
    /// once per malformed payload (we never see the hook label).
    pub frames_parse_error_total: AtomicU64,
    /// Phase8b — `cortex_adapter_envelopes_built_total{kind}`. Bumped
    /// from the dispatcher right after `build_event` returns
    /// `Some(envelope)`.
    pub envelopes_built_total: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_adapter_envelopes_publish_ok_total{kind}`.
    /// Bumped per ingestion-accepted envelope (BatchReport.accepted).
    pub envelopes_publish_ok_total: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_adapter_envelopes_publish_fail_total{kind}`.
    /// Bumped per envelope the publisher couldn't deliver
    /// (network failure or BatchReport.errors entry).
    pub envelopes_publish_fail_total: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `last_frame_ts_ms{hook}` — Unix-epoch ms of the most
    /// recent successfully parsed hook frame, per hook label.
    pub last_frame_ts_ms: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `last_envelope_ts_ms{kind}` — Unix-epoch ms of the
    /// most recent envelope built by the dispatcher, per kind.
    pub last_envelope_ts_ms: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `last_publish_ok_ts_ms{kind}` — Unix-epoch ms of the
    /// most recent successful ingestion publish, per kind. The
    /// scalar `last_publish_ok_ts_ms` (phase8a) summarises across
    /// kinds; this map keeps the per-kind breakdown the freshness
    /// aggregator pivots on.
    pub last_publish_ok_ts_ms_by_kind: Mutex<BTreeMap<String, u64>>,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment `events_total{kind}`.
    pub fn incr_events_total(&self, kind: &str) {
        if let Ok(mut m) = self.events_total.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `events_dropped{reason}`.
    pub fn incr_dropped(&self, reason: &str) {
        if let Ok(mut m) = self.events_dropped.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a sync hook latency in milliseconds.
    pub fn observe_sync_latency(&self, hook: &str, ms: u32) {
        if let Ok(mut m) = self.sync_latency_ms.lock() {
            m.entry(hook.to_string()).or_default().push(ms);
        }
    }
    /// Increment `sync.timeouts{hook}`.
    pub fn incr_sync_timeout(&self, hook: &str) {
        if let Ok(mut m) = self.sync_timeouts.lock() {
            *m.entry(hook.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `publisher.errors{status}`.
    pub fn incr_publisher_error(&self, status: &str) {
        if let Ok(mut m) = self.publisher_errors.lock() {
            *m.entry(status.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `publisher.rejected{reason}` — bumped per envelope
    /// the ingestion 202 response body lists in `errors[]`.
    pub fn incr_publisher_rejected(&self, reason: &str) {
        if let Ok(mut m) = self.publisher_rejected.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    /// Add `n` to the `publisher.accepted_total` counter.
    pub fn add_publisher_accepted(&self, n: u64) {
        self.publisher_accepted.fetch_add(n, Ordering::Relaxed);
    }
    /// Read `publisher.accepted_total`.
    pub fn publisher_accepted(&self) -> u64 {
        self.publisher_accepted.load(Ordering::Relaxed)
    }
    /// Record a pre-thinking bundle size in bytes.
    pub fn observe_bundle_bytes(&self, bytes: u32) {
        if let Ok(mut g) = self.bundle_bytes.lock() {
            g.push(bytes);
        }
    }
    /// Increment `laws.blocks{law_id}`.
    pub fn incr_law_block(&self, law_id: &str) {
        if let Ok(mut m) = self.law_blocks.lock() {
            *m.entry(law_id.to_string()).or_insert(0) += 1;
        }
    }
    /// Set the `overflow.wal_bytes` gauge.
    pub fn set_wal_bytes(&self, bytes: u64) {
        self.wal_bytes.store(bytes, Ordering::Relaxed);
    }
    /// Read the WAL gauge.
    pub fn wal_bytes(&self) -> u64 {
        self.wal_bytes.load(Ordering::Relaxed)
    }

    /// Phase8a — stamp `last_publish_ok_ts_ms` to the current
    /// Unix-epoch ms. Called from the publisher's success path.
    pub fn record_publish_ok_now(&self) {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_publish_ok_ts_ms.store(now_ms, Ordering::Relaxed);
    }
    /// Phase8a — read the most recent successful-publish timestamp.
    /// `0` means the publisher has never succeeded since boot.
    pub fn last_publish_ok_ts_ms(&self) -> u64 {
        self.last_publish_ok_ts_ms.load(Ordering::Relaxed)
    }
    /// Phase8a — set the current queue depth gauge.
    pub fn set_publisher_queue_depth(&self, depth: u64) {
        self.publisher_queue_depth.store(depth, Ordering::Relaxed);
    }
    /// Phase8a — read the queue depth gauge.
    pub fn publisher_queue_depth(&self) -> u64 {
        self.publisher_queue_depth.load(Ordering::Relaxed)
    }
    /// Phase8a — flip the IPC-pipe-alive flag.
    pub fn set_ipc_pipe_alive(&self, alive: bool) {
        self.ipc_pipe_alive
            .store(if alive { 1 } else { 0 }, Ordering::Relaxed);
    }
    /// Phase8a — read the IPC-pipe-alive flag.
    pub fn ipc_pipe_alive(&self) -> bool {
        self.ipc_pipe_alive.load(Ordering::Relaxed) == 1
    }

    fn now_ms() -> u64 {
        chrono::Utc::now().timestamp_millis().max(0) as u64
    }

    /// Phase8b — bump `frames_received_total{hook}`. Called from the
    /// IPC handler at the moment a line lands, BEFORE JSON parsing —
    /// the divergence between received and parsed is the smoking gun
    /// for the 2026-04-28 truncation incident.
    pub fn incr_frames_received(&self, hook: &str) {
        if let Ok(mut m) = self.frames_received_total.lock() {
            *m.entry(hook.to_string()).or_insert(0) += 1;
        }
    }
    /// Phase8b — bump `frames_parsed_total{hook}` and stamp
    /// `last_frame_ts_ms{hook}` with the current wall-clock ms.
    pub fn incr_frames_parsed(&self, hook: &str) {
        if let Ok(mut m) = self.frames_parsed_total.lock() {
            *m.entry(hook.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut m) = self.last_frame_ts_ms.lock() {
            m.insert(hook.to_string(), Self::now_ms());
        }
    }
    /// Phase8b — bump the parse-error counter. We don't know the hook
    /// label at this point because the JSON didn't deserialize, so the
    /// counter is unlabelled — same shape `cortex-ingestion` uses for
    /// `events_rejected_total`.
    pub fn incr_frames_parse_error(&self) {
        self.frames_parse_error_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Phase8b — bump `envelopes_built_total{kind}` and stamp
    /// `last_envelope_ts_ms{kind}` with the current wall-clock ms.
    pub fn incr_envelopes_built(&self, kind: &str) {
        if let Ok(mut m) = self.envelopes_built_total.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut m) = self.last_envelope_ts_ms.lock() {
            m.insert(kind.to_string(), Self::now_ms());
        }
    }
    /// Phase8b — bump `envelopes_publish_ok_total{kind}` and stamp
    /// `last_publish_ok_ts_ms_by_kind{kind}`.
    pub fn incr_envelopes_publish_ok(&self, kind: &str) {
        if let Ok(mut m) = self.envelopes_publish_ok_total.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut m) = self.last_publish_ok_ts_ms_by_kind.lock() {
            m.insert(kind.to_string(), Self::now_ms());
        }
    }
    /// Phase8b — bump `envelopes_publish_fail_total{kind}`.
    pub fn incr_envelopes_publish_fail(&self, kind: &str) {
        if let Ok(mut m) = self.envelopes_publish_fail_total.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
    }

    /// Phase8b — snapshot `frames_received_total` (test/dashboard helper).
    pub fn frames_received_snapshot(&self) -> BTreeMap<String, u64> {
        self.frames_received_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `frames_parsed_total`.
    pub fn frames_parsed_snapshot(&self) -> BTreeMap<String, u64> {
        self.frames_parsed_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — read the cumulative parse-error counter.
    pub fn frames_parse_error_total(&self) -> u64 {
        self.frames_parse_error_total.load(Ordering::Relaxed)
    }
    /// Phase8b — snapshot `envelopes_built_total`.
    pub fn envelopes_built_snapshot(&self) -> BTreeMap<String, u64> {
        self.envelopes_built_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `envelopes_publish_ok_total`.
    pub fn envelopes_publish_ok_snapshot(&self) -> BTreeMap<String, u64> {
        self.envelopes_publish_ok_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `envelopes_publish_fail_total`.
    pub fn envelopes_publish_fail_snapshot(&self) -> BTreeMap<String, u64> {
        self.envelopes_publish_fail_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot the per-hook `last_frame_ts_ms` map.
    pub fn last_frame_ts_snapshot(&self) -> BTreeMap<String, u64> {
        self.last_frame_ts_ms
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot the per-kind `last_envelope_ts_ms` map.
    pub fn last_envelope_ts_snapshot(&self) -> BTreeMap<String, u64> {
        self.last_envelope_ts_ms
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot the per-kind `last_publish_ok_ts_ms` map.
    pub fn last_publish_ok_ts_by_kind_snapshot(&self) -> BTreeMap<String, u64> {
        self.last_publish_ok_ts_ms_by_kind
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Phase8b — render the metrics in Prometheus text format. Used by
    /// the admin `/metrics` endpoint mounted on the same listener as
    /// `/healthz`. Deliberately covers the per-stage divergence
    /// counters first; static gauges (queue depth, WAL bytes) come
    /// after so a casual `grep frames_` lands on the smoking-gun pair.
    pub fn render_prom(&self) -> String {
        let mut out = String::new();

        out.push_str("# TYPE cortex_adapter_ipc_frames_received_total counter\n");
        for (hook, n) in self.frames_received_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_ipc_frames_received_total{{hook=\"{hook}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_adapter_ipc_frames_parsed_total counter\n");
        for (hook, n) in self.frames_parsed_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_ipc_frames_parsed_total{{hook=\"{hook}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_adapter_ipc_frames_parse_error_total counter\n");
        let _ = writeln!(
            out,
            "cortex_adapter_ipc_frames_parse_error_total {}",
            self.frames_parse_error_total()
        );

        out.push_str("# TYPE cortex_adapter_envelopes_built_total counter\n");
        for (kind, n) in self.envelopes_built_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_envelopes_built_total{{kind=\"{kind}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_adapter_envelopes_publish_ok_total counter\n");
        for (kind, n) in self.envelopes_publish_ok_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_envelopes_publish_ok_total{{kind=\"{kind}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_adapter_envelopes_publish_fail_total counter\n");
        for (kind, n) in self.envelopes_publish_fail_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_envelopes_publish_fail_total{{kind=\"{kind}\"}} {n}"
            );
        }

        out.push_str("# TYPE cortex_adapter_last_frame_ts_ms gauge\n");
        for (hook, ms) in self.last_frame_ts_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_last_frame_ts_ms{{hook=\"{hook}\"}} {ms}"
            );
        }
        out.push_str("# TYPE cortex_adapter_last_envelope_ts_ms gauge\n");
        for (kind, ms) in self.last_envelope_ts_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_last_envelope_ts_ms{{kind=\"{kind}\"}} {ms}"
            );
        }
        out.push_str("# TYPE cortex_adapter_last_publish_ok_ts_ms gauge\n");
        for (kind, ms) in self.last_publish_ok_ts_by_kind_snapshot() {
            let _ = writeln!(
                out,
                "cortex_adapter_last_publish_ok_ts_ms{{kind=\"{kind}\"}} {ms}"
            );
        }

        out.push_str("# TYPE cortex_adapter_publisher_accepted_total counter\n");
        let _ = writeln!(
            out,
            "cortex_adapter_publisher_accepted_total {}",
            self.publisher_accepted()
        );
        out.push_str("# TYPE cortex_adapter_publisher_queue_depth gauge\n");
        let _ = writeln!(
            out,
            "cortex_adapter_publisher_queue_depth {}",
            self.publisher_queue_depth()
        );
        out.push_str("# TYPE cortex_adapter_overflow_wal_bytes gauge\n");
        let _ = writeln!(out, "cortex_adapter_overflow_wal_bytes {}", self.wal_bytes());
        out.push_str("# TYPE cortex_adapter_ipc_pipe_alive gauge\n");
        let _ = writeln!(
            out,
            "cortex_adapter_ipc_pipe_alive {}",
            u64::from(self.ipc_pipe_alive())
        );

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_per_kind_maps_are_empty() {
        let m = Metrics::new();
        assert!(m.frames_received_snapshot().is_empty());
        assert!(m.frames_parsed_snapshot().is_empty());
        assert!(m.envelopes_built_snapshot().is_empty());
        assert!(m.envelopes_publish_ok_snapshot().is_empty());
        assert!(m.envelopes_publish_fail_snapshot().is_empty());
        assert_eq!(m.frames_parse_error_total(), 0);
    }

    #[test]
    fn frames_received_increments_per_hook() {
        let m = Metrics::new();
        m.incr_frames_received("PostToolUse");
        m.incr_frames_received("PostToolUse");
        m.incr_frames_received("UserPromptSubmit");
        let snap = m.frames_received_snapshot();
        assert_eq!(snap.get("PostToolUse"), Some(&2));
        assert_eq!(snap.get("UserPromptSubmit"), Some(&1));
    }

    #[test]
    fn frames_parsed_stamps_last_frame_ts() {
        let m = Metrics::new();
        let before = chrono::Utc::now().timestamp_millis().max(0) as u64;
        m.incr_frames_parsed("PostToolUse");
        let after = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let snap = m.last_frame_ts_snapshot();
        let ts = *snap.get("PostToolUse").expect("hook key present");
        assert!(ts >= before && ts <= after);
    }

    #[test]
    fn envelopes_built_increments_and_stamps_last_envelope_ts() {
        let m = Metrics::new();
        m.incr_envelopes_built("tool_call");
        m.incr_envelopes_built("tool_call");
        m.incr_envelopes_built("turn");
        let by_kind = m.envelopes_built_snapshot();
        assert_eq!(by_kind.get("tool_call"), Some(&2));
        assert_eq!(by_kind.get("turn"), Some(&1));
        let ts = m.last_envelope_ts_snapshot();
        assert!(ts.contains_key("tool_call"));
        assert!(ts.contains_key("turn"));
    }

    #[test]
    fn publish_ok_and_fail_pivots_per_kind() {
        let m = Metrics::new();
        m.incr_envelopes_publish_ok("tool_call");
        m.incr_envelopes_publish_ok("tool_call");
        m.incr_envelopes_publish_fail("turn");
        assert_eq!(m.envelopes_publish_ok_snapshot().get("tool_call"), Some(&2));
        assert_eq!(m.envelopes_publish_fail_snapshot().get("turn"), Some(&1));
        assert!(m.last_publish_ok_ts_by_kind_snapshot().contains_key("tool_call"));
    }

    #[test]
    fn parse_error_counter_is_unlabelled_and_atomic() {
        let m = Metrics::new();
        m.incr_frames_parse_error();
        m.incr_frames_parse_error();
        assert_eq!(m.frames_parse_error_total(), 2);
    }

    #[test]
    fn render_prom_includes_per_hook_and_per_kind_rows() {
        let m = Metrics::new();
        m.incr_frames_received("PostToolUse");
        m.incr_frames_parsed("PostToolUse");
        m.incr_envelopes_built("tool_call");
        m.incr_envelopes_publish_ok("tool_call");
        m.incr_envelopes_publish_fail("turn");
        m.incr_frames_parse_error();
        let txt = m.render_prom();
        assert!(txt.contains(
            "cortex_adapter_ipc_frames_received_total{hook=\"PostToolUse\"} 1"
        ));
        assert!(txt.contains(
            "cortex_adapter_ipc_frames_parsed_total{hook=\"PostToolUse\"} 1"
        ));
        assert!(txt.contains("cortex_adapter_ipc_frames_parse_error_total 1"));
        assert!(txt.contains(
            "cortex_adapter_envelopes_built_total{kind=\"tool_call\"} 1"
        ));
        assert!(txt.contains(
            "cortex_adapter_envelopes_publish_ok_total{kind=\"tool_call\"} 1"
        ));
        assert!(txt.contains(
            "cortex_adapter_envelopes_publish_fail_total{kind=\"turn\"} 1"
        ));
        assert!(txt.contains("cortex_adapter_last_frame_ts_ms{hook=\"PostToolUse\"}"));
    }
}

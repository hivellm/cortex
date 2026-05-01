//! Phase11i §5.2 — long-running tail watcher.
//!
//! Polls the configured root every `poll_interval` and re-reads any
//! session file whose `(mtime, len)` advanced since the last tick.
//! Emitted envelopes are deduped via an in-memory `event_id` set so
//! a re-read of an unchanged session is a no-op (the cost is the
//! re-parse only, which keeps the watcher honest under restart).
//!
//! An axum HTTP server runs alongside the polling loop and exposes
//! `GET /healthz` returning a JSON snapshot of the watcher's
//! progress: last flush timestamp, files watched, envelope rate
//! over the most recent sample window, process RSS in bytes, total
//! emitted / dropped envelope counts, and uptime in ms.
//!
//! Cross-platform notes:
//! - `notify-rs` / inotify / `ReadDirectoryChangesW` are deliberately
//!   avoided. A 1 Hz polling cadence is cheap (mtime stat is O(N) in
//!   the file count and N is bounded by the corpus size — 1 K files
//!   at most) and sidesteps the platform-specific event-loss modes
//!   that bite long-running watchers in production.
//! - RSS is read via the `sysinfo` crate so Linux + macOS + Windows
//!   are covered without per-platform syscalls.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::{map_session, read_records, walk, Emitter, WalkConfig, WalkKind};

/// Default polling cadence — one tick per second strikes a balance
/// between latency to the dashboard and wasted stat() syscalls on
/// quiescent corpora.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(1_000);

/// Default bind address for the watcher's health endpoint. Matches
/// the §5.1 docker-compose port mapping.
pub const DEFAULT_HEALTH_BIND: &str = "0.0.0.0:17030";

/// Snapshot returned by `GET /healthz`. Fields match phase11i §5.2's
/// contract verbatim.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HealthSnapshot {
    /// Epoch ms of the most recent emitter flush. `0` when no tick
    /// has flushed yet.
    pub last_flush_ts: i64,
    /// Number of `.jsonl` session files the watcher saw on its
    /// most recent walk.
    pub files_watched: usize,
    /// Envelopes emitted per second over the most recent sample
    /// window. `0.0` on the very first tick (no baseline yet).
    pub envelope_rate: f64,
    /// Process RSS in bytes (via `sysinfo`). `0` when the platform
    /// sampler returns no value.
    pub rss_bytes: u64,
    /// Cumulative envelopes emitted since the watcher started.
    pub envelopes_emitted: u64,
    /// Cumulative envelopes dropped (mapper / emitter rejections).
    pub envelopes_dropped: u64,
    /// Wall-clock ms since the watcher started.
    pub uptime_ms: u64,
    /// Watcher status — `"healthy"` when the most recent tick
    /// completed successfully, `"degraded"` when the last tick
    /// surfaced a parse / IO error.
    pub status: String,
}

/// Concurrent state shared by the polling loop and the axum
/// handler. Atomics on the hot fields keep the read path lock-free;
/// the rate sample uses a tokio RwLock so the polling loop can
/// update both `(ts_ms, envelopes_emitted)` atomically.
#[derive(Debug)]
pub struct TailState {
    started_at_ms: i64,
    last_flush_ts: AtomicI64,
    files_watched: AtomicUsize,
    envelopes_emitted: AtomicU64,
    envelopes_dropped: AtomicU64,
    rss_bytes: AtomicU64,
    rate_sample: RwLock<RateSample>,
    status: RwLock<TailStatus>,
}

#[derive(Debug, Clone, Copy, Default)]
struct RateSample {
    last_ts_ms: i64,
    last_envelopes: u64,
    rate_per_sec: f64,
}

#[derive(Debug, Clone, Default)]
struct TailStatus {
    last_error: Option<String>,
    healthy: bool,
}

impl TailState {
    /// Build a fresh state stamped with `now_ms` as the start time.
    pub fn new(now_ms: i64) -> Self {
        Self {
            started_at_ms: now_ms,
            last_flush_ts: AtomicI64::new(0),
            files_watched: AtomicUsize::new(0),
            envelopes_emitted: AtomicU64::new(0),
            envelopes_dropped: AtomicU64::new(0),
            rss_bytes: AtomicU64::new(0),
            rate_sample: RwLock::new(RateSample::default()),
            status: RwLock::new(TailStatus {
                last_error: None,
                healthy: true,
            }),
        }
    }

    /// Update the monotonically-increasing emitted / dropped counts
    /// AND recompute the per-second rate against the previous sample.
    /// Returns the freshly-computed rate so the caller can log it.
    pub async fn record_tick(
        &self,
        now_ms: i64,
        emitted_total: u64,
        dropped_total: u64,
        files_watched: usize,
        rss_bytes: u64,
    ) -> f64 {
        self.envelopes_emitted
            .store(emitted_total, Ordering::Relaxed);
        self.envelopes_dropped
            .store(dropped_total, Ordering::Relaxed);
        self.files_watched.store(files_watched, Ordering::Relaxed);
        self.rss_bytes.store(rss_bytes, Ordering::Relaxed);
        self.last_flush_ts.store(now_ms, Ordering::Relaxed);

        let mut sample = self.rate_sample.write().await;
        let rate = if sample.last_ts_ms == 0 || now_ms <= sample.last_ts_ms {
            0.0
        } else {
            let dt_secs = (now_ms - sample.last_ts_ms) as f64 / 1_000.0;
            let delta = emitted_total.saturating_sub(sample.last_envelopes) as f64;
            if dt_secs > 0.0 {
                delta / dt_secs
            } else {
                0.0
            }
        };
        sample.last_ts_ms = now_ms;
        sample.last_envelopes = emitted_total;
        sample.rate_per_sec = rate;
        rate
    }

    /// Mark the tick degraded with the given error message. Cleared
    /// by the next successful `record_tick`.
    pub async fn record_error(&self, err: impl Into<String>) {
        let mut status = self.status.write().await;
        status.last_error = Some(err.into());
        status.healthy = false;
    }

    /// Mark the tick healthy (no error this round).
    pub async fn record_healthy(&self) {
        let mut status = self.status.write().await;
        status.last_error = None;
        status.healthy = true;
    }

    /// Build a snapshot for the HTTP handler. Cheap — clones a few
    /// atomics and reads the rate sample under a short read lock.
    pub async fn snapshot(&self, now_ms: i64) -> HealthSnapshot {
        let sample = self.rate_sample.read().await;
        let status = self.status.read().await;
        HealthSnapshot {
            last_flush_ts: self.last_flush_ts.load(Ordering::Relaxed),
            files_watched: self.files_watched.load(Ordering::Relaxed),
            envelope_rate: sample.rate_per_sec,
            rss_bytes: self.rss_bytes.load(Ordering::Relaxed),
            envelopes_emitted: self.envelopes_emitted.load(Ordering::Relaxed),
            envelopes_dropped: self.envelopes_dropped.load(Ordering::Relaxed),
            uptime_ms: (now_ms - self.started_at_ms).max(0) as u64,
            status: if status.healthy {
                "healthy".to_string()
            } else {
                status
                    .last_error
                    .clone()
                    .map(|e| format!("degraded: {e}"))
                    .unwrap_or_else(|| "degraded".to_string())
            },
        }
    }
}

/// Handler for `GET /healthz`. Wired in [`build_health_router`].
async fn healthz_handler(State(state): State<Arc<TailState>>) -> Json<HealthSnapshot> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    Json(state.snapshot(now_ms).await)
}

/// Build the axum router exposing `/healthz`. Pure function so tests
/// can drive it via tower's in-memory transport.
pub fn build_health_router(state: Arc<TailState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz_handler))
        .with_state(state)
}

/// Per-file polling cursor — tracks the last `(mtime, len)` we saw
/// so the next tick skips quiescent files entirely.
#[derive(Debug, Default)]
struct FileCursor {
    last_mtime_ms: i64,
    last_len: u64,
}

/// Tail watcher — polls the corpus, emits new envelopes, exposes a
/// health endpoint. Owns the emitter + cursor + dedupe set; the
/// runtime stitches it together via [`run_watcher_loop`].
pub struct TailWatcher {
    cfg: WalkConfig,
    emitter: Box<dyn Emitter>,
    cursors: BTreeMap<PathBuf, FileCursor>,
    emitted_ids: BTreeSet<String>,
    state: Arc<TailState>,
    sysinfo: sysinfo::System,
    sysinfo_pid: sysinfo::Pid,
}

impl TailWatcher {
    /// Build a fresh watcher around a [`WalkConfig`] + [`Emitter`].
    pub fn new(cfg: WalkConfig, emitter: Box<dyn Emitter>, state: Arc<TailState>) -> Self {
        let mut sysinfo = sysinfo::System::new();
        sysinfo.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new().with_memory(),
        );
        let sysinfo_pid = sysinfo::Pid::from_u32(std::process::id());
        Self {
            cfg,
            emitter,
            cursors: BTreeMap::new(),
            emitted_ids: BTreeSet::new(),
            state,
            sysinfo,
            sysinfo_pid,
        }
    }

    /// Sample current process RSS in bytes via `sysinfo`. Returns
    /// `0` when the platform sampler has no signal (rare; usually
    /// only on locked-down CI containers).
    fn sample_rss(&mut self) -> u64 {
        self.sysinfo.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[self.sysinfo_pid]),
            true,
            sysinfo::ProcessRefreshKind::new().with_memory(),
        );
        self.sysinfo
            .process(self.sysinfo_pid)
            .map(|p| p.memory())
            .unwrap_or(0)
    }

    /// One pass over the corpus. Returns `(emitted_delta, dropped_delta,
    /// files_seen)` so the caller can update the shared state.
    pub fn tick_once(&mut self) -> Result<TickReport> {
        let entries = walk(&self.cfg);
        let mut report = TickReport::default();
        report.files_seen = entries.len();

        for entry in entries {
            // Sidecars round-trip through the §1.5 follow-up
            // projector; the v1 watcher only re-reads sessions.
            if !matches!(entry.kind, WalkKind::Session | WalkKind::CodexSession) {
                continue;
            }
            let stat = match std::fs::metadata(&entry.path) {
                Ok(m) => m,
                Err(err) => {
                    tracing::debug!(path = %entry.path.display(), %err, "stat failed; skipping");
                    continue;
                }
            };
            let mtime_ms = file_mtime_ms(&stat);
            let len = stat.len();
            let cursor = self.cursors.entry(entry.path.clone()).or_default();
            if cursor.last_mtime_ms == mtime_ms && cursor.last_len == len {
                continue;
            }
            cursor.last_mtime_ms = mtime_ms;
            cursor.last_len = len;

            // Re-read the whole file. Cheap because changed sessions
            // are rare and the dedupe set short-circuits emit.
            let (records, read_stats) = match read_records(&entry.path) {
                Ok(r) => r,
                Err(err) => {
                    report
                        .errors
                        .push(format!("{}: {err}", entry.path.display()));
                    continue;
                }
            };
            // Phase11i §5.4 — malformed JSONL lines are *recoverable*
            // (the reader warns + drops them), but they still belong
            // in the watcher's drop counter so the dashboard can spot
            // a corruption regression. Typeless / unknown-kind
            // records are tracked alongside as a separate counter.
            if read_stats.malformed_lines > 0 || read_stats.typeless_records > 0 {
                let bad = read_stats.malformed_lines as u64 + read_stats.typeless_records as u64;
                report.dropped += bad;
                tracing::warn!(
                    path = %entry.path.display(),
                    malformed = read_stats.malformed_lines,
                    typeless = read_stats.typeless_records,
                    "tail: dropped malformed records",
                );
            }
            let (envelopes, _map_stats) = map_session(&records, None)?;
            for env in &envelopes {
                if !self.emitted_ids.insert(env.event_id.clone()) {
                    continue;
                }
                match self.emitter.emit(env) {
                    Ok(()) => report.emitted += 1,
                    Err(err) => {
                        tracing::warn!(
                            path = %entry.path.display(),
                            %err,
                            "emit failed; counted as dropped",
                        );
                        report.dropped += 1;
                    }
                }
            }
        }
        if let Err(err) = self.emitter.flush() {
            report.errors.push(format!("flush: {err}"));
        }
        Ok(report)
    }
}

/// One tick's deltas — accumulated by the polling loop into the
/// shared [`TailState`].
#[derive(Debug, Default, Clone)]
pub struct TickReport {
    /// Envelopes successfully emitted this tick.
    pub emitted: u64,
    /// Envelopes the emitter rejected this tick.
    pub dropped: u64,
    /// Total `.jsonl` files the walker surfaced this tick.
    pub files_seen: usize,
    /// Per-file errors (read / parse failures). Drives the
    /// `degraded` health status.
    pub errors: Vec<String>,
}

/// Run the polling loop until `shutdown` resolves. Each tick:
/// 1. calls [`TailWatcher::tick_once`],
/// 2. samples process RSS via sysinfo,
/// 3. updates the shared [`TailState`] under the rate sampler.
pub async fn run_watcher_loop(
    mut watcher: TailWatcher,
    interval: Duration,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let state = watcher.state.clone();
    let mut emitted_total: u64 = 0;
    let mut dropped_total: u64 = 0;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("tail watcher shutting down");
                    break;
                }
            }
            _ = ticker.tick() => {
                let started = Instant::now();
                match watcher.tick_once() {
                    Ok(report) => {
                        emitted_total += report.emitted;
                        dropped_total += report.dropped;
                        let rss = watcher.sample_rss();
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        let rate = state
                            .record_tick(now_ms, emitted_total, dropped_total, report.files_seen, rss)
                            .await;
                        if report.errors.is_empty() {
                            state.record_healthy().await;
                        } else {
                            state.record_error(report.errors.join("; ")).await;
                        }
                        tracing::debug!(
                            files = report.files_seen,
                            emitted = report.emitted,
                            dropped = report.dropped,
                            rss_bytes = rss,
                            rate_per_sec = rate,
                            tick_ms = started.elapsed().as_millis() as u64,
                            "tail tick",
                        );
                    }
                    Err(err) => {
                        state.record_error(err.to_string()).await;
                        tracing::warn!(%err, "tail tick failed");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Spawn the axum server bound to `addr`. Returns the join handle so
/// the caller can `await` it during shutdown.
pub async fn serve_health(
    state: Arc<TailState>,
    addr: SocketAddr,
) -> Result<tokio::task::JoinHandle<Result<()>>> {
    let router = build_health_router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!(%addr, "tail health endpoint listening");
    let handle =
        tokio::spawn(async move { axum::serve(listener, router).await.context("axum::serve") });
    Ok(handle)
}

fn file_mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Resolve the bind address from `CORTEX_CLAUDE_ARCHIVE_BIND` env or
/// fall back to [`DEFAULT_HEALTH_BIND`]. Surfaces a parse error so a
/// typo aborts the daemon at boot rather than silently falling back.
pub fn resolve_health_bind() -> Result<SocketAddr> {
    let raw = std::env::var("CORTEX_CLAUDE_ARCHIVE_BIND")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_HEALTH_BIND.to_string());
    raw.parse::<SocketAddr>()
        .with_context(|| format!("parse CORTEX_CLAUDE_ARCHIVE_BIND={raw:?}"))
}

/// Default poll interval, overridable via
/// `CORTEX_CLAUDE_ARCHIVE_POLL_MS`. Out-of-range values fall back
/// to [`DEFAULT_POLL_INTERVAL`] with a WARN.
pub fn resolve_poll_interval() -> Duration {
    let raw = match std::env::var("CORTEX_CLAUDE_ARCHIVE_POLL_MS") {
        Ok(v) => v,
        Err(_) => return DEFAULT_POLL_INTERVAL,
    };
    match raw.parse::<u64>() {
        Ok(v) if (100..=60_000).contains(&v) => Duration::from_millis(v),
        Ok(v) => {
            tracing::warn!(
                raw = %raw,
                parsed = v,
                "CORTEX_CLAUDE_ARCHIVE_POLL_MS out of [100, 60_000]; using default"
            );
            DEFAULT_POLL_INTERVAL
        }
        Err(err) => {
            tracing::warn!(raw = %raw, %err, "CORTEX_CLAUDE_ARCHIVE_POLL_MS not a u64; using default");
            DEFAULT_POLL_INTERVAL
        }
    }
}

/// Resolve `WalkConfig.root` against `CORTEX_CLAUDE_ARCHIVE_ROOT`
/// when the CLI did not pass `--root`.
#[allow(dead_code)]
fn resolve_root_env() -> Option<PathBuf> {
    std::env::var("CORTEX_CLAUDE_ARCHIVE_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

/// One-shot helper used by the binary's `run_tail` entry point.
/// Wires up the watcher + axum server + signal handling. The
/// function is `pub` so an IT can call it with a synthetic shutdown
/// channel and a temp root.
pub async fn run_tail_daemon(
    cfg: WalkConfig,
    emitter: Box<dyn Emitter>,
    bind: SocketAddr,
    interval: Duration,
) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let state = Arc::new(TailState::new(now_ms));
    let watcher = TailWatcher::new(cfg, emitter, state.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // SIGINT / SIGTERM trigger a graceful shutdown.
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(err) = wait_for_shutdown_signal().await {
            tracing::warn!(%err, "shutdown signal listener failed");
        }
        let _ = shutdown_signal.send(true);
    });

    let server = serve_health(state.clone(), bind).await?;
    run_watcher_loop(watcher, interval, shutdown_rx).await?;

    // Drop server when the watcher loop exits. The graceful axum
    // path lives behind `tokio::signal`; we abort here so SIGTERM
    // unblocks the daemon promptly.
    server.abort();
    Ok(())
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    tokio::select! {
        _ = sigterm.recv() => Ok(()),
        _ = sigint.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .context("install Ctrl+C handler")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn record_tick_computes_per_second_rate_against_previous_sample() {
        let state = Arc::new(TailState::new(0));
        // First tick at t=1000ms with 0 emitted — rate=0 because the
        // baseline was unset.
        let r1 = state.record_tick(1_000, 0, 0, 0, 0).await;
        assert_eq!(r1, 0.0);
        // Second tick at t=2000ms with 12 emitted — rate=12/sec.
        let r2 = state.record_tick(2_000, 12, 0, 1, 0).await;
        assert!((r2 - 12.0).abs() < 1e-9, "rate={r2}");
        // Third tick at t=3000ms with 12 emitted (no new envelopes)
        // — rate=0/sec because the delta is zero.
        let r3 = state.record_tick(3_000, 12, 0, 1, 0).await;
        assert_eq!(r3, 0.0);
    }

    #[tokio::test]
    async fn snapshot_reports_uptime_and_status() {
        let state = Arc::new(TailState::new(1_000));
        state.record_tick(2_000, 5, 1, 3, 4_096).await;
        let snap = state.snapshot(2_500).await;
        assert_eq!(snap.last_flush_ts, 2_000);
        assert_eq!(snap.files_watched, 3);
        assert_eq!(snap.envelopes_emitted, 5);
        assert_eq!(snap.envelopes_dropped, 1);
        assert_eq!(snap.rss_bytes, 4_096);
        assert_eq!(snap.uptime_ms, 1_500);
        assert_eq!(snap.status, "healthy");
    }

    #[tokio::test]
    async fn snapshot_marks_status_degraded_after_error() {
        let state = Arc::new(TailState::new(0));
        state.record_error("disk full").await;
        let snap = state.snapshot(1_000).await;
        assert!(
            snap.status.starts_with("degraded"),
            "status={}",
            snap.status
        );
        assert!(snap.status.contains("disk full"), "status={}", snap.status);
    }

    #[tokio::test]
    async fn record_healthy_clears_prior_error() {
        let state = Arc::new(TailState::new(0));
        state.record_error("transient").await;
        state.record_healthy().await;
        let snap = state.snapshot(0).await;
        assert_eq!(snap.status, "healthy");
    }

    #[tokio::test]
    async fn healthz_endpoint_returns_snapshot_json() {
        let state = Arc::new(TailState::new(0));
        state.record_tick(1_000, 7, 2, 4, 8_192).await;
        let app = build_health_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["files_watched"], 4);
        assert_eq!(v["envelopes_emitted"], 7);
        assert_eq!(v["envelopes_dropped"], 2);
        assert_eq!(v["rss_bytes"], 8_192);
        assert_eq!(v["status"], "healthy");
    }

    #[test]
    fn resolve_health_bind_falls_back_to_default_when_unset() {
        let prior = std::env::var("CORTEX_CLAUDE_ARCHIVE_BIND").ok();
        std::env::remove_var("CORTEX_CLAUDE_ARCHIVE_BIND");
        let addr = resolve_health_bind().unwrap();
        assert_eq!(addr.port(), 17030);
        if let Some(v) = prior {
            std::env::set_var("CORTEX_CLAUDE_ARCHIVE_BIND", v);
        }
    }

    #[test]
    fn resolve_health_bind_honours_env_override() {
        std::env::set_var("CORTEX_CLAUDE_ARCHIVE_BIND", "127.0.0.1:0");
        let addr = resolve_health_bind().unwrap();
        assert_eq!(addr.port(), 0);
        std::env::remove_var("CORTEX_CLAUDE_ARCHIVE_BIND");
    }

    #[test]
    fn resolve_health_bind_surfaces_parse_error() {
        std::env::set_var("CORTEX_CLAUDE_ARCHIVE_BIND", "not-an-address");
        let err = resolve_health_bind().unwrap_err();
        assert!(format!("{err:#}").contains("parse"), "{err:#}");
        std::env::remove_var("CORTEX_CLAUDE_ARCHIVE_BIND");
    }

    #[test]
    fn resolve_poll_interval_clamps_out_of_range_values() {
        std::env::set_var("CORTEX_CLAUDE_ARCHIVE_POLL_MS", "5");
        assert_eq!(resolve_poll_interval(), DEFAULT_POLL_INTERVAL);
        std::env::set_var("CORTEX_CLAUDE_ARCHIVE_POLL_MS", "999999");
        assert_eq!(resolve_poll_interval(), DEFAULT_POLL_INTERVAL);
        std::env::set_var("CORTEX_CLAUDE_ARCHIVE_POLL_MS", "1500");
        assert_eq!(resolve_poll_interval(), Duration::from_millis(1_500));
        std::env::remove_var("CORTEX_CLAUDE_ARCHIVE_POLL_MS");
    }
}

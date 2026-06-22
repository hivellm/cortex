//! Phase8f — synthetic end-to-end canary.
//!
//! phase8a–8e watch real traffic. During quiet hours (no claude-code
//! activity) the pipeline could be silently broken for hours and no
//! divergence would fire because no events flow. The canary closes
//! that gap: every N seconds, fire a known fake hook frame through
//! the real IPC path and assert it lands in the archive within a
//! deadline. If it doesn't, alert via the same path phase8e uses.
//!
//! The frame mimics the 2026-04-28 regression vector verbatim:
//! pretty-printed JSON with newlines between top-level fields and
//! multi-line escaped strings inside `tool_response.stdout`. A
//! canary that mimics that behavior would have caught the regression
//! the moment it landed.
//!
//! This module is library-only — the CLI driver lives in
//! `cortex-cli/src/bin/cortex-ops.rs::canary` and the cortex-api
//! background runner lives in [`run_canary_loop`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable, schema-valid ULID the canary failure-alert envelopes share
/// as their `session_id` so every canary `law_violation` groups under
/// one synthetic session. All chars are in the Crockford alphabet
/// `[0-9A-HJ-KM-NP-TV-Z]` and the length is exactly 26.
const CANARY_SESSION_ID: &str = "01CANARY000000000000000000";

/// Map `std::env::consts::OS` to the envelope schema's `context.platform`
/// enum (`win32` / `darwin` / `linux`, Node-style). The Rust names
/// (`windows` / `macos`) are not valid schema values; anything else
/// falls back to `linux` (the production container target).
fn platform_tag() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        _ => "linux",
    }
}

/// Hex-encode a SHA-256 digest as the lowercase 64-char string the
/// `content_hash` schema (`^sha256:[0-9a-f]{64}$`) requires.
fn sha256_hex(bytes: &[u8]) -> String {
    let out = Sha256::new().chain_update(bytes).finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Canary execution result, suitable for both CLI exit-code mapping
/// and JSONL history recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum CanaryOutcome {
    /// The synthetic envelope was observed in the archive within the
    /// deadline.
    Success {
        /// Wall-clock latency from frame send to archive observation.
        latency_ms: u64,
    },
    /// The deadline elapsed without observing the envelope.
    Timeout {
        /// Marker id that never appeared.
        marker_id: String,
        /// Configured deadline in milliseconds.
        deadline_ms: u64,
    },
    /// Pipe / archive transport error.
    Transport {
        /// Stage that failed (`pipe-connect`, `frame-write`,
        /// `archive-poll`).
        stage: String,
        /// One-line reason.
        reason: String,
    },
}

impl CanaryOutcome {
    /// CLI exit code mapping — `0` success, `2` timeout, `1`
    /// transport error.
    pub fn exit_code(&self) -> i32 {
        match self {
            CanaryOutcome::Success { .. } => 0,
            CanaryOutcome::Timeout { .. } => 2,
            CanaryOutcome::Transport { .. } => 1,
        }
    }
    /// `true` for any non-success outcome.
    pub fn is_failure(&self) -> bool {
        !matches!(self, CanaryOutcome::Success { .. })
    }
    /// Free-text explanation suitable for an alert envelope's
    /// `payload.message`.
    pub fn describe(&self) -> String {
        match self {
            CanaryOutcome::Success { latency_ms } => {
                format!("canary ok in {latency_ms}ms")
            }
            CanaryOutcome::Timeout {
                marker_id,
                deadline_ms,
            } => format!("canary marker {marker_id} not seen in archive within {deadline_ms}ms"),
            CanaryOutcome::Transport { stage, reason } => {
                format!("canary transport failure at {stage}: {reason}")
            }
        }
    }
}

/// Configuration for one canary tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryConfig {
    /// Master switch — when `false`, the runner is never spawned.
    pub enabled: bool,
    /// How often the background runner ticks. Default 300 s.
    pub interval_secs: u64,
    /// Deadline before a canary attempt is declared a `Timeout`.
    /// Default 10 s.
    pub deadline_secs: u64,
    /// Hook flavours to fire. Default `["PostToolUse"]`.
    pub hooks: Vec<String>,
    /// Append history file. Default `~/.cortex/canary-history.jsonl`.
    pub history_path: PathBuf,
    /// IPC binding override — defaults to platform default
    /// (named pipe on Windows, unix socket on Unix).
    pub ipc_path: Option<String>,
    /// `cortex-api` URL for the archive poll. Default
    /// `http://127.0.0.1:17000`.
    pub api_url: String,
    /// `cortex-ingestion` URL for the alert-emit POST. Default
    /// `http://127.0.0.1:17010`.
    pub ingestion_url: String,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Self {
            enabled: false,
            interval_secs: 300,
            deadline_secs: 10,
            hooks: vec!["PostToolUse".to_string()],
            history_path: PathBuf::from(home)
                .join(".cortex")
                .join("canary-history.jsonl"),
            ipc_path: None,
            api_url: cortex_config::Config::load()
                .ok()
                .and_then(|c| c.dashboard.api_url)
                .unwrap_or_else(|| "http://127.0.0.1:17000".to_string()),
            ingestion_url: cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.ingestion_url)
                .unwrap_or_else(|| "http://127.0.0.1:17010".to_string()),
        }
    }
}

/// Build a synthetic canary frame body for the given hook. The
/// `marker` is embedded in the tool name (PostToolUse) or the
/// prompt (UserPromptSubmit) so the archive poll can identify it.
///
/// The frame is deliberately *pretty-printed* with newlines between
/// fields and a multi-line `\n`-escaped string inside the payload —
/// the exact regression vector that bit on 2026-04-28.
pub fn build_canary_frame(hook: &str, marker: &str) -> serde_json::Value {
    match hook {
        "PostToolUse" => serde_json::json!({
            "hook": "PostToolUse",
            "session_id": format!("01CANARY{}", marker.chars().take(18).collect::<String>()),
            "cwd": "/canary",
            "payload": {
                "tool_name": format!("Canary-{marker}"),
                "tool_input": {
                    "command": "echo canary",
                },
                "tool_response": {
                    "stdout": format!("canary-{marker}\nline two\nline three"),
                    "stderr": "",
                    "exit_code": 0,
                },
            }
        }),
        "UserPromptSubmit" => serde_json::json!({
            "hook": "UserPromptSubmit",
            "session_id": format!("01CANARY{}", marker.chars().take(18).collect::<String>()),
            "cwd": "/canary",
            "payload": {
                "prompt": format!("canary-{marker}\nsecond line"),
            }
        }),
        "Stop" => serde_json::json!({
            "hook": "Stop",
            "session_id": format!("01CANARY{}", marker.chars().take(18).collect::<String>()),
            "cwd": "/canary",
            "payload": {
                "transcript_path": format!("/canary/transcript-{marker}.jsonl"),
            }
        }),
        other => serde_json::json!({
            "hook": other,
            "session_id": format!("01CANARY{}", marker.chars().take(18).collect::<String>()),
            "cwd": "/canary",
            "payload": {
                "marker": marker,
            }
        }),
    }
}

/// Generate a unique canary marker. ULID-ish (timestamp + random
/// suffix) — embedded in the tool name so the archive poll can
/// identify it among thousands of real envelopes.
pub fn new_marker() -> String {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let nonce: u32 = (now as u32).wrapping_mul(2_654_435_761);
    format!("{:014x}{:08x}", now, nonce)
}

/// Append one history entry to the JSONL file. Best-effort: I/O
/// errors log + continue (a missing history line is preferable to
/// killing the runner over a disk hiccup).
pub fn append_history(history_path: &Path, entry: &CanaryHistoryEntry) {
    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = match serde_json::to_string(entry) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "canary: history serialize failed");
            return;
        }
    };
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_path)
    {
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        let _ = f.write_all(&bytes);
    }
}

/// One row in the JSONL history file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryHistoryEntry {
    /// RFC-3339 timestamp the canary tick started.
    pub ts: String,
    /// Hook flavour fired this tick.
    pub hook: String,
    /// Marker embedded in the synthetic frame.
    pub marker: String,
    /// Outcome.
    #[serde(flatten)]
    pub outcome: CanaryOutcome,
}

/// Send a canary frame through the daemon's IPC and poll the archive
/// for the marker. Returns the outcome of the round-trip.
///
/// Pure async function — no global state, suitable for both the CLI
/// driver and the background runner.
pub async fn run_canary_once(cfg: &CanaryConfig, hook: &str) -> CanaryOutcome {
    let marker = new_marker();
    let frame = build_canary_frame(hook, &marker);
    let bytes = match serde_json::to_vec(&frame) {
        Ok(b) => b,
        Err(e) => {
            return CanaryOutcome::Transport {
                stage: "frame-build".into(),
                reason: e.to_string(),
            };
        }
    };

    let started = std::time::Instant::now();
    if let Err(e) = send_frame_via_ipc(cfg.ipc_path.as_deref(), &bytes).await {
        return CanaryOutcome::Transport {
            stage: "pipe-connect".into(),
            reason: e,
        };
    }

    // Poll the dashboard timeline for the marker. The PostToolUse
    // path lands the envelope under `kind=tool_call` with the canary
    // tool name visible in the row text — same surface dashboards
    // render. Other hooks land under `turn`.
    let deadline = Duration::from_millis(cfg.deadline_secs.saturating_mul(1_000));
    let target_kind = if hook == "PostToolUse" {
        "tool_call"
    } else {
        "turn"
    };
    match poll_archive_for_marker(&cfg.api_url, target_kind, &marker, deadline).await {
        Ok(()) => {
            let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(0);
            CanaryOutcome::Success { latency_ms }
        }
        Err(PollError::Timeout) => CanaryOutcome::Timeout {
            marker_id: marker.clone(),
            deadline_ms: cfg.deadline_secs.saturating_mul(1_000),
        },
        Err(PollError::Transport(reason)) => CanaryOutcome::Transport {
            stage: "archive-poll".into(),
            reason,
        },
    }
}

#[derive(Debug)]
enum PollError {
    Timeout,
    Transport(String),
}

/// Poll `GET /v1/dashboard/timeline/recent?kind=<kind>` every 250 ms
/// until the marker is found in any row's text or the deadline elapses.
async fn poll_archive_for_marker(
    api_url: &str,
    kind: &str,
    marker: &str,
    deadline: Duration,
) -> Result<(), PollError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| PollError::Transport(format!("client: {e}")))?;
    let url = format!(
        "{}/v1/dashboard/timeline/recent?kind={kind}",
        api_url.trim_end_matches('/')
    );
    let started = std::time::Instant::now();
    while started.elapsed() < deadline {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let text = resp.text().await.unwrap_or_default();
                if text.contains(marker) {
                    return Ok(());
                }
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "canary: timeline non-2xx");
            }
            Err(e) => {
                tracing::debug!(error = %e, "canary: timeline poll transport error");
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(PollError::Timeout)
}

/// Send raw bytes to the daemon's IPC endpoint and read at most one
/// response (the daemon replies with a single `HookResponse` JSON
/// object then closes). Implementation is platform-conditional.
async fn send_frame_via_ipc(ipc_path: Option<&str>, bytes: &[u8]) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let pipe = ipc_path
            .unwrap_or(r"\\.\pipe\cortex-adapter-claude")
            .to_string();
        let mut client = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe)
        })
        .await
        .map_err(|_| "named-pipe connect timeout".to_string())?
        .map_err(|e| format!("named-pipe open: {e}"))?;
        client
            .write_all(bytes)
            .await
            .map_err(|e| format!("write: {e}"))?;
        client.shutdown().await.ok();
        let mut buf = Vec::with_capacity(1024);
        let _ = tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut buf)).await;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let path = match ipc_path {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                std::path::PathBuf::from(home)
                    .join(".cortex")
                    .join("adapter-claude.sock")
            }
        };
        let mut stream = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::UnixStream::connect(&path),
        )
        .await
        .map_err(|_| "unix-socket connect timeout".to_string())?
        .map_err(|e| format!("unix-socket open: {e}"))?;
        stream
            .write_all(bytes)
            .await
            .map_err(|e| format!("write: {e}"))?;
        stream.shutdown().await.ok();
        let mut buf = Vec::with_capacity(1024);
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
        Ok(())
    }
}

/// Background runner — invoked from cortex-api's main when the
/// canary feature is enabled in config.
pub async fn run_canary_loop(cfg: CanaryConfig) {
    let interval = Duration::from_secs(cfg.interval_secs.max(10));
    tracing::info!(
        interval_secs = interval.as_secs(),
        deadline_secs = cfg.deadline_secs,
        hooks = ?cfg.hooks,
        history_path = %cfg.history_path.display(),
        "canary runner started"
    );
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // skip immediate first tick
    loop {
        ticker.tick().await;
        for hook in &cfg.hooks {
            let started_at = chrono::Utc::now().to_rfc3339();
            let marker = new_marker();
            let mut local_cfg = cfg.clone();
            local_cfg.hooks = vec![hook.clone()]; // no-op but explicit
            let outcome = run_canary_once(&local_cfg, hook).await;
            let entry = CanaryHistoryEntry {
                ts: started_at,
                hook: hook.clone(),
                marker: marker.clone(),
                outcome: outcome.clone(),
            };
            append_history(&cfg.history_path, &entry);
            if outcome.is_failure() {
                tracing::warn!(
                    hook = %hook,
                    outcome = ?outcome,
                    "canary tick failed"
                );
                emit_failure_alert(&cfg, hook, &outcome).await;
            } else {
                tracing::debug!(hook = %hook, outcome = ?outcome, "canary tick ok");
            }
        }
    }
}

/// Build a SCHEMA-VALID `law_violation` envelope describing a canary
/// failure. The previous inline shape hand-rolled invalid ULIDs
/// (`01CANARYFAIL…`), an out-of-enum `tool` (`cortex-api`), an invalid
/// `content_hash`, and a payload carrying a `hook` property the schema
/// forbids — so cortex-ingestion rejected every alert
/// (`reason_class=invalid_json`), the failure never reached the
/// Violations dashboard it targets, and the ingestion log filled with
/// rejection WARNs every tick. This valid form lands the alert and
/// keeps the log quiet.
fn build_failure_alert_envelope(
    hook: &str,
    outcome: &CanaryOutcome,
    now: &str,
) -> serde_json::Value {
    let message = format!("canary tick failed for hook '{hook}': {}", outcome.describe());
    let content_hash = format!("sha256:{}", sha256_hex(message.as_bytes()));
    serde_json::json!({
        "event_id": ulid::Ulid::new().to_string(),
        "schema_version": "1",
        "occurred_at": now,
        "session_id": CANARY_SESSION_ID,
        "stream": "live",
        "tool": "cortex-cli",
        "kind": "law_violation",
        "context": { "platform": platform_tag() },
        "payload": {
            "violation_id": ulid::Ulid::new().to_string(),
            "law_id": "CANARY-1",
            "severity": "critical",
            "message": message,
            // `hook` is not an allowed top-level payload property;
            // `evidence` accepts free-form keys, so carry it there.
            "evidence": { "hook": hook, "outcome": outcome.describe() },
        },
        "redactions": [],
        "content_hash": content_hash,
    })
}

/// POST a `law_violation` envelope describing the canary failure to
/// cortex-ingestion. Same alert path phase8e uses, so the failure
/// shows up in the existing Violations dashboard automatically.
async fn emit_failure_alert(cfg: &CanaryConfig, hook: &str, outcome: &CanaryOutcome) {
    let now = chrono::Utc::now().to_rfc3339();
    let envelope = build_failure_alert_envelope(hook, outcome, &now);
    let body = serde_json::json!({ "events": [envelope] });
    let url = format!(
        "{}/v1/events/batch",
        cfg.ingestion_url.trim_end_matches('/')
    );
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "canary: alert client build failed");
            return;
        }
    };
    if let Err(e) = client.post(&url).json(&body).send().await {
        tracing::warn!(error = %e, url = %url, "canary: alert POST failed");
    }
}

/// Phase15f — background pre-thinking health canary.
///
/// Ticks every `interval_secs` seconds, calls `GET /v1/health/pre-thinking`
/// on the running cortex-api, records the result in the `canary_runs` SQLite
/// table, and emits a structured `tracing::warn!` when two consecutive ticks
/// fail. The consecutive counter resets on the first successful tick.
/// Rows older than 24 h are deleted each tick.
pub async fn run_pre_thinking_health_canary_loop(
    interval_secs: u64,
    api_url: String,
    metadata_path: std::path::PathBuf,
) {
    let interval = Duration::from_secs(interval_secs.max(5));
    tracing::info!(
        interval_secs = interval.as_secs(),
        api_url = %api_url,
        "pre-thinking health canary started"
    );

    let store = match cortex_storage::metadata::MetadataStore::open(&metadata_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "pre-thinking canary: failed to open metadata db");
            return;
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "pre-thinking canary: failed to build http client");
            return;
        }
    };

    let url = format!("{}/v1/health/pre-thinking", api_url.trim_end_matches('/'));

    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // skip immediate first tick
    let mut consecutive_failures: u64 = 0;

    loop {
        ticker.tick().await;

        let tick_start = std::time::Instant::now();
        let now = chrono::Utc::now().to_rfc3339();

        let (status, latency_ms, error_message) = match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let latency = u64::try_from(tick_start.elapsed().as_millis()).unwrap_or(0);
                ("ok".to_string(), Some(latency as i64), None)
            }
            Ok(resp) => {
                let latency = u64::try_from(tick_start.elapsed().as_millis()).unwrap_or(0);
                let msg = format!("http {}", resp.status().as_u16());
                ("error".to_string(), Some(latency as i64), Some(msg))
            }
            Err(e) => ("error".to_string(), None, Some(e.to_string())),
        };

        if let Err(e) = store.insert_canary_run(&now, &status, latency_ms, error_message.as_deref())
        {
            tracing::warn!(error = %e, "pre-thinking canary: failed to insert run row");
        }

        if status == "ok" {
            consecutive_failures = 0;
        } else {
            consecutive_failures += 1;
            if consecutive_failures >= 2 {
                tracing::warn!(
                    target: "canary",
                    consecutive_failures,
                    last_error = error_message.as_deref().unwrap_or("unknown"),
                    "canary alarm"
                );
            }
        }

        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        if let Err(e) = store.cleanup_canary_runs(&cutoff) {
            tracing::debug!(error = %e, "pre-thinking canary: cleanup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_alert_envelope_is_schema_valid() {
        // Regression: the old hand-rolled shape was rejected by
        // cortex-ingestion (invalid ULIDs, out-of-enum tool, bad
        // content_hash, forbidden `hook` payload prop) and spammed the
        // ingestion log every tick. The valid form must pass the
        // canonical validator.
        let outcome = CanaryOutcome::Transport {
            stage: "post".to_string(),
            reason: "connection refused".to_string(),
        };
        let env = build_failure_alert_envelope("PostToolUse", &outcome, "2026-06-21T00:00:00Z");
        cortex_core::validate::validate_event(&env)
            .unwrap_or_else(|e| panic!("canary failure-alert envelope must validate: {e:?}"));
        // The hook rides in `evidence` (free-form), not as a forbidden
        // top-level payload property.
        let payload = env.get("payload").unwrap();
        assert!(payload.get("hook").is_none());
        assert_eq!(
            payload
                .get("evidence")
                .and_then(|e| e.get("hook"))
                .and_then(|v| v.as_str()),
            Some("PostToolUse")
        );
        // content_hash matches the schema's sha256 shape.
        let ch = env.get("content_hash").and_then(|v| v.as_str()).unwrap();
        assert!(ch.starts_with("sha256:") && ch.len() == "sha256:".len() + 64);
    }

    #[test]
    fn build_canary_frame_post_tool_use_carries_marker_in_tool_name() {
        let f = build_canary_frame("PostToolUse", "abc123");
        assert_eq!(f.get("hook").and_then(|v| v.as_str()), Some("PostToolUse"));
        let payload = f.get("payload").unwrap();
        assert_eq!(
            payload.get("tool_name").and_then(|v| v.as_str()),
            Some("Canary-abc123")
        );
        // The stdout is multi-line — the regression vector from
        // 2026-04-28. The serializer emits `\n` (escaped) which is
        // exactly what claude-code does.
        let stdout = payload
            .get("tool_response")
            .and_then(|v| v.get("stdout"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(stdout.contains('\n'));
        assert!(stdout.contains("abc123"));
    }

    #[test]
    fn build_canary_frame_user_prompt_submit_contains_marker_in_prompt() {
        let f = build_canary_frame("UserPromptSubmit", "xyz999");
        let prompt = f
            .get("payload")
            .and_then(|v| v.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(prompt.contains("xyz999"));
    }

    #[test]
    fn build_canary_frame_unknown_hook_falls_back_to_marker_payload() {
        let f = build_canary_frame("WeirdHook", "m1");
        assert_eq!(f.get("hook").and_then(|v| v.as_str()), Some("WeirdHook"));
        let marker = f
            .get("payload")
            .and_then(|v| v.get("marker"))
            .and_then(|v| v.as_str());
        assert_eq!(marker, Some("m1"));
    }

    #[test]
    fn new_marker_is_unique_per_call_and_stable_length() {
        let a = new_marker();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_marker();
        assert_ne!(a, b);
        assert_eq!(a.len(), b.len());
        assert_eq!(a.len(), 22);
    }

    #[test]
    fn outcome_exit_code_mapping_matches_spec() {
        assert_eq!(CanaryOutcome::Success { latency_ms: 1 }.exit_code(), 0);
        assert_eq!(
            CanaryOutcome::Timeout {
                marker_id: "m".into(),
                deadline_ms: 10_000,
            }
            .exit_code(),
            2
        );
        assert_eq!(
            CanaryOutcome::Transport {
                stage: "x".into(),
                reason: "y".into(),
            }
            .exit_code(),
            1
        );
    }

    #[test]
    fn outcome_describe_includes_relevant_fields() {
        assert!(CanaryOutcome::Success { latency_ms: 42 }
            .describe()
            .contains("42ms"));
        let t = CanaryOutcome::Timeout {
            marker_id: "abc".into(),
            deadline_ms: 5_000,
        };
        let s = t.describe();
        assert!(s.contains("abc"));
        assert!(s.contains("5000ms"));
    }

    #[test]
    fn outcome_is_failure_for_non_success() {
        assert!(!CanaryOutcome::Success { latency_ms: 0 }.is_failure());
        assert!(CanaryOutcome::Timeout {
            marker_id: "x".into(),
            deadline_ms: 0,
        }
        .is_failure());
        assert!(CanaryOutcome::Transport {
            stage: "x".into(),
            reason: "y".into(),
        }
        .is_failure());
    }

    #[test]
    fn outcome_serde_round_trips_via_internal_tag() {
        let success = CanaryOutcome::Success { latency_ms: 7 };
        let s = serde_json::to_string(&success).unwrap();
        // The serde tag is `outcome` — verify shape AND round-trip.
        assert!(s.contains("\"outcome\":\"success\""));
        let parsed: CanaryOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, success);
    }

    #[test]
    fn append_history_creates_dir_and_appends_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("history.jsonl");
        let entry = CanaryHistoryEntry {
            ts: "2026-04-29T18:00:00Z".to_string(),
            hook: "PostToolUse".to_string(),
            marker: "m1".to_string(),
            outcome: CanaryOutcome::Success { latency_ms: 5 },
        };
        append_history(&path, &entry);
        // Append a second time — verify both rows present.
        let entry2 = CanaryHistoryEntry {
            ts: "2026-04-29T18:01:00Z".to_string(),
            hook: "PostToolUse".to_string(),
            marker: "m2".to_string(),
            outcome: CanaryOutcome::Timeout {
                marker_id: "m2".into(),
                deadline_ms: 10_000,
            },
        };
        append_history(&path, &entry2);
        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"marker\":\"m1\""));
        assert!(lines[1].contains("\"marker\":\"m2\""));
    }

    #[test]
    fn config_default_pulls_from_env_vars() {
        let cfg = CanaryConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.interval_secs, 300);
        assert_eq!(cfg.deadline_secs, 10);
        assert_eq!(cfg.hooks, vec!["PostToolUse".to_string()]);
    }

    #[test]
    fn history_entry_serializes_with_outcome_tag_inline() {
        let entry = CanaryHistoryEntry {
            ts: "now".to_string(),
            hook: "Stop".to_string(),
            marker: "z".to_string(),
            outcome: CanaryOutcome::Success { latency_ms: 1 },
        };
        let s = serde_json::to_string(&entry).unwrap();
        // The `#[serde(flatten)]` on `outcome` lifts its tagged
        // fields into the parent object — so the line contains
        // `"outcome":"success"` and `"latency_ms":1` at the same
        // depth as `ts`/`hook`/`marker`.
        assert!(s.contains("\"outcome\":\"success\""));
        assert!(s.contains("\"latency_ms\":1"));
        assert!(s.contains("\"hook\":\"Stop\""));
    }
}

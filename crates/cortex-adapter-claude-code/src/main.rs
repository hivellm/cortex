//! `cortex-adapter-claude` binary — daemon + install / uninstall /
//! status sub-commands. Spec 10 §Install / uninstall.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortex_adapter_claude_code::{
    install_with as install_layout_with, load_config, serve, spawn_flusher,
    uninstall as uninstall_layout, AdapterSection, DisabledPublisher, Dispatcher, HttpPublisher,
    InstallOptions, IpcBinding, Layout, Metrics, OverflowWal, Publisher, SessionManager,
    SyncClient,
};
use tracing_subscriber::{fmt, EnvFilter};

/// Phase14c — cadence the heartbeat task bumps `last_heartbeat_ts_ms`
/// at. Picked at 30 s so the cortex-api freshness aggregator's
/// 60 s WARN threshold has a 2× safety margin (a single missed tick
/// still keeps the gauge fresh).
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Parser)]
#[command(name = "cortex-adapter-claude", version, about)]
struct Cli {
    /// Sub-command — defaults to `daemon` when omitted.
    #[command(subcommand)]
    command: Option<Command>,
    /// Path to `~/.cortex/adapter.toml`.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Override the IPC binding (Unix socket path or
    /// `\\.\pipe\<name>` on Windows).
    #[arg(long, value_name = "BINDING")]
    bind: Option<String>,
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Run the daemon (default).
    Daemon,
    /// Install hook shims + patch `~/.claude/settings.json`.
    ///
    /// Pass `--no-hooks` when you already have the spec-18 plugin
    /// installed (`claude plugin install cortex@hivellm-cortex`) — the
    /// installer keeps the daemon socket but skips the hook-shim
    /// write + settings.json patch so the plugin is the only source
    /// of hook firing.
    Install {
        /// Skip the `~/.claude/hooks/` write + the settings.json
        /// patch. The daemon and adapter binary are still set up.
        #[arg(long)]
        no_hooks: bool,
    },
    /// Reverse install. Pass `--purge` to also remove the hook files.
    Uninstall {
        /// Remove the on-disk hook shims as well as the settings
        /// stanza.
        #[arg(long)]
        purge: bool,
    },
    /// Print current daemon status (best-effort, file-based).
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command.unwrap_or(Command::Daemon) {
        Command::Daemon => run_daemon(cli.config.as_deref(), cli.bind.as_deref()).await,
        Command::Install { no_hooks } => {
            let layout = default_layout();
            let report = install_layout_with(&layout, InstallOptions { no_hooks })?;
            if report.hooks_omitted {
                println!(
                    "hook shim install omitted (--no-hooks); spec-18 plugin owns the hook surface"
                );
            } else {
                println!("installed {} hook shims", report.hooks_written.len());
                if report.settings_modified {
                    println!(
                        "settings.json patched at {}",
                        layout.settings_path.display()
                    );
                }
            }
            Ok(())
        }
        Command::Uninstall { purge } => {
            let layout = default_layout();
            let report = uninstall_layout(&layout, purge)?;
            if report.settings_modified {
                println!(
                    "settings.json restored at {}",
                    layout.settings_path.display()
                );
            }
            if purge && !report.hooks_written.is_empty() {
                println!("removed {} hook shims", report.hooks_written.len());
            }
            Ok(())
        }
        Command::Status => {
            let layout = default_layout();
            println!("settings: {}", layout.settings_path.display());
            println!("hooks dir: {}", layout.hooks_dir.display());
            Ok(())
        }
    }
}

async fn run_daemon(
    config_path: Option<&std::path::Path>,
    bind_override: Option<&str>,
) -> Result<()> {
    let cfg_path = config_path.map(PathBuf::from).unwrap_or_else(|| {
        let home = home_dir();
        home.join(".cortex").join("adapter.toml")
    });
    let cfg = load_config(&cfg_path)
        .with_context(|| format!("load adapter config from {}", cfg_path.display()))?;
    let adapter = cfg.adapter;
    let typed_cfg =
        cortex_config::Config::load_client().context("load Cortex runtime configuration")?;

    let metrics = Arc::new(Metrics::new());
    let wal_path = home_dir().join(".cortex").join("overflow.wal");
    let daily = typed_cfg.is_daily();
    let publisher = runtime_publisher(daily, &adapter, &wal_path, metrics.clone()).await?;
    let bulk_capture = publisher.enabled();

    let mut sync_client = SyncClient::new(&adapter, metrics.clone());
    if let Some(token) = typed_cfg
        .dashboard
        .api_token
        .clone()
        .filter(|token| !token.is_empty())
    {
        sync_client = sync_client.with_api_key(token);
    }
    let sync = Arc::new(sync_client);
    // Grab the revision-checked memory cache metrics handle before `sync`
    // is moved into the Dispatcher, so the /healthz closure below can
    // surface the live cache hit/miss counters.
    let prethink_metrics_for_health = sync.prethink_metrics();
    let sessions = Arc::new(SessionManager::new());
    let pid = std::process::id();
    let dispatcher = Arc::new(
        Dispatcher::new(sessions, publisher.clone(), sync, pid).with_metrics(metrics.clone()),
    );

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let _flusher = bulk_capture.then(|| {
        spawn_flusher(
            publisher.clone(),
            Duration::from_millis(200),
            shutdown.clone(),
        )
    });

    let binding = match bind_override {
        Some(s) if s.starts_with(r"\\.\pipe\") => IpcBinding::NamedPipe(s.to_string()),
        Some(s) => IpcBinding::UnixSocket(PathBuf::from(s)),
        None => IpcBinding::default_for_platform(),
    };

    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "ctrl_c handler install failed");
            return;
        }
        tracing::info!("ctrl-c received; shutting down");
        signal_shutdown.notify_waiters();
    });

    // Phase14c — adapter heartbeat. Bumps `last_heartbeat_ts_ms`
    // every HEARTBEAT_INTERVAL_SECS so the cortex-api freshness
    // aggregator can distinguish "adapter idle" from "adapter dead"
    // without relying on Claude Code firing hooks. Synthetic — does
    // NOT touch any per-kind frame/envelope timestamp, so the
    // dashboard's per-kind rows stay honest about real Claude Code
    // activity.
    let heartbeat_metrics = metrics.clone();
    let heartbeat_shutdown = shutdown.clone();
    tokio::spawn(async move {
        let mut tick =
            tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        // Fire one tick immediately so the gauge is non-zero from
        // boot — otherwise a /healthz probe in the first 30 s would
        // see `last_heartbeat_ts_ms = 0` and the aggregator would
        // fall back to per-kind rows.
        tick.tick().await;
        heartbeat_metrics.record_heartbeat_now();
        loop {
            tokio::select! {
                _ = tick.tick() => heartbeat_metrics.record_heartbeat_now(),
                _ = heartbeat_shutdown.notified() => break,
            }
        }
    });

    // Phase8a — admin /healthz listener. Spawned independently so a
    // crash in the IPC path doesn't break health probes (and vice
    // versa). Port comes from CORTEX_ADAPTER_ADMIN_PORT (default
    // 17011 — published in cortex_health::DEFAULT_ADAPTER_ADMIN_PORT).
    let admin_port = typed_cfg
        .adapter
        .adapter_admin_port
        .unwrap_or(cortex_health::DEFAULT_ADAPTER_ADMIN_PORT);
    let metrics_for_health = metrics.clone();
    let started_at = chrono::Utc::now().to_rfc3339();
    // The IPC pipe is alive once `serve()` starts accepting; stamp
    // it true here and rely on `serve()` to flip it false on
    // shutdown (out of scope for this commit — the health stays
    // honest while the process is up).
    metrics.set_ipc_pipe_alive(true);
    // Phase8c — capture version block once at boot (compile-time
    // env vars never change for the running process), avoid re-
    // building it on every /healthz probe.
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        use cortex_health::server::HealthSnapshot;
        use cortex_health::{HealthState, DEFAULT_FRESHNESS_DEGRADED_SECS};
        let mut extras = serde_json::Map::new();
        // Phase8c — `version` block lets the cortex-api drift
        // aggregator detect "running binary != source HEAD".
        extras.insert("version".into(), version_block.clone());
        extras.insert(
            "runtime_profile".into(),
            serde_json::json!(if daily { "daily" } else { "full" }),
        );
        extras.insert(
            "bulk_capture_enabled".into(),
            serde_json::json!(bulk_capture),
        );
        extras.insert(
            "publisher_status".into(),
            serde_json::json!(if bulk_capture {
                "enabled"
            } else {
                "disabled_by_profile"
            }),
        );
        extras.insert(
            "wal_status".into(),
            serde_json::json!(if bulk_capture {
                "enabled"
            } else {
                "untouched_by_profile"
            }),
        );
        let queue_depth = metrics_for_health.publisher_queue_depth();
        let wal_bytes = metrics_for_health.wal_bytes();
        let last_publish_ok_ts_ms = metrics_for_health.last_publish_ok_ts_ms();
        let ipc_alive = metrics_for_health.ipc_pipe_alive();
        extras.insert(
            "publisher_queue_depth".into(),
            serde_json::json!(queue_depth),
        );
        extras.insert("wal_bytes".into(), serde_json::json!(wal_bytes));
        extras.insert(
            "last_publish_ok_ts_ms".into(),
            serde_json::json!(last_publish_ok_ts_ms),
        );
        extras.insert("ipc_pipe_alive".into(), serde_json::json!(ipc_alive));

        // Phase8b — surface the per-stage divergence counters and
        // freshness timestamps so the cortex-api freshness aggregator
        // can pivot without scraping `/metrics` in addition to
        // `/healthz`. Each map is a JSON object keyed by hook (frames)
        // or canonical kind (envelopes / publishes).
        let frames_received = metrics_for_health.frames_received_snapshot();
        let frames_parsed = metrics_for_health.frames_parsed_snapshot();
        let envelopes_built = metrics_for_health.envelopes_built_snapshot();
        let envelopes_publish_ok = metrics_for_health.envelopes_publish_ok_snapshot();
        let envelopes_publish_fail = metrics_for_health.envelopes_publish_fail_snapshot();
        let last_frame_ts = metrics_for_health.last_frame_ts_snapshot();
        let last_envelope_ts = metrics_for_health.last_envelope_ts_snapshot();
        let last_publish_ok_ts_by_kind = metrics_for_health.last_publish_ok_ts_by_kind_snapshot();
        extras.insert(
            "frames_received_total".into(),
            serde_json::to_value(&frames_received).unwrap_or_default(),
        );
        extras.insert(
            "frames_parsed_total".into(),
            serde_json::to_value(&frames_parsed).unwrap_or_default(),
        );
        extras.insert(
            "frames_parse_error_total".into(),
            serde_json::json!(metrics_for_health.frames_parse_error_total()),
        );
        extras.insert(
            "envelopes_built_total".into(),
            serde_json::to_value(&envelopes_built).unwrap_or_default(),
        );
        extras.insert(
            "envelopes_publish_ok_total".into(),
            serde_json::to_value(&envelopes_publish_ok).unwrap_or_default(),
        );
        extras.insert(
            "envelopes_publish_fail_total".into(),
            serde_json::to_value(&envelopes_publish_fail).unwrap_or_default(),
        );
        extras.insert(
            "last_frame_ts_ms".into(),
            serde_json::to_value(&last_frame_ts).unwrap_or_default(),
        );
        extras.insert(
            "last_envelope_ts_ms".into(),
            serde_json::to_value(&last_envelope_ts).unwrap_or_default(),
        );
        extras.insert(
            "last_publish_ok_ts_ms_by_kind".into(),
            serde_json::to_value(&last_publish_ok_ts_by_kind).unwrap_or_default(),
        );
        // Phase14c — heartbeat gauge. Bumped every
        // HEARTBEAT_INTERVAL_SECS by the background task above so
        // the cortex-api freshness aggregator can floor every
        // per-kind row's gap against this value (= "adapter is
        // alive but idle").
        let last_heartbeat_ts_ms = metrics_for_health.last_heartbeat_ts_ms();
        extras.insert(
            "last_heartbeat_ts_ms".into(),
            serde_json::json!(last_heartbeat_ts_ms),
        );
        // phase26e §2.1 — bundle-cache hit/miss counters from the
        // daemon's SyncClient. Lets the operator see the live
        // pre-thinking cache hit-rate via `/v1/health` cortex-adapter
        // extras (the cortex-api pre-thinking health source is unwired
        // and in a different process — phase26d gap B).
        let (cache_hit_total, cache_miss_total) = prethink_metrics_for_health.cache_counters();
        extras.insert(
            "pre_thinking_cache_hit_total".into(),
            serde_json::json!(cache_hit_total),
        );
        extras.insert(
            "pre_thinking_cache_miss_total".into(),
            serde_json::json!(cache_miss_total),
        );
        // phase26e §3 — TRUE pre-thinking bundle-assembly latency
        // quantiles (cortex.prethink.latency_ms), distinct from the
        // dashboard `pre_thinking_p95_ms` series (which is generic
        // envelope duration_ms — phase26d gap C). Surfacing the real
        // p50/p95/p99 here gives operators an accurate latency signal.
        let (lat_count, lat_p50, lat_p95, lat_p99) =
            prethink_metrics_for_health.latency_quantiles();
        extras.insert(
            "pre_thinking_latency_ms".into(),
            serde_json::json!({
                "count": lat_count,
                "p50": lat_p50,
                "p95": lat_p95,
                "p99": lat_p99,
            }),
        );
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        // Stall detection now honours the heartbeat: if the
        // heartbeat is fresh, the adapter is functionally alive
        // even when no publishes have landed recently (which is
        // the steady-state between Claude Code sessions).
        let heartbeat_fresh = last_heartbeat_ts_ms > 0
            && now_ms.saturating_sub(last_heartbeat_ts_ms)
                <= DEFAULT_FRESHNESS_DEGRADED_SECS * 1_000;
        let stale = !heartbeat_fresh
            && last_publish_ok_ts_ms > 0
            && now_ms.saturating_sub(last_publish_ok_ts_ms)
                > DEFAULT_FRESHNESS_DEGRADED_SECS * 1_000;
        let (state, last_error) = if !ipc_alive {
            (HealthState::Down, Some("ipc pipe not alive".to_string()))
        } else if stale || wal_bytes > 0 {
            (
                HealthState::Degraded,
                Some(format!(
                    "publisher stalled (last_publish_ok={last_publish_ok_ts_ms}ms, wal_bytes={wal_bytes})"
                )),
            )
        } else {
            (HealthState::Ok, None)
        };
        HealthSnapshot {
            state,
            last_error,
            extras,
        }
    });
    let admin_started_at = started_at.clone();
    // Phase8b — expose `/metrics` on the same admin port so a single
    // scrape target picks up both health JSON and the Prometheus-text
    // counters. The renderer reads from the shared metrics registry,
    // so the per-hook / per-kind counts are live per scrape.
    let metrics_for_prom = metrics.clone();
    let metrics_renderer: cortex_health::server::MetricsRenderer =
        std::sync::Arc::new(move || metrics_for_prom.render_prom());
    tokio::spawn(async move {
        if let Err(e) = cortex_health::server::serve_standalone_with_metrics(
            admin_port,
            "cortex-adapter",
            env!("CARGO_PKG_VERSION"),
            admin_started_at,
            provider,
            Some(metrics_renderer),
        )
        .await
        {
            tracing::warn!(error = %e, port = admin_port, "admin /healthz listener failed");
        }
    });
    tracing::info!(
        runtime_profile = if daily { "daily" } else { "full" },
        bulk_capture,
        admin_port,
        "cortex-adapter admin /healthz listening on http://0.0.0.0:{admin_port}/healthz"
    );

    tracing::info!(
        endpoint = %adapter.endpoint,
        api_endpoint = %adapter.api_endpoint,
        timeout_ms = adapter.timeout_ms,
        "cortex-adapter-claude daemon starting"
    );

    // Phase11w §2.4 — spawn the HTTP listener alongside the primary
    // socket/pipe binding when the typed `cortex_config::AdapterConfig.http_bind`
    // is set (which the ADR-016 env map populates from
    // `CORTEX_ADAPTER_HTTP_BIND`). The OpenCode TS plugin posts
    // here; the primary socket / named pipe keeps serving the
    // Claude Code hook shims.
    if typed_cfg.adapter.http_bind.is_some() {
        let http_binding = IpcBinding::default_http();
        let http_dispatcher = dispatcher.clone();
        let http_shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(http_binding, http_dispatcher, http_shutdown).await {
                tracing::warn!(error = %e, "ipc http listener exited with error");
            }
        });
    }

    serve(binding, dispatcher, shutdown).await
}

async fn runtime_publisher(
    daily: bool,
    adapter: &AdapterSection,
    wal_path: &std::path::Path,
    metrics: Arc<Metrics>,
) -> Result<Arc<dyn Publisher>> {
    // Branch before any WAL open/replay: daily must leave existing backlog
    // untouched and must start even when the bulk archive path is unavailable.
    if daily {
        return Ok(Arc::new(DisabledPublisher));
    }
    let wal = Arc::new(OverflowWal::open(wal_path).context("open overflow wal")?);
    let publisher = Arc::new(HttpPublisher::new(
        adapter.endpoint.clone(),
        adapter.queue_bounded,
        Duration::from_millis(adapter.timeout_ms),
        wal,
        metrics,
    ));
    publisher.replay_wal().await;
    Ok(publisher)
}

fn default_layout() -> Layout {
    Layout::from_home(&home_dir())
}

fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

fn init_tracing(verbose: bool) {
    let level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},cortex_adapter_claude_code={level}")));
    fmt().with_env_filter(filter).with_target(true).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE_ENV: &str = "ADAPTER_CREDENTIAL_PROBE";
    const SECRET: &str = "synthetic-adapter-credential-7c1d";

    /// Child half of the credential test: a no-op in a normal run. When
    /// re-executed with [`PROBE_ENV`] it runs the real daemon entry point,
    /// which must stop at the broken credential before binding anything.
    #[tokio::test]
    async fn run_daemon_credential_probe_child() {
        if std::env::var_os(PROBE_ENV).is_none() {
            return;
        }
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let bind = std::env::var("ADAPTER_CREDENTIAL_PROBE_BIND").unwrap();
        let error = run_daemon(Some(&home.join("adapter.toml")), Some(&bind))
            .await
            .expect_err("a broken credential must stop the daemon");
        println!("PROBE_ERROR {error:#}");
        println!("PROBE_ERROR_DEBUG {error:?}");
    }

    #[test]
    fn daemon_reads_home_token_file_and_hides_it_when_credential_is_broken() {
        let home = tempfile::tempdir().unwrap();
        let cortex_home = home.path().join("cortex home");
        std::fs::create_dir_all(&cortex_home).unwrap();
        // Oversized, so `load_client` must reject it; contents are the secret.
        let token_file = cortex_home.join(cortex_config::API_TOKEN_FILE);
        std::fs::write(&token_file, SECRET.repeat(4096 / SECRET.len() + 1)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let bind = if cfg!(windows) {
            format!(r"\\.\pipe\cortex-adapter-probe-{}", std::process::id())
        } else {
            home.path().join("probe.sock").display().to_string()
        };
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "tests::run_daemon_credential_probe_child",
            "--nocapture",
            "--test-threads=1",
        ]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("CORTEX_") {
                command.env_remove(&key);
            }
        }
        command
            .env(PROBE_ENV, "1")
            .env("ADAPTER_CREDENTIAL_PROBE_BIND", &bind)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("CORTEX_HOME", &cortex_home)
            .env("CORTEX_CONFIG_FILE", cortex_home.join("config.toml"))
            .env("CORTEX_ADAPTER_ADMIN_PORT", "0")
            .current_dir(home.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        // A regression that ignored the credential would start serving;
        // never let that daemon outlive the test.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("daemon kept running despite a broken credential");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("1 passed"), "probe did not run: {stdout}");
        let error = stdout
            .lines()
            .find_map(|line| line.split_once("PROBE_ERROR ").map(|(_, rest)| rest))
            .unwrap_or_else(|| panic!("no PROBE_ERROR in {stdout}"));
        assert!(
            error.contains("load Cortex runtime configuration"),
            "{error}"
        );
        assert!(error.contains(cortex_config::API_TOKEN_FILE), "{error}");
        assert!(error.contains("larger than 4096 bytes"), "{error}");
        assert!(!stdout.contains(SECRET), "token leaked: {stdout}");
        assert!(!stderr.contains(SECRET), "token leaked: {stderr}");
    }

    #[tokio::test]
    async fn daily_publisher_leaves_existing_wal_untouched_and_ignores_unavailable_path() {
        let directory = tempfile::tempdir().unwrap();
        let wal_path = directory.path().join("overflow.wal");
        let backlog = b"existing backlog must not be drained or rewritten\n";
        std::fs::write(&wal_path, backlog).unwrap();
        let adapter = AdapterSection::default();
        let metrics = Arc::new(Metrics::new());
        let publisher = runtime_publisher(true, &adapter, &wal_path, metrics.clone())
            .await
            .unwrap();
        assert!(!publisher.enabled());
        assert_eq!(publisher.queue_depth(), 0);
        assert_eq!(publisher.flush().await, 0);
        assert_eq!(std::fs::read(&wal_path).unwrap(), backlog);

        // A regular file cannot serve as the WAL's parent directory. Daily
        // startup must not depend on that path, or even attempt to create it.
        let impossible = wal_path.join("must-not-open.wal");
        let publisher = runtime_publisher(true, &adapter, &impossible, metrics.clone())
            .await
            .unwrap();
        assert!(!publisher.enabled());
        assert_eq!(std::fs::read(&wal_path).unwrap(), backlog);
        assert!(runtime_publisher(false, &adapter, &impossible, metrics)
            .await
            .is_err());
    }
}

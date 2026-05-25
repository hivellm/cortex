//! `cortex-adapter-claude` binary — daemon + install / uninstall /
//! status sub-commands. Spec 10 §Install / uninstall.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortex_adapter_claude_code::{
    install_with as install_layout_with, load_config, serve, spawn_flusher,
    uninstall as uninstall_layout, Dispatcher, HttpPublisher, InstallOptions, IpcBinding, Layout,
    Metrics, OverflowWal, SessionManager, SyncClient,
};
use tracing_subscriber::{fmt, EnvFilter};

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

    let metrics = Arc::new(Metrics::new());
    let wal_path = home_dir().join(".cortex").join("overflow.wal");
    let wal = Arc::new(OverflowWal::open(&wal_path).context("open overflow wal")?);
    let publisher = Arc::new(HttpPublisher::new(
        adapter.endpoint.clone(),
        adapter.queue_bounded,
        Duration::from_millis(adapter.timeout_ms),
        wal.clone(),
        metrics.clone(),
    ));
    let _replayed = publisher.replay_wal().await;

    let sync = Arc::new(SyncClient::new(&adapter, metrics.clone()));
    let sessions = Arc::new(SessionManager::new());
    let pid = std::process::id();
    let dispatcher = Arc::new(
        Dispatcher::new(sessions, publisher.clone(), sync, pid).with_metrics(metrics.clone()),
    );

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let _flusher = spawn_flusher(
        publisher.clone(),
        Duration::from_millis(200),
        shutdown.clone(),
    );

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

    // Phase8a — admin /healthz listener. Spawned independently so a
    // crash in the IPC path doesn't break health probes (and vice
    // versa). Port comes from CORTEX_ADAPTER_ADMIN_PORT (default
    // 17011 — published in cortex_health::DEFAULT_ADAPTER_ADMIN_PORT).
    let admin_port = cortex_config::Config::load()
        .ok()
        .and_then(|c| c.adapter.adapter_admin_port)
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
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let stale = last_publish_ok_ts_ms > 0
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
        admin_port,
        "cortex-adapter admin /healthz listening on http://0.0.0.0:{admin_port}/healthz"
    );

    tracing::info!(
        endpoint = %adapter.endpoint,
        api_endpoint = %adapter.api_endpoint,
        timeout_ms = adapter.timeout_ms,
        "cortex-adapter-claude daemon starting"
    );
    serve(binding, dispatcher, shutdown).await
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

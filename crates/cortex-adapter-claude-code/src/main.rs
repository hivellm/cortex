//! `cortex-adapter-claude` binary — daemon + install / uninstall /
//! status sub-commands. Spec 10 §Install / uninstall.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortex_adapter_claude_code::{
    install as install_layout, load_config, serve, spawn_flusher, uninstall as uninstall_layout,
    Dispatcher, HttpPublisher, IpcBinding, Layout, Metrics, OverflowWal, SessionManager,
    SyncClient,
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
    Install,
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
        Command::Install => {
            let layout = default_layout();
            let report = install_layout(&layout)?;
            println!("installed {} hook shims", report.hooks_written.len());
            if report.settings_modified {
                println!("settings.json patched at {}", layout.settings_path.display());
            }
            Ok(())
        }
        Command::Uninstall { purge } => {
            let layout = default_layout();
            let report = uninstall_layout(&layout, purge)?;
            if report.settings_modified {
                println!("settings.json restored at {}", layout.settings_path.display());
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
    let cfg_path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(|| {
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
    let dispatcher = Arc::new(Dispatcher::new(
        sessions,
        publisher.clone(),
        sync,
        pid,
    ));

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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("{level},cortex_adapter_claude_code={level}"))
    });
    fmt().with_env_filter(filter).with_target(true).init();
}

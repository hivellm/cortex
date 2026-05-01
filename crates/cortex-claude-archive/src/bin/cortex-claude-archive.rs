//! `cortex-claude-archive` CLI — phase11i §1.7.
//!
//! Subcommands:
//!
//! - `estimate` — count files + bytes + projected envelopes;
//!   no writes. Used by the operator to size the bootstrap pass
//!   before committing to it.
//!
//! - `bootstrap` — full one-shot ingest. Walks every selected
//!   file, parses + maps every record, ships envelopes through
//!   the chosen sink. Honours `--resume` against the persisted
//!   checkpoint so a crashed run picks up where it left off.
//!
//! - `tail` — long-running watcher. The full implementation
//!   ships in §1.5 follow-up; for now this subcommand prints a
//!   message naming the resolved root and exits 0 so the CLI
//!   surface is forward-compatible.
//!
//! Sinks:
//!
//! - `--sink stdout` — one canonical-JSON line per envelope.
//!   Cheap; default; pipes well into `jq`.
//! - `--sink archive` — zstd-NDJSON files under
//!   `<CORTEX_ARCHIVE_ROOT>/events/year=…/…/bootstrap-claude-NNNNN.parquet`.
//!   `cortex-api`'s `archive_loader` re-reads them at boot.
//!
//! On Windows the default `--root` is
//! `%USERPROFILE%/.claude`; on Unix it is `$HOME/.claude`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cortex_claude_archive::{
    map_session, read_records, walk, ArchiveEmitter, CheckpointStore, Emitter, MapStats, ReadStats,
    StdoutEmitter, WalkConfig, WalkEntry, WalkKind,
};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Parser)]
#[command(
    name = "cortex-claude-archive",
    about = "Ingest Claude Code's ~/.claude/projects/*.jsonl conversation archive into Cortex.",
    version
)]
struct Cli {
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Count files + projected envelopes without emitting.
    Estimate(EstimateArgs),
    /// Full one-shot ingest of every selected file.
    Bootstrap(BootstrapArgs),
    /// Long-running watcher — emits new envelopes as sessions
    /// append. Full implementation ships in §1.5 follow-up.
    Tail(TailArgs),
}

#[derive(Debug, clap::Args)]
struct CommonRootArgs {
    /// Root containing `projects/` + sidecars. Defaults to
    /// `~/.claude`.
    #[arg(long, value_name = "PATH")]
    root: Option<PathBuf>,
    /// Walk only `<root>/projects/` (default).
    #[arg(long, default_value_t = true)]
    projects_only: bool,
    /// Also walk sidecars (history.jsonl, todos/, plans/,
    /// settings.json).
    #[arg(long, default_value_t = false)]
    sidecars: bool,
    /// Also walk `~/.codex/` parallel corpus.
    #[arg(long, default_value_t = false)]
    codex: bool,
}

impl CommonRootArgs {
    fn resolve(&self) -> Result<WalkConfig> {
        let root = match &self.root {
            Some(p) => p.clone(),
            None => default_claude_root()?,
        };
        // `--sidecars` implies `--projects-only=false`. The flag
        // pair is otherwise wired so the caller can explicitly
        // select projects-only OR sidecars-on; the implication
        // here covers the natural case.
        let projects_only = if self.sidecars {
            false
        } else {
            self.projects_only
        };
        Ok(WalkConfig {
            root,
            projects_only,
            sidecars: self.sidecars,
            codex: self.codex,
        })
    }
}

#[derive(Debug, clap::Args)]
struct EstimateArgs {
    #[command(flatten)]
    common: CommonRootArgs,
}

#[derive(Debug, clap::Args)]
struct BootstrapArgs {
    #[command(flatten)]
    common: CommonRootArgs,
    /// Where to send the emitted envelopes.
    #[arg(long, value_enum, default_value_t = SinkChoice::Stdout)]
    sink: SinkChoice,
    /// Archive root (only used when `--sink archive`). Defaults
    /// to `$CORTEX_ARCHIVE_ROOT` or `~/.cortex/archive`.
    #[arg(long, value_name = "PATH")]
    archive_root: Option<PathBuf>,
    /// Resume from the checkpoint at `--checkpoint`. Without this
    /// flag the bootstrap re-emits everything (idempotent — the
    /// archive_loader dedupes by event_id, but the runtime cost
    /// is doubled).
    #[arg(long, default_value_t = false)]
    resume: bool,
    /// Path of the checkpoint file. Defaults to
    /// `<root>/.cortex-claude-archive.checkpoint.json`.
    #[arg(long, value_name = "PATH")]
    checkpoint: Option<PathBuf>,
    /// Skip files whose path matches one of these substrings.
    /// Repeatable.
    #[arg(long = "skip", value_name = "SUBSTRING")]
    skip: Vec<String>,
}

#[derive(Debug, clap::Args)]
struct TailArgs {
    #[command(flatten)]
    common: CommonRootArgs,
    /// Where to ship envelopes. The watcher defaults to the
    /// archive sink so cortex-api's archive_loader picks them up
    /// at boot; flip to stdout for local debugging.
    #[arg(long, value_enum, default_value_t = SinkChoice::Archive)]
    sink: SinkChoice,
    /// Archive root (only used when `--sink archive`). Defaults to
    /// `$CORTEX_ARCHIVE_ROOT` or `~/.cortex/archive`.
    #[arg(long, value_name = "PATH")]
    archive_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum SinkChoice {
    Stdout,
    Archive,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match cli.command {
        Command::Estimate(args) => run_estimate(args),
        Command::Bootstrap(args) => run_bootstrap(args),
        Command::Tail(args) => run_tail(args),
    }
}

fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")
}

fn init_tracing(verbose: bool) {
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .try_init();
}

fn run_estimate(args: EstimateArgs) -> Result<()> {
    let cfg = args.common.resolve()?;
    let entries = walk(&cfg);
    let bytes_total: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let session_count = entries
        .iter()
        .filter(|e| matches!(e.kind, WalkKind::Session | WalkKind::CodexSession))
        .count();
    let sidecar_count = entries.len() - session_count;
    println!("root            : {}", cfg.root.display());
    println!(
        "scan mode       : projects_only={} sidecars={} codex={}",
        cfg.projects_only, cfg.sidecars, cfg.codex
    );
    println!("session files   : {session_count}");
    println!("sidecar files   : {sidecar_count}");
    println!(
        "bytes total     : {} ({} MiB)",
        bytes_total,
        bytes_total / (1024 * 1024)
    );
    // Rough envelope estimate: ~30% of records map to Turn /
    // ToolCall / AgentCall (the rest are attachment / system /
    // file-history snapshots the mapper drops). 800 records per
    // session is the corpus-wide average per the corpus inventory
    // analysis.
    let envelopes_estimate = (session_count as u64) * 800 * 30 / 100;
    println!("envelopes (est.): {envelopes_estimate}");
    Ok(())
}

fn run_bootstrap(args: BootstrapArgs) -> Result<()> {
    let cfg = args.common.resolve()?;
    let entries = walk(&cfg);
    let entries: Vec<WalkEntry> = entries
        .into_iter()
        .filter(|e| {
            args.skip
                .iter()
                .all(|needle| !e.path.to_string_lossy().contains(needle))
        })
        .collect();

    let checkpoint_path = args
        .checkpoint
        .clone()
        .unwrap_or_else(|| cfg.root.join(".cortex-claude-archive.checkpoint.json"));
    let mut checkpoint = CheckpointStore::load(&checkpoint_path).context("loading checkpoint")?;

    let emitter: Box<dyn Emitter> = match args.sink {
        SinkChoice::Stdout => Box::new(StdoutEmitter::new()),
        SinkChoice::Archive => {
            let root = match args.archive_root {
                Some(p) => p,
                None => default_archive_root()?,
            };
            Box::new(ArchiveEmitter::new(root, 3))
        }
    };

    let bar = ProgressBar::new(entries.len() as u64);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files · {msg}",
        )
        .expect("progress style")
        .progress_chars("=>-"),
    );

    let envelopes_emitted = AtomicUsize::new(0);
    let envelopes_dropped = AtomicUsize::new(0);
    let records_consumed = AtomicUsize::new(0);
    let mut last_checkpoint_save = std::time::Instant::now();

    for entry in &entries {
        bar.set_message(format!(
            "{}/{}",
            entry.project_dir,
            entry
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        ));

        // §1.7 v1: only Session + CodexSession entries get the
        // full reader+mapper treatment. Sidecar variants are
        // tracked in the walker but the projector for each lands
        // in §1.5 follow-up alongside the watcher path.
        let is_session = matches!(entry.kind, WalkKind::Session | WalkKind::CodexSession);
        if !is_session {
            tracing::debug!(
                path = %entry.path.display(),
                kind = ?entry.kind,
                "skipping sidecar entry — projector lands in §1.5 follow-up",
            );
            bar.inc(1);
            continue;
        }

        let session_filename = entry
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Skip entries already past the checkpoint when --resume
        // is set. The phase11i §1 v1 checkpoint lives at file
        // granularity; future versions track byte offset for
        // partially-processed files.
        if args.resume {
            if checkpoint
                .get(&entry.project_dir, &session_filename)
                .is_some()
            {
                bar.inc(1);
                continue;
            }
        }

        let (records, read_stats) = match read_records(&entry.path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    path = %entry.path.display(),
                    err = %e,
                    "read failed; skipping file",
                );
                bar.inc(1);
                continue;
            }
        };
        log_read_stats(&entry.path, &read_stats);

        let (envelopes, map_stats) = map_session(&records, None)?;
        log_map_stats(&entry.path, &map_stats);

        let mut last_uuid = None;
        for env in &envelopes {
            match emitter.emit(env) {
                Ok(()) => {
                    envelopes_emitted.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!(
                        path = %entry.path.display(),
                        err = %e,
                        "emit failed; counted as dropped",
                    );
                    envelopes_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            last_uuid = Some(env.event_id.clone());
        }
        records_consumed.fetch_add(map_stats.records_consumed, Ordering::Relaxed);

        // Stamp checkpoint after every file so a crash mid-run
        // never re-emits more than one file's worth of work.
        if let Some(uuid) = last_uuid {
            let cp = cortex_claude_archive::Checkpoint {
                session_id: envelopes
                    .last()
                    .map(|e| e.session_id.clone())
                    .unwrap_or_default(),
                last_record_uuid: uuid,
                last_byte_offset: 0,
                written_at_ms: chrono::Utc::now().timestamp_millis(),
            };
            checkpoint.put(&entry.project_dir, &session_filename, cp);
        }
        if last_checkpoint_save.elapsed().as_secs() >= 5 {
            checkpoint.save_atomic().context("saving checkpoint")?;
            last_checkpoint_save = std::time::Instant::now();
        }

        bar.inc(1);
    }

    emitter.flush().context("flushing emitter")?;
    checkpoint
        .save_atomic()
        .context("saving final checkpoint")?;
    bar.finish_with_message("done");

    println!();
    println!(
        "envelopes emitted : {}",
        envelopes_emitted.load(Ordering::Relaxed)
    );
    println!(
        "envelopes dropped : {}",
        envelopes_dropped.load(Ordering::Relaxed)
    );
    println!(
        "records consumed  : {}",
        records_consumed.load(Ordering::Relaxed)
    );
    println!("checkpoint        : {}", checkpoint_path.display());
    Ok(())
}

fn run_tail(args: TailArgs) -> Result<()> {
    let cfg = args.common.resolve()?;
    let emitter: Box<dyn Emitter> = match args.sink {
        SinkChoice::Stdout => Box::new(StdoutEmitter::new()),
        SinkChoice::Archive => {
            let root = match args.archive_root {
                Some(p) => p,
                None => default_archive_root()?,
            };
            Box::new(ArchiveEmitter::new(root, 3))
        }
    };
    let bind = cortex_claude_archive::tail::resolve_health_bind()?;
    let interval = cortex_claude_archive::tail::resolve_poll_interval();
    tracing::info!(
        root = %cfg.root.display(),
        sink = ?args.sink,
        bind = %bind,
        interval_ms = interval.as_millis() as u64,
        "tail watcher starting (phase11i §5.2)"
    );
    let rt = build_runtime()?;
    rt.block_on(cortex_claude_archive::tail::run_tail_daemon(
        cfg, emitter, bind, interval,
    ))
}

fn log_read_stats(path: &std::path::Path, stats: &ReadStats) {
    if stats.malformed_lines > 0 || stats.unknown_kinds > 0 || stats.typeless_records > 0 {
        tracing::info!(
            path = %path.display(),
            lines = stats.lines_read,
            parsed = stats.records_parsed,
            malformed = stats.malformed_lines,
            unknown = stats.unknown_kinds,
            typeless = stats.typeless_records,
            "read stats (with anomalies)",
        );
    } else {
        tracing::debug!(
            path = %path.display(),
            parsed = stats.records_parsed,
            "read stats",
        );
    }
}

fn log_map_stats(path: &std::path::Path, stats: &MapStats) {
    tracing::debug!(
        path = %path.display(),
        consumed = stats.records_consumed,
        turns = stats.turns_emitted,
        tool_calls = stats.tool_calls_emitted,
        agent_calls = stats.agent_calls_emitted,
        dropped = stats.dropped_records,
        sessionless = stats.sessionless_records,
        "map stats",
    );
}

fn default_claude_root() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Ok(PathBuf::from(home).join(".claude"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join(".claude"));
    }
    Err(anyhow!(
        "could not resolve a default --root; set USERPROFILE / HOME or pass --root explicitly"
    ))
}

fn default_archive_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CORTEX_ARCHIVE_ROOT") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Ok(PathBuf::from(home).join(".cortex").join("archive"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join(".cortex").join("archive"));
    }
    Err(anyhow!(
        "could not resolve a default --archive-root; set CORTEX_ARCHIVE_ROOT / USERPROFILE / HOME or pass --archive-root explicitly"
    ))
}

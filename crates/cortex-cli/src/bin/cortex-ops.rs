//! `cortex-ops` — local-stack operator CLI.
//!
//! Subcommands:
//! - `plan` — prints the expected Vectorizer collections, Nexus Cypher,
//!   Meilisearch indexes, Synap streams/KV namespaces in JSON. Useful as
//!   the source the seed scripts consume.
//! - `doctor` — pokes every backend and reports liveness.
//!
//! This CLI never mutates external state directly in v1 — that job belongs
//! to `cortex-api` (spec 04) and the workers (specs 05–08). `cortex-ops
//! plan` is what `bin/cortex-init.sh` feeds into each backend's native
//! create API.

#![forbid(unsafe_code)]

// phase11w — submodules live under `src/bin/cortex-ops/`; rustc's
// default search for `mod <name>;` in a bin file is `src/bin/<name>.rs`
// (sibling to the bin), so we point each module at the correct path
// explicitly. Keeps the bin's submodule layout self-contained.
#[path = "cortex-ops/acl.rs"]
mod acl_cmd;
#[path = "cortex-ops/backfill_cross_project.rs"]
mod backfill_cross_project;
#[path = "cortex-ops/bootstrap.rs"]
mod bootstrap;
#[path = "cortex-ops/branch_cmd.rs"]
mod branch_cmd;
#[path = "cortex-ops/canary.rs"]
mod canary;
#[path = "cortex-ops/cas.rs"]
mod cas;
#[path = "cortex-ops/config_audit.rs"]
mod config_audit;
#[path = "cortex-ops/consolidation.rs"]
mod consolidation;
#[path = "cortex-ops/decisions_reindex.rs"]
mod decisions_reindex;
#[path = "cortex-ops/digest.rs"]
mod digest;
#[path = "cortex-ops/doctor.rs"]
mod doctor;
#[path = "cortex-ops/doctor_redaction_coverage.rs"]
mod doctor_redaction_coverage;
#[path = "cortex-ops/doctor_registry_sync.rs"]
mod doctor_registry_sync;
#[path = "cortex-ops/doctor_smoke.rs"]
mod doctor_smoke;
#[path = "cortex-ops/doctor_synap_workers.rs"]
mod doctor_synap_workers;
#[path = "cortex-ops/graph_cmd.rs"]
mod graph_cmd;
#[path = "cortex-ops/helpers.rs"]
mod helpers;
#[path = "cortex-ops/identity_coverage.rs"]
mod identity_coverage;
#[path = "cortex-ops/intent_stats.rs"]
mod intent_stats;
#[path = "cortex-ops/laws_reindex.rs"]
mod laws_reindex;
#[path = "cortex-ops/laws_repair.rs"]
mod laws_repair;
#[path = "cortex-ops/meili.rs"]
mod meili;
#[path = "cortex-ops/meili_audit.rs"]
mod meili_audit;
#[path = "cortex-ops/meili_rekey.rs"]
mod meili_rekey;
#[path = "cortex-ops/memory_consolidate_cmd.rs"]
mod memory_consolidate_cmd;
#[path = "cortex-ops/metadata.rs"]
mod metadata;
#[path = "cortex-ops/migrate_classification.rs"]
mod migrate_classification;
#[path = "cortex-ops/pii.rs"]
mod pii;
#[path = "cortex-ops/plan.rs"]
mod plan;
#[path = "cortex-ops/query_cmd.rs"]
mod query_cmd;
#[path = "cortex-ops/retention.rs"]
mod retention;
#[path = "cortex-ops/retention_archive_purge.rs"]
mod retention_archive_purge;
#[path = "cortex-ops/rollup.rs"]
mod rollup;
#[path = "cortex-ops/schedule_cmd.rs"]
mod schedule_cmd;
#[path = "cortex-ops/sessions_backfill.rs"]
mod sessions_backfill;
#[path = "cortex-ops/temporal_digest.rs"]
mod temporal_digest;
#[path = "cortex-ops/timeline.rs"]
mod timeline;
#[path = "cortex-ops/timeline_backfill.rs"]
mod timeline_backfill;
#[path = "cortex-ops/tool_call_digest_live.rs"]
mod tool_call_digest_live;
#[path = "cortex-ops/turn_digest_live.rs"]
mod turn_digest_live;
#[path = "cortex-ops/watchdog.rs"]
mod watchdog;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "cortex-ops", version, about = "Cortex operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Emit the bootstrap plan (collections / Cypher / indexes / streams) as JSON.
    Plan {
        /// Pretty-print the JSON (default: compact).
        #[arg(long)]
        pretty: bool,
        /// Which slice to emit. Default: all.
        #[arg(long, value_enum, default_value_t = PlanSlice::All)]
        slice: PlanSlice,
    },
    /// Poke every backend at the configured URL and report status.
    Doctor {
        /// Vectorizer base URL (defaults to `$VECTORIZER_URL`).
        #[arg(long)]
        vectorizer: Option<String>,
        /// Nexus base URL (defaults to `$NEXUS_URL`).
        #[arg(long)]
        nexus: Option<String>,
        /// Synap base URL (defaults to `$SYNAP_URL`).
        #[arg(long)]
        synap: Option<String>,
        /// Meilisearch base URL (defaults to `$MEILI_URL`).
        #[arg(long)]
        meili: Option<String>,
    },
    /// Phase12g §1 — Meilisearch index audit. For every index in
    /// `cortex_storage::fulltext::INDEXES`, fetches the live
    /// `numberOfDocuments` from Meili `/stats` and classifies each
    /// row as `populated` / `empty` / `missing` / `orphan`. Catches
    /// the operational shape that bit phase12g (rulebook + vectorizer
    /// indexes shipping configured-but-empty so every query against
    /// those repos returned zero hits). Pairs with the existing
    /// `doctor-meili-indexes` (settings drift) — together they cover
    /// the two "is the index healthy?" axes. Exit `0` when every row
    /// is `populated`; `2` on any drift or transport failure.
    MeiliAudit {
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase12g §2 — Meilisearch reindex. Wraps the production
    /// worker's boot-time replay path
    /// (`cortex-workers::fulltext::boot_replay::replay_missing_partitions`),
    /// which walks the parquet archive for every `(slug, family)`
    /// partition not already present in Meili and routes each
    /// envelope through the standard indexer pipeline. Idempotent —
    /// re-running upserts via Meili `addDocuments` semantics, so
    /// recovery from a partial run is safe. Returns the
    /// `ReplayReport` (examined archives, missing partitions,
    /// replayed events, latency).
    MeiliReindex {
        /// Override the archive root. Defaults to
        /// `$CORTEX_ARCHIVE_ROOT` → `$CORTEX_HOME/archive` →
        /// `<HOME|USERPROFILE>/.cortex/archive`.
        #[arg(long)]
        archive_root: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },

    /// Phase12b — bulk Parquet archive purge. Walks
    /// `${CORTEX_HOME}/events/**/*.parquet`, deletes every file whose
    /// newest envelope's `occurred_at` is strictly older than
    /// `--before`. Replaces the per-event `/v1/admin/forget` path
    /// operators were avoiding by reaching for `rm -rf`. Honours the
    /// live-frame guard so a half-flushed current-hour file is never
    /// deleted; `--repo <slug>` pins files to the named repo (a
    /// mixed-repo file is preserved even if every other envelope is
    /// old). Exit code: `0` when every classifiable file was either
    /// kept or deleted cleanly; `2` when at least one file was
    /// unreadable or a per-file delete failed (the report counts both
    /// in `files_unreadable`).
    RetentionArchivePurge {
        /// RFC-3339 cutoff. Files whose newest envelope is `< before`
        /// are deleted. Required.
        #[arg(long, value_name = "RFC3339")]
        before: String,
        /// Print the report without removing any file.
        #[arg(long)]
        dry_run: bool,
        /// Restrict deletion to envelopes whose `context.repo`
        /// matches. Files mixing this repo with others are kept.
        #[arg(long)]
        repo: Option<String>,
        /// Override the archive home directory. Defaults to
        /// `$CORTEX_HOME` then `<HOME|USERPROFILE>/.cortex`.
        #[arg(long)]
        home: Option<String>,
    },

    /// phase0 §5 — coverage / health watchdog. Probes the archive
    /// watcher, the `retention_sweeps` table, and the pruner-status
    /// file, then raises alarms (exit `1` warn / `2` critical) when
    /// ingestion / sweeps / consolidation go silent. The
    /// `health.watchdog` seed job runs this on a cadence so silent
    /// failures surface without an operator looking.
    Watchdog {
        /// Archive watcher base URL. Defaults to the first entry of
        /// `CORTEX_ARCHIVE_WATCHER_URLS` then `http://localhost:17030`.
        #[arg(long)]
        watcher_url: Option<String>,
        /// Seconds without an emitter flush before `ingest_stale`
        /// warns. Default 3600 (1 h).
        #[arg(long)]
        ingest_staleness_secs: Option<i64>,
        /// Seconds since the last retention sweep before `sweep_stale`
        /// warns. Default 90000 (~25 h).
        #[arg(long)]
        sweep_staleness_secs: Option<i64>,
        /// Seconds since the last consolidation before
        /// `consolidation_stale` warns. Default 172800 (48 h).
        #[arg(long)]
        consolidation_staleness_secs: Option<i64>,
        /// Override the home directory used to locate
        /// `metadata.sqlite` and `pruner-status.json`.
        #[arg(long)]
        home: Option<String>,
        /// Emit the report as JSON instead of human lines.
        #[arg(long)]
        json: bool,
    },

    /// Phase9f — Meilisearch archival pruner. Blanks turn /
    /// tool_call body fields older than 90 d, caps `summary` at
    /// 4 KiB with an ellipsis marker, and stamps `pruned: true` +
    /// `pruned_at`. Documents are NEVER deleted — the keyword lane
    /// surfaces them on a `summary` match. Idempotent on re-run;
    /// `--rebuild` re-prunes already-pruned docs.
    ///
    /// Today's CLI runs against an in-memory backend preview; the
    /// production walker (Meili `update_documents` task with
    /// terminal-state await) lands with phase9k's cron scheduler.
    MeiliPrune {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the candidate set without backend mutation.
        #[arg(long)]
        dry_run: bool,
        /// Re-prune already-pruned documents.
        #[arg(long)]
        rebuild: bool,
        /// Maximum docs per Meili `update_documents` task.
        #[arg(long, default_value_t = 1_000)]
        batch_size: u32,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9e — LLM turn digest summarizer. Buckets `turn` envelopes
    /// older than 30 d by `(repo, ISO_year_week, top_topic)` and
    /// emits one `:Memory{memory_type="turn_digest"}` per non-empty
    /// bucket with size ≥ 5. With `--purge-originals` set, hard-
    /// deletes the source rows from Meili (`cortex-<repo>-turns`),
    /// Vectorizer (`cortex.turn.fp32` / `.pq` / `.cold.binary`), and
    /// the matching Parquet partitions after the digest persists.
    /// Default mode is a synthetic preview against an in-process
    /// backend; pass `--apply` to switch to the live Meili +
    /// cortex-ingestion + cortex-api wiring (phase11x).
    TurnDigest {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Skip every backend mutation; surface buckets as pending.
        #[arg(long)]
        dry_run: bool,
        /// Re-summarise buckets that already have a digest.
        #[arg(long)]
        rebuild: bool,
        /// Per-run budget ceiling in US cents.
        #[arg(long, default_value_t = 500)]
        budget_cents: u64,
        /// Use the live Meili enumerator + cortex-ingestion +
        /// cortex-api backend. Without this flag, the handler runs
        /// the synthetic preview against the in-process backend.
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// **Destructive flag.** When set AND `--dry-run` is off
        /// AND `--apply` is on, hard-purges every source turn row
        /// after the digest persists. Default `false`.
        #[arg(long, default_value_t = false)]
        purge_originals: bool,
        /// Maximum rows to digest per run. Bounds memory + cron
        /// runtime when the backlog is huge.
        #[arg(long, default_value_t = 50_000)]
        max_records: usize,
        /// Page size for the Meili enumerator.
        #[arg(long, default_value_t = 1_000)]
        page_size: u32,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// phase11w — Tool-call digest summariser. Buckets `tool_call`
    /// envelopes older than 30 d by `(repo, year_week, tool)` and
    /// emits one `:Memory{memory_type=tool_call_digest}` per bucket
    /// with size ≥ 5. With `--purge-originals` set, hard-deletes the
    /// source rows from Meili (`cortex_tool_calls`), Vectorizer
    /// (`cortex.tool_call.fp32` / `.pq` / `.cold.binary`), and the
    /// matching Parquet partitions after the digest persists.
    /// Default mode is a synthetic preview against an in-process
    /// backend; pass `--apply` to switch to the live Meili +
    /// cortex-ingestion + cortex-api wiring.
    ToolCallDigest {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Skip every backend mutation; surface buckets as pending.
        #[arg(long)]
        dry_run: bool,
        /// Re-summarise buckets that already have a digest.
        #[arg(long)]
        rebuild: bool,
        /// Per-run budget ceiling in US cents.
        #[arg(long, default_value_t = 500)]
        budget_cents: u64,
        /// Use the live Meili enumerator + cortex-ingestion +
        /// cortex-api backend. Without this flag, the handler runs
        /// the synthetic preview against the in-process backend.
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// **Destructive flag.** When set AND `--dry-run` is off
        /// AND `--apply` is on, hard-purges every source tool_call
        /// row after the digest persists. Default `false`.
        #[arg(long, default_value_t = false)]
        purge_originals: bool,
        /// Maximum rows to digest per run. Bounds memory + cron
        /// runtime when the backlog is huge.
        #[arg(long, default_value_t = 50_000)]
        max_records: usize,
        /// Vectorizer-style page size for the Meili enumerator.
        #[arg(long, default_value_t = 1_000)]
        page_size: u32,
        /// Override `digest_after_days` (default 30). Operator-only
        /// escape hatch for one-shot manual purges; cron continues
        /// to honour the spec default.
        #[arg(long)]
        age_days: Option<i64>,
        /// Override `min_bucket_size` (default 5). Set to 1 to force
        /// every bucket through the classifier regardless of size.
        #[arg(long)]
        min_bucket_size: Option<usize>,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9d — PII retention enforcement. Drops `pii_risk=high`
    /// raw payloads at 30 d (Parquet body blanked, Vectorizer +
    /// Meili records purged, CAS refcount decremented) and re-
    /// summarizes `pii_risk=medium` records at 90 d via the
    /// classifier. Records with `pii_risk=null` enter the medium
    /// path automatically (defaulting to `low` would silently
    /// retain unclassified PII).
    ///
    /// The library surface (`cortex_workers::retention::pii_enforce`)
    /// exposes the matcher + cohort logic + run_enforcement
    /// orchestrator. The production backend (live Vectorizer /
    /// Meili / CAS / classifier wiring) lands when phase9k's cron
    /// scheduler integrates the retention jobs end-to-end. Today's
    /// CLI surface is a documentation + dry-run probe so operators
    /// can preview cohort assignments against synthetic targets.
    PiiEnforce {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the cohort assignment without backend mutation.
        #[arg(long)]
        dry_run: bool,
        /// Limit to one cohort.
        #[arg(long)]
        cohort: Option<String>,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9c — CAS vacuum. Deletes blobs whose `refcount = 0`
    /// AND `last_referenced < now - 30 d`, then `VACUUM`s the
    /// metadata DB when the freelist exceeds 25 % of pages.
    /// Refuses to drop more than 50 % of total blobs without
    /// `--force` (catastrophic-deletion safeguard).
    CasVacuum {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the candidate set + projected reclamation without
        /// mutating the CAS store.
        #[arg(long)]
        dry_run: bool,
        /// Override the catastrophic-deletion safeguard.
        #[arg(long)]
        force: bool,
        /// Path to the CAS SQLite file.
        #[arg(long)]
        cas_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9k — cron scheduler for retention sweeps.
    ///
    /// Subcommands manage the `cron_jobs` registry that the
    /// cortex-ops daemon ticks every 30 s.
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommand,
    },
    /// 2026-05-20 — populate the `sessions` SQLite table from the
    /// parquet archive. The ingestion router never wrote here, which
    /// left the consolidator nightly's 24h window enumeration always
    /// returning zero rows — daily consolidations never fired in
    /// production. This subcommand walks the archive, aggregates
    /// envelopes by `session_id`, and `upsert_session`s each one with
    /// its earliest `occurred_at` as `started_at`. Idempotent;
    /// scheduled hourly so new sessions appear within an hour of
    /// their first envelope landing.
    /// Phase21 §2.7 — backfill classification columns on events that
    /// predate the classification stamper. Dry-run by default; pass
    /// `--no-dry-run` to record imputed counts (graph writes are
    /// wired in phase21 §3.2). Exits `2` when anomalies are found.
    MigrateClassification {
        /// Parquet archive root. Defaults to `$CORTEX_ARCHIVE_ROOT`
        /// / `<CORTEX_HOME>/archive` / `<home>/.cortex/archive`.
        #[arg(long)]
        archive_root: Option<String>,
        /// Restrict scan to a single project slug (lower-cased repo).
        /// Omit to scan all projects.
        #[arg(long)]
        project: Option<String>,
        /// Sensitivity level to impute on rows lacking `class_level`
        /// (0=public, 1=internal, 2=confidential, 3=restricted).
        #[arg(long, default_value_t = 0)]
        default_level: u8,
        /// Count missing rows and report anomalies without writing.
        #[arg(long, default_value_t = true)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    SessionsBackfill {
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` / `<CORTEX_HOME>/metadata.sqlite` /
        /// `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Parquet archive root. Defaults to `$CORTEX_ARCHIVE_ROOT`
        /// / `<CORTEX_HOME>/archive` / `<home>/.cortex/archive`.
        #[arg(long)]
        archive_root: Option<String>,
        /// Walk the archive and report what would be upserted,
        /// without touching SQLite.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9h — Claude Code auto-memory consolidator. Embeds every
    /// `*.md` under `~/.claude/projects/<slug>/memory/`, clusters
    /// near-duplicates within each `type`, and asks the merge agent
    /// to produce one denser entry per cluster. Default mode is
    /// dry-run; pass `--apply` to mutate the filesystem (archive
    /// originals + write `consolidated_<hash>.md` + regenerate
    /// `MEMORY.md`).
    MemoryConsolidate {
        /// Project slug under `~/.claude/projects/`. Defaults to the
        /// slug derived from the current working directory.
        #[arg(long)]
        project: Option<String>,
        /// Cosine cutoff for greedy clustering.
        #[arg(long, default_value_t = 0.78)]
        threshold: f32,
        /// Source-to-merged cosine floor for the drift guard.
        #[arg(long, default_value_t = 0.6)]
        drift_floor: f32,
        /// Maximum clusters to merge in one run. Omit for unlimited.
        #[arg(long)]
        max_clusters: Option<usize>,
        /// Mutate the filesystem. Without this flag the run is a
        /// preview only.
        #[arg(long)]
        apply: bool,
        /// Override the memory directory (debugging / fixtures).
        #[arg(long)]
        memory_dir: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9g — SQLite metadata reaper. Aggregates aged
    /// `bootstrap_jobs` (status='success') / `sessions` /
    /// `classifier_spend` rows into rollup tables, deletes the
    /// sources, then rotates `~/.cortex/hook-invocations.log` and
    /// `~/.cortex/hook-errors.log` to dated `.gz` siblings when they
    /// exceed the size or age threshold. Failed bootstrap rows stay
    /// raw. Re-running with no aged rows is a no-op.
    MetadataReap {
        /// Override "now" so the 30 / 365-day boundaries are
        /// deterministic for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the candidate counters without mutating SQLite or
        /// rotating logs.
        #[arg(long)]
        dry_run: bool,
        /// Restrict the SQL rollup to one target.
        #[arg(long, value_enum, default_value_t = MetadataReapTargetArg::All)]
        target: MetadataReapTargetArg,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Directory containing `hook-invocations.log` and
        /// `hook-errors.log`. Defaults to `<home>/.cortex`.
        #[arg(long)]
        log_dir: Option<String>,
        /// Skip the log rotator. Useful when the operator runs the
        /// reaper from inside a container that mounts the host's
        /// `~/.cortex` read-only.
        #[arg(long)]
        skip_logs: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9b — archive rollup compactor. Merges hourly Parquet
    /// files older than 90 d into one daily file per `day=`, daily
    /// files older than 365 d into one monthly file per `month=`,
    /// and drops monthly files older than 3 y unless `kind ∈
    /// {decision, analysis, law_violation}` or
    /// `redactions[].pii_risk = "low"`. Atomic + crash-safe.
    Rollup {
        /// Override "now" so the 90 / 365 / 1095-day boundaries
        /// are deterministic.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the plan + per-partition counts without mutating
        /// any file.
        #[arg(long)]
        dry_run: bool,
        /// Limit the run to one granularity. `all` runs the full
        /// pipeline (hourly→daily, daily→monthly, three-year drop)
        /// in order.
        #[arg(long, value_enum, default_value_t = RollupGranularityArg::All)]
        granularity: RollupGranularityArg,
        /// Override `CORTEX_ARCHIVE_ROOT` for one-shot CI runs.
        #[arg(long)]
        archive_root: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9a — Vectorizer tier-transition sweep. Re-encodes
    /// records FP32 → PQ at 30 d and PQ → Binary at 365 d per spec
    /// 02 §quantization. Idempotent + concurrency-safe via the
    /// `retention_sweeps` SQLite table; emits one bookkeeping row
    /// per invocation.
    ///
    /// `--dry-run` runs the plan against an empty in-memory store
    /// so the operator can see the canonical plan layout without
    /// touching the live Vectorizer.
    RetentionSweep {
        /// Override "now" so the 30 d / 365 d boundaries are
        /// deterministic for tests + scheduled CI runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the plan + transitions without mutating the
        /// destination collections.
        #[arg(long)]
        dry_run: bool,
        /// Maximum records per Vectorizer batch list call.
        #[arg(long, default_value_t = 256)]
        batch_size: u32,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase8f — fire a synthetic end-to-end canary frame through
    /// the live IPC pipe and assert it lands in the archive within
    /// the deadline. Detects regressions in the adapter → ingestion
    /// → archive path that latent unit tests miss.
    Canary {
        /// Hook flavour to fire (default `PostToolUse`).
        #[arg(long, default_value = "PostToolUse")]
        hook: String,
        /// Override the IPC binding (named pipe / unix socket path).
        #[arg(long)]
        ipc: Option<String>,
        /// Override `cortex-api` URL (defaults to `CORTEX_API_URL` /
        /// `http://127.0.0.1:17000`).
        #[arg(long)]
        api_url: Option<String>,
        /// Deadline before the canary is declared a `Timeout`.
        /// Default 10 s.
        #[arg(long, default_value_t = 10)]
        deadline_secs: u64,
        /// Emit JSON instead of the plain-text outcome line.
        #[arg(long)]
        json: bool,
    },
    /// Phase11o §2.5 — consolidation pruner. Walks the
    /// `cortex_consolidations` Meili index, demotes vectors between
    /// `cortex.consolidation.fp32` → `cortex.consolidation.pq` →
    /// `cortex.cold.binary` per the 0-7d / 7-90d / 90-365d schedule,
    /// and hard-purges the >365 d tail (vector + meili rows). The
    /// retention daemon fires this nightly at 03:00 via the
    /// `retention.consolidation_prune` cron row.
    ConsolidationPrune {
        /// Override "now" so the tier boundaries are deterministic
        /// for tests + scheduled CI runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the plan + per-tier counts without mutating any
        /// backend.
        #[arg(long)]
        dry_run: bool,
        /// Vectorizer base URL. Defaults to
        /// `CORTEX_EMBEDDER_VECTORIZER_URL` then `http://127.0.0.1:17001`.
        #[arg(long)]
        vectorizer_url: Option<String>,
        /// Meili base URL. Defaults to `CORTEX_FULLTEXT_MEILI_URL`
        /// then `http://127.0.0.1:7700`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meili master key. Defaults to `CORTEX_FULLTEXT_MEILI_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Cap on consolidations pulled per Meili page. Default 1000.
        #[arg(long, default_value_t = 1000)]
        page_size: u32,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase 12a §4 — replay envelopes that the consolidator
    /// persisted to the JSONL fallback because cortex-ingestion was
    /// unreachable when they were produced. POSTs each line to the
    /// resolved ingestion URL and reports per-line outcomes.
    ConsolidationsReplay {
        /// JSONL file produced by `publish_consolidation`'s fallback
        /// path. Defaults to `${CORTEX_CONSOLIDATIONS_FALLBACK_FILE}`,
        /// then `${CORTEX_HOME}/consolidations.jsonl`, then
        /// `<HOME|USERPROFILE>/.cortex/consolidations.jsonl`.
        #[arg(long)]
        from: Option<PathBuf>,
        /// cortex-ingestion base URL. Defaults to
        /// `CORTEX_INGESTION_URL`, then `http://127.0.0.1:17010`.
        #[arg(long)]
        ingest_url: Option<String>,
        /// Print the planned replays without sending any HTTP request.
        #[arg(long)]
        dry_run: bool,
        /// Maximum lines to replay in this run. Default: every line.
        #[arg(long)]
        limit: Option<usize>,
        /// Emit a single JSON object on stdout instead of the plain
        /// summary so cron jobs can parse the outcome.
        #[arg(long)]
        json: bool,
    },
    /// Phase8e — list active silent-drop alerts. Walks
    /// `~/.cortex/alerts/*.json` produced by the cortex-api
    /// silent-drop watcher and renders one row per pair. Exit `0`
    /// when no Critical alerts are active, `2` when any are.
    DoctorAlerts {
        /// Override the alerts directory (defaults to
        /// `~/.cortex/alerts`).
        #[arg(long)]
        state_dir: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase11e §2.3 — collection / index coverage doctor. Hits
    /// `<api_url>/v1/health/coverage` and renders the per-backend
    /// expected/present/missing counts plus the first ten missing
    /// names. Exit codes mirror the audit's `severity` field:
    /// `0` ok (every expected name present), `1` warn (at least
    /// one missing), `2` critical (nothing present at all).
    DoctorCoverage {
        /// `cortex-api` base URL. Defaults to `$CORTEX_API_URL`,
        /// then `http://127.0.0.1:17000`.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase15b §4.1 — graph-edge coverage doctor. Queries Nexus
    /// for `MATCH ()-[r]->() WHERE type(r) IN [...] RETURN type(r),
    /// count(r)` across every edge kind the phase15b projection
    /// pipeline registers, then renders a per-kind count + share
    /// table. Threshold per §4.2: every kind MUST have ≥1% of
    /// total edges — falls below trip a WARN. Exit codes: `0` all
    /// kinds present + above floor, `1` any kind missing OR below
    /// floor, `2` Nexus unreachable.
    DoctorGraphCoverage {
        /// Override the Nexus URL. Defaults to
        /// `$CORTEX_GRAPH_NEXUS_URL` then `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Per-kind minimum share of total edges (0.0..=1.0).
        /// Default 0.01 (1%) per §4.2.
        #[arg(long, default_value_t = 0.01)]
        floor: f64,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase12d §3 — Meili index settings drift checker. For every
    /// index in `cortex_storage::fulltext::INDEXES`, fetches the live
    /// `<meili>/indexes/{name}/settings` and compares the declared
    /// `searchableAttributes` / `filterableAttributes` /
    /// `sortableAttributes` against the live values. Surfaces drift,
    /// missing indexes, and unreachable backends. Exit codes: `0` all
    /// match, `2` any drift OR any HTTP failure. Pairs with the
    /// bootstrap PATCH (`bin/cortex-init.sh` § "seed: Meilisearch
    /// indexes") which is the reconcile path — running this doctor
    /// after a deploy confirms reconcile succeeded.
    /// ADR-016 §4 — workspace audit of `std :: env :: var ("CORTEX_*")`
    /// call sites outside `cortex-config`. Exits `0` when zero
    /// ad-hoc reads remain (every knob bound through the typed
    /// `cortex_config::Config`), `2` otherwise. The CI grep gate
    /// shares the same call so a regression that adds a new
    /// `env :: var ("CORTEX_*")` reference fails both surfaces in
    /// lockstep.
    DoctorConfigAudit {
        /// Workspace `crates/` root (defaults to `crates/`
        /// resolved against the current working directory).
        #[arg(long)]
        crates_root: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// ADR-012 §4.1 — `event_identity` coverage probe. Walks the
    /// SQLite `event_identity` table once and reports the per-
    /// backend coverage gap: for every row, which of `nexus_id` /
    /// `vec_id` / `meili_id` / `archive_partition` is NULL. A
    /// projection that forgot to stamp its backend (or that
    /// failed mid-batch and never retried) surfaces here as a
    /// non-zero `<backend>_missing` counter. Budget: indexed scan
    /// of 100k rows finishes in under 10 s on the running stack
    /// (vs. minutes for the legacy per-backend doctor fan-out).
    /// Exit code `2` when ANY coverage gap is found so cron
    /// wrappers escalate visibly.
    DoctorIdentityCoverage {
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` / `<CORTEX_HOME>/metadata.sqlite`
        /// / `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Cap the number of orphan event ids surfaced in the
        /// report. Defaults to 50 so the operator can paginate
        /// through large gaps with successive runs scoped to a
        /// `--since`/`--repo` slice (those filters land in §4.2
        /// alongside the budget bench).
        #[arg(long, default_value_t = 50)]
        sample_limit: usize,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase29 (mcp-surface §2) — compare spec 20's Registry table
    /// against `ToolRegistry::default_set()`; exits 1 on one-tool
    /// drift, 2 (critical) at >=2 per spec 20's blocking threshold.
    /// Phase30 (live-e2e-smoke §1) — long-lived-stack doctor + e2e
    /// smoke: backend/worker/adapter health, then exercises every
    /// READ MCP tool in-process against the live cortex-api (the
    /// "registered but never exercised" gate). Scheduled by the
    /// `health.doctor_smoke` cron seed; non-zero exit surfaces in
    /// the cron row failure streak.
    DoctorSmoke {
        /// Override the cortex-api base URL. Defaults to
        /// `CORTEX_API_URL` then `http://127.0.0.1:17000`.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    DoctorRegistrySync {
        /// Override the spec path. Defaults to
        /// `docs/specs/20-mcp-tool-surface.md`.
        #[arg(long)]
        spec: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    DoctorMeiliIndexes {
        /// Meili base URL. Defaults to `$MEILI_URL`, then
        /// `http://127.0.0.1:17004`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meili master key. Defaults to `$MEILI_MASTER_KEY`. Empty
        /// means the doctor sends no `Authorization` header (works
        /// for unauthenticated dev stacks).
        #[arg(long)]
        master_key: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase8d — config-coherence audit. Read-only static analysis
    /// of every config surface (`.env`, `~/.cortex/adapter.toml`,
    /// `packages/cortex-claude-plugin/.mcp.json`,
    /// `packages/cortex-claude-plugin/hooks/hooks.json`)
    /// plus cross-checks (e.g. adapter.endpoint must match
    /// CORTEX_INGESTION_URL). Exit codes: `0` all ok, `1` any warn,
    /// `2` any critical.
    DoctorConfig {
        /// Workspace root (defaults to current dir). The audit
        /// expects `.env` and `packages/cortex-claude-plugin/` under this path.
        #[arg(long)]
        workspace: Option<String>,
        /// Override `~/.cortex/adapter.toml` location (CI / fixtures).
        #[arg(long)]
        adapter_toml: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase14g §2.3 — pre-thinking intent routing stats. Fetches
    /// `/v1/health/pre-thinking` and reports per-(from, to) intent
    /// mismatch counts + per-path rewriter cascade counts so the
    /// operator can tune `DEFAULT_RULES` + spot Sonnet outages.
    IntentStats {
        /// cortex-api base URL. Defaults to `$CORTEX_API_URL`
        /// then `http://127.0.0.1:17000`.
        #[arg(long)]
        api_url: Option<String>,
        /// Window descriptor (`7d`, `24h`, etc.). Today the
        /// endpoint returns lifetime counters; the flag is
        /// accepted for forward-compat and stamped on the report
        /// header so the operator records what window they
        /// queried.
        #[arg(long, default_value = "lifetime")]
        since: String,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase14h §3.3 — Synap-worker cross-cut doctor. Probes each
    /// worker's `/healthz` endpoint and prints per-worker lag,
    /// `consume_errors_consecutive`, last-consume freshness, and
    /// dead-letter counters so the operator can spot a stuck
    /// pipeline without four separate doctor calls.
    DoctorSynapWorkers {
        /// Embedder `/healthz` URL. Defaults to
        /// `$CORTEX_EMBEDDER_HEALTH_URL` then
        /// `http://127.0.0.1:17100/healthz`.
        #[arg(long)]
        embedder_url: Option<String>,
        /// Fulltext `/healthz` URL. Defaults to
        /// `$CORTEX_FULLTEXT_HEALTH_URL` then
        /// `http://127.0.0.1:17110/healthz`.
        #[arg(long)]
        fulltext_url: Option<String>,
        /// Graph `/healthz` URL. Defaults to
        /// `$CORTEX_GRAPH_HEALTH_URL` then
        /// `http://127.0.0.1:17120/healthz`.
        #[arg(long)]
        graph_url: Option<String>,
        /// Classifier `/healthz` URL. Defaults to
        /// `$CORTEX_CLASSIFIER_HEALTH_URL` then
        /// `http://127.0.0.1:17130/healthz`.
        #[arg(long)]
        classifier_url: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Cross-backend consistency checker. v1 (phase4d) covered the
    /// archive ↔ Meili axis; phase4h widens it to Vectorizer +
    /// Nexus. Each probe is opt-in via its own flag / env var so
    /// the doctor still runs against partial backends.
    DoctorConsistency {
        /// Archive root (defaults to `$CORTEX_ARCHIVE_ROOT` then
        /// `~/.cortex/archive`).
        #[arg(long)]
        archive_root: Option<String>,
        /// Meilisearch base URL (defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL`).
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master key (defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`).
        #[arg(long)]
        meili_key: Option<String>,
        /// Vectorizer base URL (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_URL`). Probe runs only
        /// when both URL and credentials are present.
        #[arg(long)]
        vectorizer: Option<String>,
        /// Vectorizer admin username (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_USER`).
        #[arg(long)]
        vectorizer_user: Option<String>,
        /// Vectorizer admin password (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_PASSWORD`).
        #[arg(long)]
        vectorizer_password: Option<String>,
        /// Nexus base URL (defaults to `$CORTEX_NEXUS_URL`). Probe
        /// runs only when this is set; the rest of the
        /// `cortex-graph` env vars (auth, transport) are picked up
        /// via `GraphConfig::from_env`.
        #[arg(long)]
        nexus: Option<String>,
        /// Repeatable query-overlap probe (phase4i). Each value
        /// runs against the three lanes; the report renders one
        /// per-query block with pairwise Jaccards. The run fails
        /// when any pair falls below `--min-overlap-jaccard`.
        #[arg(long = "query")]
        queries: Vec<String>,
        /// Top-K cap for query-overlap probes (default 10).
        #[arg(long, default_value_t = 10)]
        probe_k: usize,
        /// Pairwise Jaccard threshold below which the probe fails
        /// (default 0.2).
        #[arg(long, default_value_t = 0.2)]
        min_overlap_jaccard: f64,
        /// Emit JSON instead of the markdown table.
        #[arg(long)]
        json: bool,
    },
    /// Phase10i — session-tool backfill. Walks the metadata
    /// `sessions` table looking for rows whose `tool` is NULL or
    /// empty (the audit caught 574 sessions in this state) and
    /// stamps a default tool string. The pre-phase10i upsert
    /// overwrote existing values when a hook passed an empty
    /// tool; the new upsert preserves the existing value but the
    /// rows that already lost their tool need this one-shot
    /// migration.
    SessionsBackfillTool {
        /// Tool string to stamp on every NULL row. Defaults to
        /// `claude-code` because that's the only adapter the
        /// pre-phase10i daemons ran.
        #[arg(long, default_value = "claude-code")]
        tool: String,
        /// Read-only — list candidate rows without mutating
        /// SQLite. Default mode.
        #[arg(long)]
        dry_run: bool,
        /// Apply the stamp. The mode is exclusive with
        /// `--dry-run`.
        #[arg(long)]
        apply: bool,
        /// Maximum rows to scan / stamp per invocation.
        #[arg(long, default_value_t = 10_000)]
        limit: usize,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then
        /// `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase10d — repo-casing canonicalizer. Migrates the metadata
    /// SQLite tables (`sessions.repo`, `bootstrap_jobs.repo_path`)
    /// from mixed-case to canonical lowercase so downstream
    /// queries that scope to `repo: "Cortex"` and
    /// `repo: "cortex"` resolve to the same rows. Live-backend
    /// rewrites (Vectorizer payload `repo`, Meili documents,
    /// Nexus properties) are reserved for the follow-up;
    /// today's CLI normalises the SQLite metadata only and
    /// reports the per-store rewrite candidate set.
    RepoCanonicalize {
        /// Restrict the migration to a single repo identifier
        /// (matches the pre-migration value, case-sensitive).
        #[arg(long)]
        repo: Option<String>,
        /// Read-only — print the rewrite plan without mutating
        /// SQLite. Default mode.
        #[arg(long)]
        dry_run: bool,
        /// Apply the SQLite rewrite. Live-backend rewrites are
        /// listed but skipped with a documentation pointer.
        #[arg(long)]
        apply: bool,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then
        /// `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase10c — bootstrap dedup ledger inspector. Walks the
    /// `bootstrap_seen` table looking for duplicate-by-content
    /// groups within a repo and prints a summary so operators can
    /// decide whether to clean up the live lane.
    ///
    /// `--dry-run` (default) is read-only. `--apply` is reserved
    /// for the future live-backend cleanup path; today it returns
    /// an actionable error pointing at the dry-run output so
    /// operators can re-walk affected files manually under the
    /// new walker.
    BootstrapDedup {
        /// Restrict the scan to a single repo identifier (matches
        /// `bootstrap_seen.repo`). Omit to scan every repo in the
        /// ledger.
        #[arg(long)]
        repo: Option<String>,
        /// Report duplicate groups without mutating any backend.
        #[arg(long)]
        dry_run: bool,
        /// Reserved for the live-backend cleanup path. Today this
        /// flag exits with a documentation pointer; the dry-run
        /// output is still produced so operators see the candidate
        /// set.
        #[arg(long)]
        apply: bool,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then
        /// `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// phase15a §3 — report multi-repo bootstrap progress from the
    /// checkpoint file: per-repo events emitted, last position, last-emit
    /// age, recent emit rate, and ETA. Exit 0 when every not-`done` repo
    /// emitted within the last 5 min; exit 2 when any is stalled.
    BootstrapStatus {
        /// Bootstrap checkpoint file (matches `cortex-bootstrap
        /// --checkpoint`). Defaults to `.cortex-bootstrap.state.json`.
        #[arg(long, default_value = ".cortex-bootstrap.state.json")]
        checkpoint: PathBuf,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase11l §7 — graph-side admin operations. Today the only
    /// subcommand is `drop`, used during the Nexus external-id
    /// migration to wipe the Cortex graph DB so a fresh
    /// `cortex-bootstrap --graph-static` pass can rebuild it under
    /// the new `_id` keying.
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    /// Phase18 §4.3 — POST a query to `cortex-api`'s `/v1/query`
    /// with the new optional `--as-of` / `--branch` / `--projects`
    /// fields. The orchestrator's phase18 §3.3 wedge applies the
    /// temporal classifier automatically.
    Query {
        /// Free-text prompt.
        query: String,
        /// Render the response as it would be believed at this
        /// point in valid time. Accepts RFC-3339 or `YYYY-MM-DD`.
        #[arg(long, value_name = "RFC3339|DATE")]
        as_of: Option<String>,
        /// Branch to retrieve from (defaults to `<project>:main`
        /// per ADR-019). Composite id form: `<project>:<branch>`.
        #[arg(long)]
        branch: Option<String>,
        /// Cross-project axis activation: extra projects whose
        /// facts the orchestrator unions into the candidate set
        /// (default-off per ADR-020).
        #[arg(long, value_name = "PROJECT", num_args = 0..)]
        projects: Vec<String>,
        /// Override the cortex-api base URL.
        #[arg(long)]
        api_url: Option<String>,
        /// Override the intent label.
        #[arg(long)]
        intent: Option<String>,
        /// Max snippets surfaced.
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Phase18 §4.3 — print every `TimelineEvent` row tagged with
    /// the entity, ordered by `valid_from_unix DESC`. Optional
    /// `--as-of` restricts to events recorded ≤ that time.
    History {
        /// Entity id to walk (ULID / ADR id / etc.).
        entity_id: String,
        /// Optional valid-time cap.
        #[arg(long, value_name = "RFC3339|DATE")]
        as_of: Option<String>,
        /// Override the Nexus base URL.
        #[arg(long)]
        nexus: Option<String>,
        /// Max rows returned.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Phase18 §4.3 — walk the `SUPERSEDES` chain off the entity in
    /// both directions. The classifier reads the same edges to
    /// derive the SUPERSEDED state (ADR-023 §1.6).
    Supersession {
        /// Entity id at the centre of the lineage.
        entity_id: String,
        /// Override the Nexus base URL.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Phase18 §4.2 — branch operator commands. Manages `Branch`
    /// nodes (spec 32): list / show / create / merge / abandon.
    /// All five subcommands talk to Nexus directly via the
    /// shared `LiveNexusClient`.
    Branch {
        #[command(subcommand)]
        command: BranchCommand,
    },
    /// Phase18 §4.1 — render the timeline of `TimelineEvent` nodes
    /// for a project. Reads from Nexus, applies the optional
    /// branch / kind / valid-time / as-of filters, and prints
    /// either a plain-text table or JSON. The branch filter
    /// defaults to the project's `main` per ADR-019.
    Timeline {
        /// Project the timeline belongs to (matches
        /// `TimelineEvent.project_id`).
        project: String,
        /// Render the timeline as it was believed at this point
        /// in valid time. Accepts RFC-3339 or `YYYY-MM-DD`
        /// (ADR-018). Defaults to "now".
        #[arg(long, value_name = "RFC3339|DATE")]
        as_of: Option<String>,
        /// Branch name (matches `TimelineEvent.branch_id` under
        /// `<project>:<branch>`). Defaults to `main`.
        #[arg(long)]
        branch: Option<String>,
        /// Restrict to one timeline kind (see
        /// `TimelineKind::as_str` for the discriminator set).
        #[arg(long)]
        kind: Option<String>,
        /// Lower bound on `valid_from_unix`. Same date forms as
        /// `--as-of`.
        #[arg(long, value_name = "RFC3339|DATE")]
        from: Option<String>,
        /// Upper bound on `valid_from_unix`.
        #[arg(long, value_name = "RFC3339|DATE")]
        to: Option<String>,
        /// Max rows returned.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Override the Nexus base URL. Defaults to
        /// `$CORTEX_NEXUS_URL` then `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase11p §4.3 — dedupe `law.imported` documents on every
    /// `cortex-{slug}-governance` Meili index. Groups documents by
    /// `(law_id, content_hash)`; for each duplicate group, keeps the
    /// oldest by `ts ASC` and drops the rest. Default is dry-run;
    /// pass `--apply` to actually `DELETE` the documents.
    ///
    /// Why grouping on `content_hash` rather than `event_id`: the
    /// pre-phase10c bootstrap history landed multiple envelopes for
    /// the same law text under different ULIDs; the ledger now
    /// suppresses re-emits but cannot retroactively dedupe what's
    /// already at rest. This op cleans the storage layer.
    DedupeLaws {
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then `http://127.0.0.1:17004`.
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master / admin API key.
        #[arg(long)]
        meili_key: Option<String>,
        /// Restrict the dedupe to one per-repo governance index
        /// (uid form: `cortex-{slug}-governance`). Omit to scan
        /// every governance index plus the global `cortex_laws`.
        #[arg(long)]
        index: Option<String>,
        /// Apply the deletes. Without this flag the command runs
        /// dry — only prints the per-group keep / drop plan.
        #[arg(long)]
        apply: bool,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Phase11p §1.2 — drop empty Meili indexes. Default is dry-run
    /// (lists candidates only); pass `--apply` to actually delete.
    /// Combines two predicates:
    ///
    /// 1. **Non-canonical empties** — names that don't match
    ///    `cortex-{slug}-{family}` (legacy migration leftovers).
    /// 2. **Canonical empties** — `cortex-{slug}-{family}` indexes
    ///    that exist but hold zero documents (renamed / abandoned
    ///    repos that survived the per-project bootstrap).
    ///
    /// The dry-run is read-only and safe to run on production. The
    /// `--apply` path issues `DELETE /indexes/{uid}` per candidate;
    /// the next bootstrap re-creates whichever names a live repo
    /// still needs.
    SweepEmpty {
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then `http://127.0.0.1:17004`.
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY` then
        /// `$MEILI_MASTER_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Apply the deletion. Without this flag the command runs
        /// dry — only prints the candidate list and exits.
        #[arg(long)]
        apply: bool,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Phase18 §5.1 — scan manifests (Cargo.toml / package.json) and
    /// ADR decision files for cross-project version references and
    /// upsert `CROSS_PROJECT_REF` edges into Nexus. Idempotent:
    /// each edge is a `MERGE` so re-running is safe. `--dry-run`
    /// prints the edge list without writing anything.
    BackfillCrossProject {
        /// Repo root holding the manifests + .rulebook/decisions to scan. Defaults to ".".
        #[arg(long, default_value = ".")]
        root: String,
        /// The `project_id` these edges originate from (the `from` side). Defaults to "cortex".
        #[arg(long, default_value = "cortex")]
        project: String,
        /// Override the Nexus base URL. Defaults to $CORTEX_NEXUS_URL then http://127.0.0.1:17002.
        #[arg(long)]
        nexus: Option<String>,
        /// Scan + report the edges but write nothing to Nexus.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase18 §7.3 — export a digest of the temporal/branch/cross-project
    /// signals from cortex-api's /metrics endpoint.
    TemporalDigest {
        /// cortex-api base URL. Defaults to $CORTEX_API_URL then http://127.0.0.1:17000.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the Markdown digest.
        #[arg(long)]
        json: bool,
    },
    /// Derive synthetic `TimelineEvent` nodes from the real progress-
    /// bearing nodes already in Nexus (`Decision`, `Analysis`,
    /// `Memory`, `LawViolation`, `Learning`, `Knowledge`) across ALL
    /// projects.  Each source node with a non-null `valid_from` becomes
    /// one idempotent `TimelineEvent` whose `id` is
    /// `tl:<Label>:<source-id>` so the Bitemporal Timeline view shows
    /// real cross-project history without requiring every writer to
    /// emit `TimelineEvent` nodes directly.  Re-running is safe
    /// (MERGE-based).
    TimelineBackfill {
        /// Override the Nexus base URL. Defaults to
        /// `$CORTEX_NEXUS_URL` then `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Scan + report candidate events but write nothing to Nexus.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase15e §4 — redaction coverage audit. Samples 100 recent
    /// envelopes from `cortex.events.raw` (or a custom stream) and
    /// runs `PATTERN_CATALOG_V1` against each payload JSON. Any match
    /// is reported as an `unredacted-candidate` with field path, byte
    /// offset, and a truncated preview (first 16 chars plus SHA-256
    /// hash) so the operator can identify the leak without the full
    /// secret appearing in logs. Exit `0` when zero candidates are
    /// found; exit `2` when any match is found.
    DoctorRedactionCoverage {
        /// Synap base URL. Defaults to `$CORTEX_SYNAP_URL`,
        /// `$SYNAP_URL`, then `http://127.0.0.1:17003`.
        #[arg(long)]
        synap_url: Option<String>,
        /// Override the stream to sample. Defaults to
        /// `cortex.events.raw`.
        #[arg(long)]
        stream: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// phase0_decision-fulltext-title-body-mismapped — scan
    /// `cortex_decisions` for malformed orphan docs whose
    /// `title == id` (the `01KQNYF4J*` early-buggy-emit signature).
    /// Exit `0` when no malformed docs are found; `2` when any
    /// are present or the index is unreachable. Pairs with
    /// `decisions-reindex` which fixes the issue.
    DoctorDecisions {
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then
        /// `http://127.0.0.1:7700`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        master_key: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Scan ANY content-addressable index for legacy (non-`bootstrap-`-
    /// keyed) docs + the malformed `title == id` subset. Exit `0` when
    /// every doc is `bootstrap-`-keyed; `2` when any legacy doc is found
    /// or the index is unreachable. Repair with `meili-rekey` (in-place)
    /// or `decisions-reindex` (file-backed kinds).
    DoctorContentAddressable {
        /// Target index (e.g. `cortex-cortex-knowledge`).
        #[arg(long)]
        index: String,
        /// Meilisearch base URL. Defaults to `$CORTEX_FULLTEXT_MEILI_URL`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        master_key: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// phase0_decision-fulltext-title-body-mismapped — re-emit all
    /// `.rulebook/decisions/*.md` into `cortex_decisions` using the
    /// stable content-addressable doc_id
    /// (`bootstrap:<repo>:<path>:<hash>`), then prune any doc whose
    /// `title == id` (the malformed-orphan signature). Idempotent —
    /// Meilisearch upserts on the stable primary key so re-running is
    /// safe. Reads `CORTEX_FULLTEXT_MEILI_URL` /
    /// `CORTEX_FULLTEXT_MEILI_API_KEY`. Use `--dry-run` to report
    /// what would change without writing.
    DecisionsReindex {
        /// Directory containing the `*.md` decision files. Defaults
        /// to `.rulebook/decisions` relative to cwd.
        #[arg(long)]
        decisions_dir: Option<String>,
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then
        /// `http://127.0.0.1:7700`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Target decisions index. Defaults to the global
        /// `cortex_decisions`; pass `cortex-<repo>-decisions` to also
        /// repair the per-repo index the `decision_lookup` strategy
        /// fans out to.
        #[arg(long)]
        index: Option<String>,
        /// Report what would change without writing to Meilisearch.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// phase0_laws-index-routing-and-malformed-docs — re-emit all
    /// `.claude/rules/*.md` law definitions into the global
    /// `cortex_laws` index using stable content-addressable doc ids
    /// (`bootstrap-<hash>`), then prune any doc whose `title == id`
    /// or that is not keyed by the `bootstrap-` scheme. Idempotent —
    /// Meilisearch upserts on the stable primary key so re-running is
    /// safe. Reads `CORTEX_FULLTEXT_MEILI_URL` /
    /// `CORTEX_FULLTEXT_MEILI_API_KEY`. Use `--dry-run` to report
    /// what would change without writing.
    LawsReindex {
        /// Directory containing the `*.md` rule files. Defaults to
        /// `.claude/rules` relative to cwd.
        #[arg(long)]
        rules_dir: Option<String>,
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then
        /// `http://127.0.0.1:7700`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Target laws index. Defaults to the global `cortex_laws`.
        #[arg(long)]
        index: Option<String>,
        /// Report what would change without writing to Meilisearch.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Repair malformed `cortex_laws` docs IN PLACE from their own
    /// embedded payload (the malformed `body` is the stringified original
    /// law payload). Recovers law_id/title/body, rebuilds via the
    /// production builder, and re-keys to the stable `bootstrap-` id —
    /// works across ALL law sources (`.claude/rules` + `docs/specs` +
    /// AGENTS) with no source re-walk and no data-loss risk.
    LawsRepair {
        /// Meilisearch base URL. Defaults to `$CORTEX_FULLTEXT_MEILI_URL`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Target laws index. Defaults to the global `cortex_laws`.
        #[arg(long)]
        index: Option<String>,
        /// Report what would change without writing to Meilisearch.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Re-key legacy random-ULID content-addressable docs to the stable
    /// Meili-safe `bootstrap-<hash>` primary key IN PLACE (no source
    /// re-emit, so non-file-backed entries like live-captured knowledge /
    /// learnings are preserved). Use for `cortex-<repo>-{knowledge,
    /// learnings}` and other content-addressable indexes.
    MeiliRekey {
        /// Target index (e.g. `cortex-cortex-knowledge`).
        #[arg(long)]
        index: String,
        /// Meilisearch base URL. Defaults to `$CORTEX_FULLTEXT_MEILI_URL`.
        #[arg(long)]
        meili_url: Option<String>,
        /// Meilisearch master / admin API key. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`.
        #[arg(long)]
        meili_key: Option<String>,
        /// Report what would change without writing to Meilisearch.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase21 §6.1 — access-control admin commands.
    /// Manage role bindings, principal grants, classification
    /// rules, and caller identity resolution.
    Acl {
        #[command(subcommand)]
        command: AclCommand,
    },
}

/// Phase18 §4.2 — `cortex-ops branch` subcommand surface.
#[derive(Subcommand)]
enum BranchCommand {
    /// List every branch for a project.
    List {
        /// Project slug.
        project: String,
        /// Nexus base URL override.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// Show one branch's full payload.
    Show {
        /// Project slug.
        project: String,
        /// Branch name (`main` reserved per ADR-019).
        branch: String,
        /// Nexus base URL override.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON instead of the indented payload.
        #[arg(long)]
        json: bool,
    },
    /// Fork a new branch off `<from>` at an optional `--valid-time`
    /// anchor (RFC-3339 or `YYYY-MM-DD`).
    Create {
        /// Project slug.
        project: String,
        /// New branch name (ADR-019 regex).
        #[arg(long)]
        name: String,
        /// Parent branch name to fork from.
        #[arg(long)]
        from: String,
        /// Fork anchor in valid-time. Optional; ADR-018 second-precision.
        #[arg(long, value_name = "RFC3339|DATE")]
        valid_time: Option<String>,
        /// Nexus base URL override.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fold a branch into its parent with one of the ADR-021
    /// merge strategies (`accept` / `partial` / `discard`).
    Merge {
        /// Project slug.
        project: String,
        /// Branch to merge.
        branch: String,
        /// Merge strategy.
        #[arg(long, value_name = "accept|partial|discard")]
        strategy: String,
        /// Nexus base URL override.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Abandon a branch with a free-text reason (ADR-022). Updates
    /// `status` + `abandonment_reason`; does not write a
    /// `MERGED_INTO` edge.
    Abandon {
        /// Project slug.
        project: String,
        /// Branch to abandon.
        branch: String,
        /// Free-text reason (required).
        #[arg(long)]
        reason: String,
        /// Nexus base URL override.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Phase11l §7.1 — `cortex-ops graph` subcommand surface.
#[derive(Subcommand)]
enum GraphCommand {
    /// Wipe every Cortex-owned label from the Nexus graph DB.
    /// Refuses to run without `--confirm`. `--dry-run` prints the
    /// per-label count it would delete and exits without mutation.
    /// Idempotent — safe to re-run after a partial failure.
    Drop {
        /// Required acknowledgement. Without this flag the command
        /// prints a warning and exits non-zero.
        #[arg(long)]
        confirm: bool,
        /// Print the per-label delete plan + projected counts and
        /// exit without mutating Nexus.
        #[arg(long)]
        dry_run: bool,
        /// Override the Nexus URL. Defaults to
        /// `$CORTEX_GRAPH_NEXUS_URL` then `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase11s §2.4 — rewind the durable graph-worker offset to a
    /// known starting point so the next worker boot replays the
    /// envelopes after `--since`. Used by the §5 drainage runbook
    /// when a known event window was lost (e.g. a worker restart
    /// during an indexer rebuild).
    Replay {
        /// Synap stream offset to rewind TO. The worker will resume
        /// from `--since + 1` on next boot. Pass `0` to replay the
        /// whole stream from the beginning.
        #[arg(long)]
        since: u64,
        /// Consumer id partition the offset ledgers under. Default
        /// matches the graph-writer bin: `cortex-graph-0`. Override
        /// when running multiple replicas.
        #[arg(long, default_value = "cortex-graph-0")]
        consumer_id: String,
        /// Synap stream the consumer reads. Default
        /// `cortex.events.enriched` (the graph worker's input).
        #[arg(long, default_value = "cortex.events.enriched")]
        stream: String,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_GRAPH_METADATA_DB` then `$CORTEX_METADATA_DB`
        /// then `${CORTEX_HOME}/metadata.sqlite` then
        /// `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Print the planned rewind without writing the row.
        #[arg(long)]
        dry_run: bool,
    },
    /// phase15h — seed the graph consumer offset at the CURRENT Synap
    /// stream head, so the worker resumes from there (capturing new
    /// events forward) instead of re-consuming the whole stream from
    /// 0. Recovery lever for a graph-worker whose ephemeral offset was
    /// lost: re-projecting all history live trips nexus#12. Run this
    /// against the worker's (volume-mounted) metadata DB, then restart
    /// the worker. History stays available via `graph backfill`.
    SeekHead {
        /// Synap base URL. Defaults to `$CORTEX_GRAPH_SYNAP_URL` then
        /// `http://127.0.0.1:17003`.
        #[arg(long)]
        synap: Option<String>,
        /// Consumer id partition (must match the worker's). Default
        /// `cortex-graph-0`.
        #[arg(long, default_value = "cortex-graph-0")]
        consumer_id: String,
        /// Synap stream the consumer reads. Default
        /// `cortex.events.enriched`.
        #[arg(long, default_value = "cortex.events.enriched")]
        stream: String,
        /// SQLite metadata DB path (the worker's). Defaults to the same
        /// resolution chain as `graph replay`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Print the discovered head + planned write without mutating.
        #[arg(long)]
        dry_run: bool,
    },
    /// Phase15b §3.3 — replay the archive through the phase15b
    /// projection pipeline. Walks every envelope newer than
    /// `--since`, runs all 12 edge extractors, and prints a
    /// per-edge-kind count summary. Today the subcommand runs in
    /// dry-run mode only: payload-driven extractors
    /// (SUPERSEDES / CONTRADICTS / EMITTED_BY / ANSWERED_BY /
    /// CITES body-regex) produce useful counts; classifier-
    /// driven ones (CALLS / IMPORTS / DEFINES / RETURNS / ABOUT
    /// / MENTIONS_FILE / RELATES_TO) need a classifier replay
    /// that lands in a follow-up commit. Use the counts to seed
    /// the §4.1 `doctor graph-coverage` thresholds.
    Backfill {
        /// RFC-3339 lower bound on `Envelope.occurred_at`. Omit
        /// to walk the full archive.
        #[arg(long)]
        since: Option<String>,
        /// Override the archive root. Defaults to
        /// `cortex_config::IngestionConfig.archive_root` then
        /// `~/.cortex/archive`.
        #[arg(long)]
        archive_root: Option<String>,
        /// phase15c §1.1 — write the projected edges to the live
        /// graph via the real `GraphWriter` instead of the
        /// count-only dry-run. Only the payload-driven kinds
        /// (SUPERSEDES / CONTRADICTS / EMITTED_BY / ANSWERED_BY /
        /// CITES body-regex) land without a classifier replay.
        #[arg(long)]
        apply: bool,
        /// phase15c §1.1 — cap the number of envelopes projected
        /// (newest-walked order). Bounds the sustained edge-write
        /// load so `--apply` stays under the nexus#12 stall
        /// threshold. `0` (default) means no cap.
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Override the Nexus URL for `--apply`. Defaults to
        /// `cortex_config::NexusConfig.nexus_url` then
        /// `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase29 (graph-projection-unblock §4.1, unblocking phase27b
    /// §2.5) — snapshot the architecture subgraph
    /// (DEFINES / CALLS / IMPORTS / ABOUT edges), run Louvain+Leiden
    /// community detection over it, and write `community_id` /
    /// `community_level` / `is_god_node` back onto the member nodes
    /// (idempotent MATCH-policy NodeOps — re-running never creates
    /// nodes). Driven nightly by the `graph.community_detect` cron
    /// seed; safe to run by hand at any time.
    CommunitiesDetect {
        /// Override the Nexus URL. Defaults to
        /// `cortex_config::NexusConfig.nexus_url` then
        /// `http://127.0.0.1:17002`.
        #[arg(long)]
        nexus: Option<String>,
        /// Cap on the number of architecture edges snapshotted.
        #[arg(long, default_value_t = 100_000)]
        edge_limit: usize,
        /// Detect + report communities but write nothing back.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of plain-text summary.
        #[arg(long)]
        json: bool,
    },
}

/// Phase21 §6.1 — `cortex-ops acl role` subcommands.
#[derive(Subcommand)]
enum AclRoleCommand {
    /// Create or overwrite a role binding.
    Create {
        /// Role name (e.g. `finance`, `acl_admin`).
        name: String,
        /// Sensitivity level 0–3 (public / internal / confidential / restricted).
        #[arg(long)]
        clearance: u8,
        /// Comma-separated compartment names granted by this role.
        #[arg(long, value_delimiter = ',', default_value = "")]
        compartments: Vec<String>,
        /// Cortex API base URL (defaults to config `dashboard.api_url`).
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the plain-text confirmation.
        #[arg(long)]
        json: bool,
    },
    /// List every registered role binding.
    List {
        /// Cortex API base URL.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
}

/// Phase21 §6.1 — `cortex-ops acl` subcommands.
#[derive(Subcommand)]
enum AclCommand {
    /// Manage RBAC role bindings (create / list).
    Role {
        #[command(subcommand)]
        command: AclRoleCommand,
    },
    /// Grant a principal an explicit role, clearance level, or compartments.
    Grant {
        /// Principal identifier (API key id or subject claim).
        principal_id: String,
        /// Role name to assign.
        #[arg(long)]
        role: Option<String>,
        /// Override clearance level (0–3).
        #[arg(long)]
        clearance: Option<u8>,
        /// Comma-separated compartments to grant.
        #[arg(long, value_delimiter = ',')]
        compartments: Vec<String>,
        /// Cortex API base URL.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// List all active classification path rules (from cortex.toml).
    ClassifyRuleList {
        /// Cortex API base URL.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Resolve and print the caller's effective clearance/compartments.
    Whoami {
        /// Cortex API base URL.
        #[arg(long)]
        api_url: Option<String>,
        /// Emit JSON instead of the lattice table.
        #[arg(long)]
        json: bool,
    },
}

/// Phase9b — granularity selector for `cortex-ops rollup`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[allow(non_camel_case_types)]
enum RollupGranularityArg {
    /// Run all three granularities in order.
    All,
    HourlyToDaily,
    DailyToMonthly,
    ThreeYearDrop,
}

/// Phase9k — `cortex-ops schedule` subcommand surface.
#[derive(Subcommand)]
enum ScheduleCommand {
    /// Print the cron_jobs registry as a table (or JSON).
    List {
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
        /// SQLite metadata DB path. Defaults to `$CORTEX_METADATA_DB`
        /// then `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Show one job's full row including stdout/stderr tail.
    Show {
        name: String,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Enable a job (sets `enabled=1`).
    Enable {
        name: String,
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Disable a job (sets `enabled=0`).
    Disable {
        name: String,
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Replace the cron expression for a job. Validates the new
    /// expression and recomputes `next_run_at` immediately.
    Set {
        name: String,
        /// 5-field cron expression (`m h dom mon dow`).
        cron: String,
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Bypass the timer and run a job immediately.
    RunNow {
        name: String,
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Seed the eight default retention jobs (idempotent
    /// `INSERT OR IGNORE`).
    SeedDefaults {
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
    },
    /// Run a single scheduler tick: pick every due row and run it.
    /// Mostly useful for the daemon and integration tests; in
    /// production the long-running daemon process loops this.
    Tick {
        /// SQLite metadata DB path.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Override "now" for tests.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
    },
}

/// Phase9g — `cortex.toml [retention.metadata]` overrides for the
/// log rotator. The reaper's SQL retention horizons land directly on
/// [`cortex_workers::retention::metadata_reap::ReapPlan`]; the rotator knobs
/// don't have a home there, so the CLI threads them through this
/// small struct.
#[derive(Debug, Default, Clone, Copy)]
struct LogConfigOverrides {
    max_bytes: Option<u64>,
    max_age_days: Option<u32>,
    keep_rotations: Option<usize>,
}

/// Phase9g — target selector for `cortex-ops metadata-reap`. Mirrors
/// [`cortex_workers::retention::metadata_reap::ReapTarget`] but lives in the
/// CLI to keep the clap-derive surface here.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum MetadataReapTargetArg {
    /// Run every rollup target.
    All,
    /// Roll only `bootstrap_jobs` → `bootstrap_jobs_daily`.
    BootstrapJobs,
    /// Roll only `sessions` → `sessions_monthly`.
    Sessions,
    /// Roll only `classifier_spend` → `classifier_spend_monthly`.
    ClassifierSpend,
    /// Skip every rollup; only rotate the hook logs.
    Logs,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum PlanSlice {
    All,
    Collections,
    Cypher,
    Indexes,
    Streams,
    Kv,
}

fn main() -> ExitCode {
    // The Windows main-thread stack is 1 MiB. clap's parse/help
    // machinery for this large `Command` enum overflows it in debug
    // builds — even `cortex-ops --help` aborts before reaching any
    // subcommand. Run the real entrypoint on a worker thread with a
    // generous stack so every subcommand (and `--help`) works.
    match std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
    {
        Ok(handle) => handle.join().unwrap_or(ExitCode::FAILURE),
        Err(e) => {
            eprintln!("ERROR: spawn cortex-ops worker thread: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan { pretty, slice } => match plan::emit_plan(pretty, slice) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Doctor {
            vectorizer,
            nexus,
            synap,
            meili,
        } => doctor::doctor(vectorizer, nexus, synap, meili),
        Command::DoctorSmoke { api_url, json } => doctor_smoke::doctor_smoke(api_url, json),
        Command::DoctorRegistrySync { spec, json } => {
            doctor_registry_sync::doctor_registry_sync(spec, json)
        }
        Command::DoctorConfig {
            workspace,
            adapter_toml,
            json,
        } => doctor::doctor_config(workspace, adapter_toml, json),
        Command::DoctorSynapWorkers {
            embedder_url,
            fulltext_url,
            graph_url,
            classifier_url,
            json,
        } => doctor_synap_workers::run(embedder_url, fulltext_url, graph_url, classifier_url, json),
        Command::IntentStats {
            api_url,
            since,
            json,
        } => intent_stats::run(api_url, since, json),
        Command::DoctorAlerts { state_dir, json } => doctor::doctor_alerts(state_dir, json),
        Command::DoctorCoverage { api_url, json } => doctor::doctor_coverage(api_url, json),
        Command::DoctorGraphCoverage { nexus, floor, json } => {
            graph_cmd::doctor_graph_coverage(nexus, floor, json)
        }
        Command::DoctorMeiliIndexes {
            meili_url,
            master_key,
            json,
        } => doctor::doctor_meili_indexes(meili_url, master_key, json),
        Command::DoctorConfigAudit { crates_root, json } => {
            config_audit::doctor_config_audit(crates_root, json)
        }
        Command::DoctorIdentityCoverage {
            metadata_db,
            sample_limit,
            json,
        } => identity_coverage::doctor_identity_coverage(metadata_db, sample_limit, json),
        Command::RetentionArchivePurge {
            before,
            dry_run,
            repo,
            home,
        } => retention_archive_purge::run(before, dry_run, repo, home),
        Command::Watchdog {
            watcher_url,
            ingest_staleness_secs,
            sweep_staleness_secs,
            consolidation_staleness_secs,
            home,
            json,
        } => watchdog::watchdog(
            watcher_url,
            ingest_staleness_secs,
            sweep_staleness_secs,
            consolidation_staleness_secs,
            home,
            json,
        ),
        Command::MeiliAudit { json } => meili_audit::meili_audit(json),
        Command::MeiliReindex { archive_root, json } => {
            meili_audit::meili_reindex(archive_root, json)
        }
        Command::MeiliPrune {
            time_travel,
            dry_run,
            rebuild,
            batch_size,
            json,
        } => meili::meili_prune(time_travel, dry_run, rebuild, batch_size, json),
        Command::TurnDigest {
            time_travel,
            dry_run,
            rebuild,
            budget_cents,
            apply,
            purge_originals,
            max_records,
            page_size,
            json,
        } => digest::turn_digest(
            time_travel,
            dry_run,
            rebuild,
            budget_cents,
            apply,
            purge_originals,
            max_records,
            page_size,
            json,
        ),
        Command::ToolCallDigest {
            time_travel,
            dry_run,
            rebuild,
            budget_cents,
            apply,
            purge_originals,
            max_records,
            page_size,
            age_days,
            min_bucket_size,
            json,
        } => digest::tool_call_digest(
            time_travel,
            dry_run,
            rebuild,
            budget_cents,
            apply,
            purge_originals,
            max_records,
            page_size,
            age_days,
            min_bucket_size,
            json,
        ),
        Command::PiiEnforce {
            time_travel,
            dry_run,
            cohort,
            json,
        } => pii::pii_enforce(time_travel, dry_run, cohort, json),
        Command::CasVacuum {
            time_travel,
            dry_run,
            force,
            cas_db,
            json,
        } => cas::cas_vacuum(time_travel, dry_run, force, cas_db, json),
        Command::Rollup {
            time_travel,
            dry_run,
            granularity,
            archive_root,
            json,
        } => rollup::rollup(time_travel, dry_run, granularity, archive_root, json),
        Command::Schedule { command } => schedule_cmd::schedule(command),
        Command::MigrateClassification {
            archive_root,
            project,
            default_level,
            dry_run,
            json,
        } => migrate_classification::migrate_classification(
            archive_root,
            project,
            default_level,
            dry_run,
            json,
        ),
        Command::SessionsBackfill {
            metadata_db,
            archive_root,
            dry_run,
            json,
        } => sessions_backfill::sessions_backfill(metadata_db, archive_root, dry_run, json),
        Command::MemoryConsolidate {
            project,
            threshold,
            drift_floor,
            max_clusters,
            apply,
            memory_dir,
            json,
        } => memory_consolidate_cmd::memory_consolidate(
            project,
            threshold,
            drift_floor,
            max_clusters,
            apply,
            memory_dir,
            json,
        ),
        Command::MetadataReap {
            time_travel,
            dry_run,
            target,
            metadata_db,
            log_dir,
            skip_logs,
            json,
        } => metadata::metadata_reap(
            time_travel,
            dry_run,
            target,
            metadata_db,
            log_dir,
            skip_logs,
            json,
        ),
        Command::RetentionSweep {
            time_travel,
            dry_run,
            batch_size,
            metadata_db,
            json,
        } => retention::retention_sweep(time_travel, dry_run, batch_size, metadata_db, json),
        Command::ConsolidationPrune {
            time_travel,
            dry_run,
            vectorizer_url,
            meili_url,
            meili_key,
            page_size,
            json,
        } => consolidation::consolidation_prune(
            time_travel,
            dry_run,
            vectorizer_url,
            meili_url,
            meili_key,
            page_size,
            json,
        ),
        Command::ConsolidationsReplay {
            from,
            ingest_url,
            dry_run,
            limit,
            json,
        } => consolidation::consolidations_replay(from, ingest_url, dry_run, limit, json),
        Command::Canary {
            hook,
            ipc,
            api_url,
            deadline_secs,
            json,
        } => canary::canary(hook, ipc, api_url, deadline_secs, json),
        Command::DoctorConsistency {
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
            queries,
            probe_k,
            min_overlap_jaccard,
            json,
        } => doctor::doctor_consistency(
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
            queries,
            probe_k,
            min_overlap_jaccard,
            json,
        ),
        Command::BootstrapDedup {
            repo,
            dry_run,
            apply,
            metadata_db,
            json,
        } => bootstrap::bootstrap_dedup(repo, dry_run, apply, metadata_db, json),
        Command::BootstrapStatus { checkpoint, json } => {
            bootstrap::bootstrap_status(checkpoint, json)
        }
        Command::RepoCanonicalize {
            repo,
            dry_run,
            apply,
            metadata_db,
            json,
        } => bootstrap::repo_canonicalize(repo, dry_run, apply, metadata_db, json),
        Command::SessionsBackfillTool {
            tool,
            dry_run,
            apply,
            limit,
            metadata_db,
            json,
        } => metadata::sessions_backfill_tool(tool, dry_run, apply, limit, metadata_db, json),
        Command::Timeline {
            project,
            as_of,
            branch,
            kind,
            from,
            to,
            limit,
            nexus,
            json,
        } => timeline::timeline(project, as_of, branch, kind, from, to, limit, nexus, json),
        Command::Query {
            query,
            as_of,
            branch,
            projects,
            api_url,
            intent,
            limit,
            json,
        } => query_cmd::query(query, as_of, branch, projects, api_url, intent, limit, json),
        Command::History {
            entity_id,
            as_of,
            nexus,
            limit,
            json,
        } => query_cmd::history(entity_id, as_of, nexus, limit, json),
        Command::Supersession {
            entity_id,
            nexus,
            json,
        } => query_cmd::supersession(entity_id, nexus, json),
        Command::Branch { command } => match command {
            BranchCommand::List {
                project,
                nexus,
                json,
            } => branch_cmd::branch_list(project, nexus, json),
            BranchCommand::Show {
                project,
                branch,
                nexus,
                json,
            } => branch_cmd::branch_show(project, branch, nexus, json),
            BranchCommand::Create {
                project,
                name,
                from,
                valid_time,
                nexus,
                json,
            } => branch_cmd::branch_create(project, name, from, valid_time, nexus, json),
            BranchCommand::Merge {
                project,
                branch,
                strategy,
                nexus,
                json,
            } => branch_cmd::branch_merge(project, branch, strategy, nexus, json),
            BranchCommand::Abandon {
                project,
                branch,
                reason,
                nexus,
                json,
            } => branch_cmd::branch_abandon(project, branch, reason, nexus, json),
        },
        Command::Graph { command } => match command {
            GraphCommand::Drop {
                confirm,
                dry_run,
                nexus,
                json,
            } => graph_cmd::graph_drop(confirm, dry_run, nexus, json),
            GraphCommand::Replay {
                since,
                consumer_id,
                stream,
                metadata_db,
                dry_run,
            } => graph_cmd::graph_replay(since, consumer_id, stream, metadata_db, dry_run),
            GraphCommand::SeekHead {
                synap,
                consumer_id,
                stream,
                metadata_db,
                dry_run,
            } => graph_cmd::graph_seek_head(synap, consumer_id, stream, metadata_db, dry_run),
            GraphCommand::Backfill {
                since,
                archive_root,
                apply,
                limit,
                nexus,
                json,
            } => graph_cmd::graph_backfill(since, archive_root, apply, limit, nexus, json),
            GraphCommand::CommunitiesDetect {
                nexus,
                edge_limit,
                dry_run,
                json,
            } => graph_cmd::graph_communities_detect(nexus, edge_limit, dry_run, json),
        },
        Command::SweepEmpty {
            meili,
            meili_key,
            apply,
            json,
        } => retention::sweep_empty(meili, meili_key, apply, json),
        Command::DedupeLaws {
            meili,
            meili_key,
            index,
            apply,
            json,
        } => bootstrap::dedupe_laws(meili, meili_key, index, apply, json),
        Command::BackfillCrossProject {
            root,
            project,
            nexus,
            dry_run,
            json,
        } => backfill_cross_project::backfill_cross_project(root, project, nexus, dry_run, json),
        Command::TemporalDigest { api_url, json } => {
            temporal_digest::temporal_digest(api_url, json)
        }
        Command::TimelineBackfill {
            nexus,
            dry_run,
            json,
        } => timeline_backfill::timeline_backfill(nexus, dry_run, json),
        Command::DoctorRedactionCoverage {
            synap_url,
            stream,
            json,
        } => doctor_redaction_coverage::run(synap_url, stream, json),
        Command::DoctorDecisions {
            meili_url,
            master_key,
            json,
        } => doctor::doctor_decisions(meili_url, master_key, json),
        Command::DoctorContentAddressable {
            index,
            meili_url,
            master_key,
            json,
        } => doctor::doctor_content_addressable(index, meili_url, master_key, json),
        Command::DecisionsReindex {
            decisions_dir,
            meili_url,
            meili_key,
            index,
            dry_run,
            json,
        } => decisions_reindex::decisions_reindex(
            decisions_dir,
            meili_url,
            meili_key,
            index,
            dry_run,
            json,
        ),
        Command::LawsReindex {
            rules_dir,
            meili_url,
            meili_key,
            index,
            dry_run,
            json,
        } => laws_reindex::laws_reindex(rules_dir, meili_url, meili_key, index, dry_run, json),
        Command::LawsRepair {
            meili_url,
            meili_key,
            index,
            dry_run,
            json,
        } => laws_repair::laws_repair(meili_url, meili_key, index, dry_run, json),
        Command::MeiliRekey {
            index,
            meili_url,
            meili_key,
            dry_run,
            json,
        } => meili_rekey::meili_rekey(index, meili_url, meili_key, dry_run, json),
        Command::Acl { command } => match command {
            AclCommand::Role { command: role_cmd } => match role_cmd {
                AclRoleCommand::Create {
                    name,
                    clearance,
                    compartments,
                    api_url,
                    json,
                } => acl_cmd::acl_role_create(name, clearance, compartments, api_url, json),
                AclRoleCommand::List { api_url, json } => acl_cmd::acl_role_list(api_url, json),
            },
            AclCommand::Grant {
                principal_id,
                role,
                clearance,
                compartments,
                api_url,
                json,
            } => acl_cmd::acl_grant(principal_id, role, clearance, compartments, api_url, json),
            AclCommand::ClassifyRuleList { api_url, json } => {
                acl_cmd::acl_classify_rule_list(api_url, json)
            }
            AclCommand::Whoami { api_url, json } => acl_cmd::acl_whoami(api_url, json),
        },
    }
}

// ---------------------------------------------------------------------------
// phase11v §6 — `retention_sweeps` bookkeeping helper for the seven
// sweeps that previously did NOT write to the canonical table. The
// legacy `retention_sweep` and `rollup` handlers keep their two-step
// `start_retention_sweep` / `finish_retention_sweep` bracket because
// they need the running-row advisory lock; every other sweep relies
// on the `cortex-workers::retention::scheduler` per-job semaphore
// and only needs the single-row write below.
//
// Best-effort: a metadata-store open or write failure logs to
// stderr and returns; the sweep does NOT exit FAILURE because of a
// missed audit row. The dashboard's "Bytes reclaimed last 30 d"
// panel under-reports in that pathological case, but the actual
// cleanup work the sweep just performed is preserved.
// ---------------------------------------------------------------------------

fn resolve_metadata_path_for_bookkeeping() -> std::path::PathBuf {
    // phase11w hot-fix — `CORTEX_HOME` MUST be honoured before
    // `HOME` / `USERPROFILE`. Compose-driven boots set
    // `CORTEX_HOME=/var/lib/cortex` so every bookkeeping write
    // lands on the same `metadata.sqlite` the dashboard
    // (`/v1/retention/state`) and the daemon
    // (`cortex-workers::retention::scheduler`) read from. Without
    // this branch, daemon-spawned sweeps would write to
    // `<HOME>/.cortex/metadata.sqlite` (a NEW DB no other process
    // reads), and the dashboard would surface zero rows under
    // `Bytes reclaimed last 30 d` despite every sweep succeeding.
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(p) = cfg.ingestion.metadata_db.as_deref() {
        return std::path::PathBuf::from(p);
    }
    if let Some(home) = cfg.ingestion.home.as_deref() {
        return std::path::PathBuf::from(home).join("metadata.sqlite");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".cortex")
        .join("metadata.sqlite")
}

fn record_sweep_run(
    sweep_name: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    status: &str,
    stats: cortex_cli::ops::sweep_bookkeeping::SweepStageStats,
) {
    let path = resolve_metadata_path_for_bookkeeping();
    if let Err(e) = cortex_cli::ops::sweep_bookkeeping::note_sweep_completion(
        &path,
        sweep_name,
        started_at,
        chrono::Utc::now(),
        status,
        stats,
    ) {
        eprintln!("{sweep_name}: bookkeeping write failed (non-fatal): {e}");
    }
}

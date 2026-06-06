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
#[path = "cortex-ops/bootstrap.rs"]
mod bootstrap;
#[path = "cortex-ops/canary.rs"]
mod canary;
#[path = "cortex-ops/cas.rs"]
mod cas;
#[path = "cortex-ops/config_audit.rs"]
mod config_audit;
#[path = "cortex-ops/consolidation.rs"]
mod consolidation;
#[path = "cortex-ops/digest.rs"]
mod digest;
#[path = "cortex-ops/doctor.rs"]
mod doctor;
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
#[path = "cortex-ops/meili.rs"]
mod meili;
#[path = "cortex-ops/meili_audit.rs"]
mod meili_audit;
#[path = "cortex-ops/memory_consolidate_cmd.rs"]
mod memory_consolidate_cmd;
#[path = "cortex-ops/metadata.rs"]
mod metadata;
#[path = "cortex-ops/pii.rs"]
mod pii;
#[path = "cortex-ops/plan.rs"]
mod plan;
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
#[path = "cortex-ops/backfill_cross_project.rs"]
mod backfill_cross_project;
#[path = "cortex-ops/branch_cmd.rs"]
mod branch_cmd;
#[path = "cortex-ops/query_cmd.rs"]
mod query_cmd;
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
    /// `cortex-plugin/.mcp.json`, `cortex-plugin/hooks/hooks.json`)
    /// plus cross-checks (e.g. adapter.endpoint must match
    /// CORTEX_INGESTION_URL). Exit codes: `0` all ok, `1` any warn,
    /// `2` any critical.
    DoctorConfig {
        /// Workspace root (defaults to current dir). The audit
        /// expects `.env` and `cortex-plugin/` under this path.
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
        /// Emit JSON instead of plain-text summary.
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
            GraphCommand::Backfill {
                since,
                archive_root,
                json,
            } => graph_cmd::graph_backfill(since, archive_root, json),
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

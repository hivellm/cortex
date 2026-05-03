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

use clap::{Parser, Subcommand};
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
    /// Phase9e — LLM turn digest summarizer. Bucketizes turns
    /// older than 30 d by `(repo, ISO_year_week, top_topic)` and
    /// produces one Sonnet-driven `:Memory{memory_type="turn_digest"}`
    /// per non-empty bucket whose size ≥ 5. Idempotent (already-
    /// digested buckets are no-ops; `--rebuild` re-summarizes in
    /// place); cost-aware via the per-run budget ceiling.
    ///
    /// Today's CLI is a synthetic-suite preview against the
    /// in-memory backend; the production walker (Parquet + classifier
    /// + embedder + Nexus + Parquet rewriter) lands with phase9k.
    TurnDigest {
        /// Override "now" for tests + scheduled runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the bucket plan + per-bucket outcomes without
        /// classifier mutation.
        #[arg(long)]
        dry_run: bool,
        /// Re-summarize buckets that already have a digest.
        #[arg(long)]
        rebuild: bool,
        /// Per-run budget ceiling in US cents.
        #[arg(long, default_value_t = 500)]
        budget_cents: u64,
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
    /// The library surface (`cortex_retention::pii_enforce`)
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
    /// Phase8d — config-coherence audit. Read-only static analysis
    /// of every config surface (`.env`, `~/.cortex/adapter.toml`,
    /// `cortex-plugin/.mcp.json`, `cortex-plugin/hooks/hooks.json`)
    /// + cross-checks (e.g. adapter.endpoint must match
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
/// [`cortex_retention::metadata_reap::ReapPlan`]; the rotator knobs
/// don't have a home there, so the CLI threads them through this
/// small struct.
#[derive(Debug, Default, Clone, Copy)]
struct LogConfigOverrides {
    max_bytes: Option<u64>,
    max_age_days: Option<u32>,
    keep_rotations: Option<usize>,
}

/// Phase9g — target selector for `cortex-ops metadata-reap`. Mirrors
/// [`cortex_retention::metadata_reap::ReapTarget`] but lives in the
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
    let cli = Cli::parse();
    match cli.command {
        Command::Plan { pretty, slice } => match emit_plan(pretty, slice) {
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
        } => doctor(vectorizer, nexus, synap, meili),
        Command::DoctorConfig {
            workspace,
            adapter_toml,
            json,
        } => doctor_config(workspace, adapter_toml, json),
        Command::DoctorAlerts { state_dir, json } => doctor_alerts(state_dir, json),
        Command::DoctorCoverage { api_url, json } => doctor_coverage(api_url, json),
        Command::MeiliPrune {
            time_travel,
            dry_run,
            rebuild,
            batch_size,
            json,
        } => meili_prune(time_travel, dry_run, rebuild, batch_size, json),
        Command::TurnDigest {
            time_travel,
            dry_run,
            rebuild,
            budget_cents,
            json,
        } => turn_digest(time_travel, dry_run, rebuild, budget_cents, json),
        Command::PiiEnforce {
            time_travel,
            dry_run,
            cohort,
            json,
        } => pii_enforce(time_travel, dry_run, cohort, json),
        Command::CasVacuum {
            time_travel,
            dry_run,
            force,
            cas_db,
            json,
        } => cas_vacuum(time_travel, dry_run, force, cas_db, json),
        Command::Rollup {
            time_travel,
            dry_run,
            granularity,
            archive_root,
            json,
        } => rollup(time_travel, dry_run, granularity, archive_root, json),
        Command::Schedule { command } => schedule(command),
        Command::MemoryConsolidate {
            project,
            threshold,
            drift_floor,
            max_clusters,
            apply,
            memory_dir,
            json,
        } => memory_consolidate(
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
        } => metadata_reap(
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
        } => retention_sweep(time_travel, dry_run, batch_size, metadata_db, json),
        Command::Canary {
            hook,
            ipc,
            api_url,
            deadline_secs,
            json,
        } => canary(hook, ipc, api_url, deadline_secs, json),
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
        } => doctor_consistency(
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
        } => bootstrap_dedup(repo, dry_run, apply, metadata_db, json),
        Command::RepoCanonicalize {
            repo,
            dry_run,
            apply,
            metadata_db,
            json,
        } => repo_canonicalize(repo, dry_run, apply, metadata_db, json),
        Command::SessionsBackfillTool {
            tool,
            dry_run,
            apply,
            limit,
            metadata_db,
            json,
        } => sessions_backfill_tool(tool, dry_run, apply, limit, metadata_db, json),
        Command::Graph { command } => match command {
            GraphCommand::Drop {
                confirm,
                dry_run,
                nexus,
                json,
            } => graph_drop(confirm, dry_run, nexus, json),
        },
        Command::SweepEmpty {
            meili,
            meili_key,
            apply,
            json,
        } => sweep_empty(meili, meili_key, apply, json),
        Command::DedupeLaws {
            meili,
            meili_key,
            index,
            apply,
            json,
        } => dedupe_laws(meili, meili_key, index, apply, json),
    }
}

/// Phase11l §7.1 — `cortex-ops graph drop` dispatcher. Calls Nexus
/// `MATCH (n:Label) DETACH DELETE n` for every label Cortex owns,
/// returning the per-label delete count. Refuses to run without
/// `--confirm`; `--dry-run` reports the planned counts via
/// `MATCH (n:Label) RETURN count(n)` without mutation.
///
/// Cortex-owned labels list mirrors `SCHEMA_STATEMENTS` plus the
/// phase11k §1.4 graph-correlation labels. The list lives here
/// rather than in the schema module because it is admin-only —
/// the runtime worker never enumerates labels for deletion.
/// Phase11p §1.2 — `cortex-ops sweep-empty` dispatcher. Lists empty
/// Meili indexes (non-canonical legacy + canonical-but-zero) and
/// optionally `DELETE`s them when `--apply` is set. Non-canonical
/// names are dropped immediately; canonical names need explicit
/// approval because empty-canonical can be a transient state right
/// after a settings PATCH but before the first upsert.
fn sweep_empty(
    meili: Option<String>,
    meili_key: Option<String>,
    apply: bool,
    json: bool,
) -> ExitCode {
    use cortex_workers::fulltext::meili_client::MeiliClient;
    use cortex_workers::fulltext::sweep::{sweep_empty_canonical, sweep_stale_indexes};
    use cortex_workers::fulltext::{FulltextConfig, LiveMeiliClient};

    let meili_url = meili
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let cfg = FulltextConfig {
        meili_url: meili_url.clone(),
        meili_api_key: api_key,
        ..FulltextConfig::default()
    };
    let client = match LiveMeiliClient::new(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sweep-empty: meili client: {e}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sweep-empty: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let canonical_empty = match runtime.block_on(sweep_empty_canonical(&client)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sweep-empty: list canonical empties: {e}");
            return ExitCode::from(2);
        }
    };

    // Plan-only: list non-canonical names alongside canonical empties.
    // sweep_stale_indexes mutates by default (it's the boot-time
    // reaper), so the dry-run path enumerates manually here. Reusing
    // the lib's classifier keeps the predicates honest.
    let indexes = match runtime.block_on(client.list_indexes()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sweep-empty: list indexes: {e}");
            return ExitCode::from(2);
        }
    };
    use cortex_workers::fulltext::routing::is_canonical_index_name;
    let mut non_canonical_empty: Vec<String> = indexes
        .iter()
        .filter(|i| !is_canonical_index_name(&i.uid) && i.number_of_documents == 0)
        .map(|i| i.uid.clone())
        .collect();
    non_canonical_empty.sort();

    let canonical_empty_uids: Vec<String> =
        canonical_empty.iter().map(|c| c.uid.clone()).collect();

    if !apply {
        if json {
            let body = serde_json::json!({
                "mode": "dry_run",
                "meili_url": meili_url,
                "non_canonical_empty": non_canonical_empty,
                "canonical_empty": canonical_empty_uids,
                "total_candidates": non_canonical_empty.len() + canonical_empty_uids.len(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
            );
        } else {
            println!("cortex-ops sweep-empty (dry-run)");
            println!("meili: {meili_url}");
            println!(
                "non-canonical empty ({}):",
                non_canonical_empty.len()
            );
            for n in &non_canonical_empty {
                println!("  would drop: {n}");
            }
            println!(
                "canonical empty ({}):",
                canonical_empty_uids.len()
            );
            for n in &canonical_empty_uids {
                println!("  would drop: {n}");
            }
            println!(
                "total candidates: {}. Pass --apply to delete.",
                non_canonical_empty.len() + canonical_empty_uids.len()
            );
        }
        return ExitCode::SUCCESS;
    }

    // --apply: run the canonical sweep first (it auto-drops non-canonical
    // empties as a side effect of the existing boot-time reaper) and
    // then delete each canonical-empty candidate one by one.
    let stale_report = match runtime.block_on(sweep_stale_indexes(&client)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sweep-empty: sweep_stale_indexes: {e}");
            return ExitCode::from(2);
        }
    };
    let mut canonical_dropped: Vec<String> = Vec::new();
    let mut canonical_failed: Vec<(String, String)> = Vec::new();
    for c in &canonical_empty {
        match runtime.block_on(client.delete_index(&c.uid)) {
            Ok(()) => canonical_dropped.push(c.uid.clone()),
            Err(e) => canonical_failed.push((c.uid.clone(), e.to_string())),
        }
    }

    if json {
        let body = serde_json::json!({
            "mode": "apply",
            "meili_url": meili_url,
            "non_canonical_dropped": stale_report.deleted_stale_empty,
            "non_canonical_warned_names": stale_report.warned_names,
            "canonical_dropped": canonical_dropped,
            "canonical_failed": canonical_failed
                .iter()
                .map(|(n, e)| serde_json::json!({"uid": n, "error": e}))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        println!("cortex-ops sweep-empty (apply)");
        println!("meili: {meili_url}");
        println!(
            "non-canonical dropped: {} (warned-preserved: {})",
            stale_report.deleted_stale_empty, stale_report.kept_warning,
        );
        println!("canonical dropped: {}", canonical_dropped.len());
        for n in &canonical_dropped {
            println!("  dropped: {n}");
        }
        if !canonical_failed.is_empty() {
            println!("canonical failed: {}", canonical_failed.len());
            for (n, e) in &canonical_failed {
                println!("  failed: {n}: {e}");
            }
        }
    }

    if canonical_failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

/// Phase11p §4.3 — dedupe `law.imported` documents on each
/// `cortex-{slug}-governance` Meili index plus the global
/// `cortex_laws`. Groups by `(law_id, content_hash)` and keeps the
/// oldest by `ts`. Default is dry-run; `--apply` issues
/// `DELETE /indexes/{uid}/documents/delete-batch` per group.
fn dedupe_laws(
    meili: Option<String>,
    meili_key: Option<String>,
    target_index: Option<String>,
    apply: bool,
    json: bool,
) -> ExitCode {
    let meili_url = meili
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dedupe-laws: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    runtime.block_on(async move {
        match dedupe_laws_async(&meili_url, api_key.as_deref(), target_index.as_deref(), apply, json).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("dedupe-laws: {e}");
                ExitCode::from(2)
            }
        }
    })
}

async fn dedupe_laws_async(
    meili_url: &str,
    api_key: Option<&str>,
    target_index: Option<&str>,
    apply: bool,
    json: bool,
) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest builder: {e}"))?;

    let auth = |req: reqwest::RequestBuilder| match api_key {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    // List candidate indexes — every per-repo `cortex-{slug}-governance`
    // plus the global `cortex_laws`.
    let candidates: Vec<String> = if let Some(t) = target_index {
        vec![t.to_string()]
    } else {
        let stats: serde_json::Value = auth(http.get(format!("{}/stats", meili_url.trim_end_matches('/'))))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let map = stats
            .get("indexes")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("/stats payload missing `indexes`"))?;
        map.keys()
            .filter(|k| k.ends_with("-governance") || k.as_str() == "cortex_laws")
            .cloned()
            .collect()
    };

    #[derive(Default)]
    struct IndexReport {
        total_docs: usize,
        groups: usize,
        keep: Vec<String>,
        drop: Vec<String>,
    }
    let mut per_index: std::collections::BTreeMap<String, IndexReport> = std::collections::BTreeMap::new();

    for index in &candidates {
        let mut all_docs: Vec<serde_json::Value> = Vec::new();
        let mut offset = 0usize;
        let limit = 1000;
        loop {
            let url = format!(
                "{}/indexes/{}/documents?limit={}&offset={}",
                meili_url.trim_end_matches('/'),
                index,
                limit,
                offset
            );
            let resp = auth(http.get(&url)).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("list documents {index} offset={offset}: status {}", resp.status());
            }
            let body: serde_json::Value = resp.json().await?;
            let results = body
                .get("results")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if results.is_empty() {
                break;
            }
            offset += results.len();
            all_docs.extend(results);
            // Stop when offset exceeds total reported to avoid loops on
            // edge servers.
            let total = body.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if total > 0 && offset >= total {
                break;
            }
        }

        // Group by (law_id, content_hash). Keep the document with the
        // oldest `ts` (numeric epoch ms).
        let mut groups: std::collections::HashMap<
            (String, String),
            Vec<(String, i64)>,
        > = std::collections::HashMap::new();
        for d in &all_docs {
            let id = d.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let law_id = d
                .get("law_id")
                .and_then(|v| v.as_str())
                .or_else(|| d.get("ext").and_then(|e| e.get("law_violation")).and_then(|lv| lv.get("law_id")).and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            let content_hash = d
                .get("content_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() || (law_id.is_empty() && content_hash.is_empty()) {
                continue;
            }
            let ts = d.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
            groups
                .entry((law_id, content_hash))
                .or_default()
                .push((id, ts));
        }

        let mut report = IndexReport {
            total_docs: all_docs.len(),
            ..Default::default()
        };
        for (_key, mut docs) in groups {
            if docs.len() < 2 {
                continue;
            }
            report.groups += 1;
            docs.sort_by_key(|d| d.1);
            let keeper = docs.first().unwrap().0.clone();
            report.keep.push(keeper);
            for (drop_id, _) in docs.into_iter().skip(1) {
                report.drop.push(drop_id);
            }
        }

        per_index.insert(index.clone(), report);
    }

    // Render plan + optionally apply.
    if json {
        let body: serde_json::Value = serde_json::json!({
            "mode": if apply { "apply" } else { "dry_run" },
            "meili_url": meili_url,
            "indexes": per_index
                .iter()
                .map(|(k, v)| serde_json::json!({
                    "uid": k,
                    "total_docs": v.total_docs,
                    "duplicate_groups": v.groups,
                    "kept": v.keep.len(),
                    "to_drop": v.drop.len(),
                    "drop_ids": v.drop,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!("cortex-ops dedupe-laws ({})", if apply { "apply" } else { "dry-run" });
        println!("meili: {meili_url}");
        for (uid, r) in &per_index {
            println!(
                "  {uid}: total={}, groups={}, keep={}, drop={}",
                r.total_docs,
                r.groups,
                r.keep.len(),
                r.drop.len()
            );
        }
        let total_drop: usize = per_index.values().map(|r| r.drop.len()).sum();
        println!("Total docs to drop: {total_drop}");
    }

    if !apply {
        return Ok(());
    }

    // --apply: Meili's `delete-batch` endpoint accepts a JSON array
    // of document ids.
    for (uid, report) in &per_index {
        if report.drop.is_empty() {
            continue;
        }
        let url = format!(
            "{}/indexes/{}/documents/delete-batch",
            meili_url.trim_end_matches('/'),
            uid
        );
        // Chunk into 500 ids per request to stay within Meili's
        // body-size guard.
        for chunk in report.drop.chunks(500) {
            let resp = auth(http.post(&url).json(&chunk)).send().await?;
            if !resp.status().is_success() {
                let detail = resp.text().await.unwrap_or_default();
                anyhow::bail!("delete-batch {uid}: {detail}");
            }
        }
        eprintln!("dedupe-laws: applied {} deletes on {}", report.drop.len(), uid);
    }
    Ok(())
}

fn graph_drop(confirm: bool, dry_run: bool, nexus: Option<String>, json: bool) -> ExitCode {
    use cortex_workers::graph::config::GraphConfig;
    use cortex_workers::graph::nexus_client::{GraphClient, LiveNexusClient};

    const CORTEX_LABELS: &[&str] = &[
        "Session",
        "Turn",
        "ToolCall",
        "AgentCall",
        "Artifact",
        "Symbol",
        "Repo",
        "Decision",
        "Memory",
        "Analysis",
        "Law",
        "LawViolation",
        "Knowledge",
        "Learning",
        "Consolidation",
        "Spec",
        "ExternalPackage",
        "UnresolvedImport",
        "DocSection",
        "Concept",
        "Topic",
        "Tool",
    ];

    if !confirm && !dry_run {
        eprintln!(
            "ERROR: cortex-ops graph drop refuses to run without --confirm. \
             Pass --dry-run to see the plan without mutation."
        );
        return ExitCode::from(2);
    }

    let nexus_url = nexus
        .or_else(|| std::env::var("CORTEX_GRAPH_NEXUS_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string());
    let cfg = GraphConfig {
        nexus_url: nexus_url.clone(),
        ..GraphConfig::default()
    };
    let client = match LiveNexusClient::new(cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            return ExitCode::from(1);
        }
    };
    let _ = (&client as &dyn GraphClient);

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let report = runtime.block_on(async {
        let mut results: Vec<(String, u64)> = Vec::with_capacity(CORTEX_LABELS.len());
        for label in CORTEX_LABELS {
            let cypher = if dry_run {
                format!("MATCH (n:{label}) RETURN count(n) AS c")
            } else {
                format!("MATCH (n:{label}) DETACH DELETE n RETURN count(n) AS c")
            };
            match client.sdk().execute_cypher(&cypher, None).await {
                Ok(out) => {
                    let count = out
                        .rows
                        .first()
                        .and_then(|row| row.get("c"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    results.push((label.to_string(), count));
                }
                Err(e) => {
                    eprintln!("warn: {label}: {e}");
                    results.push((label.to_string(), 0));
                }
            }
        }
        results
    });

    let total: u64 = report.iter().map(|(_, c)| *c).sum();
    if json {
        let payload = serde_json::json!({
            "mode": if dry_run { "dry-run" } else { "applied" },
            "nexus_url": nexus_url,
            "total_nodes": total,
            "by_label": report
                .iter()
                .map(|(l, c)| (l.clone(), *c))
                .collect::<std::collections::BTreeMap<_, _>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops graph drop @ {nexus_url}  ({}{})",
            if dry_run { "dry-run" } else { "applied" },
            if dry_run { "" } else { " — DESTRUCTIVE" }
        );
        for (label, count) in &report {
            println!("  {label:<24}  {count:>10}");
        }
        println!("  {:<24}  {total:>10}", "TOTAL");
    }
    ExitCode::SUCCESS
}

/// Phase10i — `cortex-ops sessions backfill-tool` dispatcher.
/// Walks the metadata `sessions` table for rows whose `tool` is
/// NULL or empty and stamps `--tool` on each. `--dry-run` lists
/// candidate ids without mutating; `--apply` runs the update.
fn sessions_backfill_tool(
    tool: String,
    dry_run: bool,
    apply: bool,
    limit: usize,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("sessions backfill-tool: pass either --dry-run or --apply");
        return ExitCode::from(2);
    }
    if tool.trim().is_empty() {
        eprintln!("sessions backfill-tool: --tool must not be empty");
        return ExitCode::from(2);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!("sessions backfill-tool: cannot resolve metadata DB path");
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sessions backfill-tool: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let candidates = match store.sessions_missing_tool(limit) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sessions backfill-tool: scan: {e}");
            return ExitCode::from(1);
        }
    };
    let candidate_count = candidates.len() as u64;
    let mut stamped: u64 = 0;
    if apply {
        for (sid, _started) in &candidates {
            match store.set_session_tool(sid, &tool) {
                Ok(touched) => stamped += touched,
                Err(e) => {
                    eprintln!("sessions backfill-tool: stamp {sid}: {e}");
                    return ExitCode::from(1);
                }
            }
        }
    }
    if json {
        let payload = serde_json::json!({
            "mode": if apply { "apply" } else { "dry-run" },
            "tool": tool,
            "candidates": candidate_count,
            "stamped": stamped,
            "limit": limit,
            "preview": candidates
                .iter()
                .take(10)
                .map(|(sid, started)| serde_json::json!({
                    "session_id": sid,
                    "started_at": started,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{payload}");
    } else {
        println!(
            "sessions backfill-tool ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!("  tool to stamp:           {tool}");
        println!("  candidate rows:          {candidate_count}");
        println!("  rows stamped:            {stamped}");
        println!("  limit (per invocation):  {limit}");
        let preview = candidates.iter().take(5);
        for (sid, started) in preview {
            println!("    {sid}  ({started})");
        }
        if candidate_count as usize > 5 {
            println!("    ... ({} more)", candidate_count.saturating_sub(5));
        }
    }
    ExitCode::SUCCESS
}

/// Phase10d — `cortex-ops repo-canonicalize` dispatcher.
/// Lowercases mixed-case `repo` columns in the metadata SQLite so
/// the orchestrator's scope filter and the dashboard's wire
/// shape agree on a single canonical form. Live-backend
/// rewrites (Vectorizer, Meili, Nexus) are listed in the report
/// but skipped — those backends carry the repo as a payload
/// field rather than a filterable column, so a separate
/// migration tool will handle them.
fn repo_canonicalize(
    repo: Option<String>,
    dry_run: bool,
    apply: bool,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("repo-canonicalize: pass either --dry-run or --apply");
        return ExitCode::from(2);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!("repo-canonicalize: cannot resolve metadata DB path");
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("repo-canonicalize: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let conn = store.conn();
    let target_filter = repo.clone();

    // Count rewrite candidates (rows whose `repo` differs from its
    // lowercase form) per table.
    let sessions_candidates =
        match count_canonicalize_candidates(conn, "sessions", "repo", target_filter.as_deref()) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("repo-canonicalize: count sessions: {e}");
                return ExitCode::from(1);
            }
        };
    let bootstrap_candidates = match count_canonicalize_candidates(
        conn,
        "bootstrap_jobs",
        "repo_path",
        target_filter.as_deref(),
    ) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("repo-canonicalize: count bootstrap_jobs: {e}");
            return ExitCode::from(1);
        }
    };
    let bootstrap_seen_candidates = match count_canonicalize_candidates(
        conn,
        "bootstrap_seen",
        "repo",
        target_filter.as_deref(),
    ) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("repo-canonicalize: count bootstrap_seen: {e}");
            return ExitCode::from(1);
        }
    };

    let mut sessions_rewrites = 0u64;
    let mut bootstrap_rewrites = 0u64;
    let mut bootstrap_seen_rewrites = 0u64;
    if apply {
        sessions_rewrites =
            match apply_canonicalize(conn, "sessions", "repo", target_filter.as_deref()) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("repo-canonicalize: rewrite sessions: {e}");
                    return ExitCode::from(1);
                }
            };
        bootstrap_rewrites = match apply_canonicalize(
            conn,
            "bootstrap_jobs",
            "repo_path",
            target_filter.as_deref(),
        ) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("repo-canonicalize: rewrite bootstrap_jobs: {e}");
                return ExitCode::from(1);
            }
        };
        bootstrap_seen_rewrites =
            match apply_canonicalize(conn, "bootstrap_seen", "repo", target_filter.as_deref()) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("repo-canonicalize: rewrite bootstrap_seen: {e}");
                    return ExitCode::from(1);
                }
            };
    }
    if json {
        let payload = serde_json::json!({
            "mode": if apply { "apply" } else { "dry-run" },
            "sessions": { "candidates": sessions_candidates, "rewritten": sessions_rewrites },
            "bootstrap_jobs": { "candidates": bootstrap_candidates, "rewritten": bootstrap_rewrites },
            "bootstrap_seen": { "candidates": bootstrap_seen_candidates, "rewritten": bootstrap_seen_rewrites },
            "live_backends_pending": ["vectorizer", "meili", "nexus"],
        });
        println!("{}", payload);
    } else {
        println!(
            "repo-canonicalize ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!(
            "  sessions.repo:           candidates {sessions_candidates}, rewritten {sessions_rewrites}"
        );
        println!(
            "  bootstrap_jobs.repo_path: candidates {bootstrap_candidates}, rewritten {bootstrap_rewrites}"
        );
        println!(
            "  bootstrap_seen.repo:     candidates {bootstrap_seen_candidates}, rewritten {bootstrap_seen_rewrites}"
        );
        println!(
            "  live backends (vectorizer / meili / nexus): \
             carry `repo` in payload bodies; not handled by this tool"
        );
    }
    ExitCode::SUCCESS
}

/// Count rows where `<column>` differs from `lower(<column>)`.
/// Optional `target` restricts to the original-case value (so a
/// caller can pre-flight a single repo migration).
fn count_canonicalize_candidates(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    target: Option<&str>,
) -> rusqlite::Result<u64> {
    let sql = match target {
        Some(_) => format!(
            "SELECT COUNT(*) FROM {table} \
             WHERE {column} = ?1 AND {column} != lower({column})"
        ),
        None => format!(
            "SELECT COUNT(*) FROM {table} \
             WHERE {column} != lower({column})"
        ),
    };
    let count: i64 = match target {
        Some(t) => conn.query_row(&sql, rusqlite::params![t], |r| r.get(0))?,
        None => conn.query_row(&sql, [], |r| r.get(0))?,
    };
    Ok(count.max(0) as u64)
}

/// Apply the lowercase rewrite. Returns the row count touched.
fn apply_canonicalize(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    target: Option<&str>,
) -> rusqlite::Result<u64> {
    let sql = match target {
        Some(_) => format!(
            "UPDATE {table} SET {column} = lower({column}) \
             WHERE {column} = ?1 AND {column} != lower({column})"
        ),
        None => format!(
            "UPDATE {table} SET {column} = lower({column}) \
             WHERE {column} != lower({column})"
        ),
    };
    let touched = match target {
        Some(t) => conn.execute(&sql, rusqlite::params![t])?,
        None => conn.execute(&sql, [])?,
    };
    Ok(touched as u64)
}

/// Phase10c — `cortex-ops bootstrap-dedup` dispatcher. Walks the
/// `bootstrap_seen` ledger looking for `(repo, content_hash)`
/// groups whose distinct paths > 1 (the same redacted body
/// emitted under different file paths) and prints a summary so
/// operators can decide whether to clean up the live lane. The
/// `--apply` flag is reserved for the future live-backend cleanup
/// path; today it returns an actionable error pointing at the
/// dry-run output.
fn bootstrap_dedup(
    repo: Option<String>,
    dry_run: bool,
    apply: bool,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("bootstrap-dedup: pass either --dry-run (read-only) or --apply (reserved)");
        return ExitCode::from(2);
    }
    if apply && !dry_run {
        // Honest about the current scope. The dry-run output is
        // the actionable surface today.
        eprintln!(
            "bootstrap-dedup --apply: not yet wired to the live Vectorizer/Meili/Nexus \
             backends. Re-run with --dry-run to inspect the duplicate groups."
        );
        return ExitCode::from(3);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!(
                "bootstrap-dedup: cannot resolve metadata DB path \
                 (set --metadata-db, $CORTEX_METADATA_DB, or $HOME)"
            );
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bootstrap-dedup: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let total_rows = match store.bootstrap_seen_count(repo.as_deref()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("bootstrap-dedup: count: {e}");
            return ExitCode::from(1);
        }
    };
    let target_repos: Vec<String> = match repo.clone() {
        Some(r) => vec![r],
        None => match list_distinct_dedup_repos(&store) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bootstrap-dedup: list repos: {e}");
                return ExitCode::from(1);
            }
        },
    };
    let mut groups: Vec<cortex_storage::BootstrapSeenDuplicate> = Vec::new();
    for r in &target_repos {
        match store.bootstrap_seen_duplicates_by_hash(r) {
            Ok(g) => groups.extend(g),
            Err(e) => {
                eprintln!("bootstrap-dedup: scan {r}: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if json {
        let payload = serde_json::json!({
            "ledger_rows": total_rows,
            "scanned_repos": target_repos.len(),
            "duplicate_groups": groups.len(),
            "groups": groups
                .iter()
                .map(|g| serde_json::json!({
                    "content_hash": g.content_hash,
                    "paths": g.paths,
                    "count": g.count,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", payload);
    } else {
        println!(
            "bootstrap-dedup ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!("  ledger rows scanned: {total_rows}");
        println!("  repos: {}", target_repos.len());
        println!("  duplicate-by-content groups: {}", groups.len());
        for g in &groups {
            println!(
                "    {hash}  ({n} paths)",
                hash = g.content_hash,
                n = g.count,
            );
            for p in &g.paths {
                println!("      - {p}");
            }
        }
    }
    ExitCode::SUCCESS
}

fn list_distinct_dedup_repos(
    store: &cortex_storage::MetadataStore,
) -> Result<Vec<String>, cortex_storage::MetadataError> {
    let mut stmt = store
        .conn()
        .prepare("SELECT DISTINCT repo FROM bootstrap_seen ORDER BY repo")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn resolve_metadata_db(arg: Option<String>) -> Option<std::path::PathBuf> {
    if let Some(p) = arg {
        return Some(std::path::PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("CORTEX_METADATA_DB") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    if let Some(home) = home_dir() {
        return Some(home.join(".cortex").join("metadata.sqlite"));
    }
    None
}

/// Wire the phase4d doctor: scan the archive, probe Meili, render
/// the report. Read-only end-to-end. Spins up a one-shot Tokio
/// runtime so the surrounding `main` stays sync (the rest of
/// `cortex-ops` does not need an async runtime).
#[allow(clippy::too_many_arguments)]
fn doctor_consistency(
    archive_root: Option<String>,
    meili: Option<String>,
    meili_key: Option<String>,
    vectorizer: Option<String>,
    vectorizer_user: Option<String>,
    vectorizer_password: Option<String>,
    nexus: Option<String>,
    queries: Vec<String>,
    probe_k: usize,
    min_overlap_jaccard: f64,
    json: bool,
) -> ExitCode {
    let archive_root = archive_root
        .or_else(|| std::env::var("CORTEX_ARCHIVE_ROOT").ok())
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let meili_url = match meili.or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok()) {
        Some(u) if !u.is_empty() => u,
        _ => {
            eprintln!("doctor consistency: --meili (or $CORTEX_FULLTEXT_MEILI_URL) is required");
            return ExitCode::FAILURE;
        }
    };
    let meili_key = meili_key.or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok());
    let vectorizer_url = vectorizer
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL").ok())
        .filter(|u| !u.is_empty());
    let vectorizer_user = vectorizer_user
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER").ok())
        .filter(|u| !u.is_empty());
    let vectorizer_password = vectorizer_password
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok())
        .filter(|u| !u.is_empty());
    let nexus_url = nexus
        .or_else(|| std::env::var("CORTEX_NEXUS_URL").ok())
        .filter(|u| !u.is_empty());

    let archive = match cortex_cli::ops::ArchiveProbe::new(&archive_root).scan() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("archive scan failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Phase3 — `tool_call_hash_coverage` probe: walks the same archive
    // root and asserts ≥99% of `tool_call` envelopes captured in the
    // last 24 h carry a non-empty `content_hash`. The probe never
    // fails the run when the window is empty — that's a fresh-stack
    // skip rather than a regression.
    let hash_coverage = cortex_cli::ops::scan_hash_coverage(
        std::path::Path::new(&archive_root),
        chrono::Utc::now().timestamp_millis(),
        cortex_cli::ops::HASH_COVERAGE_WINDOW_HOURS,
        cortex_cli::ops::HASH_COVERAGE_THRESHOLD,
    );

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    let meili_result = runtime.block_on(probe_meili(&meili_url, meili_key.as_deref()));
    let (meili_partitions, non_canonical) = match meili_result {
        Ok(v) => v,
        Err(e) => {
            eprintln!("meili probe failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Vectorizer probe — only runs when both URL and credentials
    // are present. A missing-cred deployment falls back to the v1
    // archive ↔ Meili report.
    let (vec_partitions, non_canonical_vec) = if let (Some(url), Some(user), Some(pwd)) =
        (vectorizer_url, vectorizer_user, vectorizer_password)
    {
        match runtime.block_on(probe_vectorizer(&url, &user, &pwd)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("vectorizer probe failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        (Default::default(), Vec::new())
    };

    let nexus_repo_counts = if let Some(url) = nexus_url {
        match runtime.block_on(probe_nexus(&url)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("nexus probe failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Default::default()
    };

    let mut report = cortex_cli::ops::coverage_report_full(
        archive,
        meili_partitions,
        non_canonical,
        vec_partitions,
        non_canonical_vec,
        nexus_repo_counts,
        cortex_cli::ops::CoverageOptions::default(),
    );
    let hash_failed = hash_coverage.failed;
    report.hash_coverage = Some(hash_coverage);
    if hash_failed {
        report.failed = true;
    }

    // Phase4i — query-overlap probes against the three live lanes.
    // Each lane fans out across its canonical partition list (Meili
    // indexes, Vectorizer collections) or runs a single Cypher
    // query (Nexus repo-grain), then dedupes the result paths into
    // a single top-K set per lane.
    if !queries.is_empty() {
        let meili_indexes: Vec<String> = report
            .rows
            .iter()
            .map(|r| r.partition.meili_index())
            .collect();
        let live_meili = LiveMeiliQueryProbe {
            base_url: meili_url.clone(),
            api_key: meili_key.clone(),
            indexes: meili_indexes.clone(),
        };
        // Vectorizer collection naming mirrors Meili index naming.
        let live_vec = match runtime.block_on(build_live_vec_query_probe(
            &report,
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL").ok(),
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER").ok(),
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok(),
        )) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("vectorizer query probe init failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let live_nexus = match runtime.block_on(build_live_nexus_query_probe(
            std::env::var("CORTEX_NEXUS_URL").ok(),
        )) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("nexus query probe init failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let q_reports = runtime.block_on(cortex_cli::ops::run_query_probes(
            &queries,
            probe_k,
            &live_meili,
            &live_vec,
            &live_nexus,
            min_overlap_jaccard,
        ));
        let any_below = q_reports.iter().any(|r| r.below_threshold);
        report.queries = q_reports;
        if any_below {
            report.failed = true;
        }
    }
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize report: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", cortex_cli::ops::render_coverage_markdown(&report));
    }
    if report.failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn probe_meili(
    url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(
    std::collections::BTreeMap<cortex_cli::ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_workers::fulltext::{FulltextConfig, LiveMeiliClient};
    let config = FulltextConfig {
        meili_url: url.to_string(),
        meili_api_key: api_key.map(String::from),
        ..FulltextConfig::default()
    };
    let client = LiveMeiliClient::new(&config).map_err(|e| anyhow::anyhow!("meili client: {e}"))?;
    cortex_cli::ops::doctor::meili_partition_counts(&client).await
}

async fn probe_vectorizer(
    url: &str,
    user: &str,
    password: &str,
) -> anyhow::Result<(
    std::collections::BTreeMap<cortex_cli::ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_cli::ops::{LiveVectorizerCoverageProbe, VectorizerCoverageScan};
    let probe = LiveVectorizerCoverageProbe::new(url, user, password).await?;
    probe.scan().await
}

async fn probe_nexus(url: &str) -> anyhow::Result<cortex_cli::ops::NexusCounts> {
    use cortex_cli::ops::{LiveNexusCoverageProbe, NexusCoverageScan};
    use cortex_workers::graph::GraphConfig;
    // GraphConfig::from_env reads the rest of the auth / transport
    // knobs (CORTEX_NEXUS_USER / _PASSWORD, transport selection, …)
    // so we let the operator set them through the same env vars the
    // streaming worker already honours.
    let mut config = GraphConfig::from_env();
    config.nexus_url = url.to_string();
    let probe = LiveNexusCoverageProbe::new(config)?;
    probe.scan().await
}

// ----- Phase4i live query probes ------------------------------------

/// Live Meili query probe — POSTs `/indexes/{uid}/search` to every
/// canonical index discovered by the coverage probe and dedupes the
/// hit `path` fields into a single top-K list. Empty when no index
/// returned anything (transport failure, missing auth) — by
/// contract the per-lane probe never propagates errors so a single
/// bad lane doesn't poison the whole probe run.
struct LiveMeiliQueryProbe {
    base_url: String,
    api_key: Option<String>,
    indexes: Vec<String>,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveMeiliQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok();
        let client = match client {
            Some(c) => c,
            None => return Vec::new(),
        };
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for uid in &self.indexes {
            let url = format!(
                "{}/indexes/{}/search",
                self.base_url.trim_end_matches('/'),
                uid
            );
            let mut req = client
                .post(&url)
                .json(&serde_json::json!({ "q": query, "limit": k }));
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }
            let body: serde_json::Value = match req.send().await {
                Ok(r) => match r.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };
            let hits = match body.get("hits").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for hit in hits {
                if let Some(path) = hit.get("path").and_then(|v| v.as_str()) {
                    seen.insert(path.to_string());
                } else if let Some(id) = hit.get("id").and_then(|v| v.as_str()) {
                    // Fall back to id when no `path` field — the
                    // Meili schema stamps `id` as the canonical
                    // dedup key, so it works as a stand-in for the
                    // overlap check.
                    seen.insert(id.to_string());
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

/// Live Vectorizer query probe — calls `search_vectors(...)` against
/// every canonical collection discovered by the coverage probe.
/// Result paths come from the per-hit `metadata.path` slot.
struct LiveVectorizerQueryProbe {
    client: vectorizer_sdk::VectorizerClient,
    collections: Vec<String>,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveVectorizerQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for col in &self.collections {
            let resp = match self.client.search_vectors(col, query, Some(k), None).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            for hit in resp.results {
                let path = hit
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or(hit.id);
                seen.insert(path);
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

async fn build_live_vec_query_probe(
    report: &cortex_cli::ops::DoctorReport,
    base_url: Option<String>,
    user: Option<String>,
    password: Option<String>,
) -> anyhow::Result<LiveVectorizerQueryProbe> {
    let url = base_url.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_URL is required for --query probes")
    })?;
    let user = user.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_USER is required for --query probes")
    })?;
    let password = password.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_PASSWORD is required for --query probes")
    })?;
    let pre_auth = vectorizer_sdk::ClientConfig {
        base_url: Some(url.clone()),
        api_key: None,
        timeout_secs: Some(30),
        ..vectorizer_sdk::ClientConfig::default()
    };
    let auth_client = vectorizer_sdk::VectorizerClient::new(pre_auth)
        .map_err(|e| anyhow::anyhow!("vectorizer client: {e}"))?;
    let token = auth_client
        .login(&user, &password)
        .await
        .map_err(|e| anyhow::anyhow!("vectorizer login: {e}"))?;
    let bearer = vectorizer_sdk::ClientConfig {
        base_url: Some(url),
        api_key: Some(token.access_token),
        timeout_secs: Some(30),
        ..vectorizer_sdk::ClientConfig::default()
    };
    let client = vectorizer_sdk::VectorizerClient::new(bearer)
        .map_err(|e| anyhow::anyhow!("vectorizer authenticated client: {e}"))?;
    // Use the same canonical naming as the coverage probe rows —
    // every populated `(repo, family)` row maps to one collection.
    let collections: Vec<String> = report
        .rows
        .iter()
        .filter(|r| r.vec_vectors.unwrap_or(0) > 0)
        .map(|r| r.partition.meili_index())
        .collect();
    Ok(LiveVectorizerQueryProbe {
        client,
        collections,
    })
}

/// Live Nexus query probe — substring match on `Artifact.body`.
/// Returns `a.path` projections, deduped + truncated to `k`.
struct LiveNexusQueryProbe {
    client: cortex_workers::graph::LiveNexusClient,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveNexusQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        // Bind the query as a literal Cypher string — the Cypher
        // CONTAINS operator does substring match. Escape the few
        // characters that can break the literal; everything else
        // passes through verbatim.
        let safe = query.replace('\\', "\\\\").replace('"', "\\\"");
        let cypher = format!(
            "MATCH (a:Artifact) \
             WHERE toLower(a.body) CONTAINS toLower(\"{safe}\") \
             RETURN a.path AS path LIMIT {k}",
        );
        let result = match self.client.execute_with_retry(&cypher, None).await {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for row in &result.rows {
            if let Some(arr) = row.as_array() {
                if let Some(p) = arr.first().and_then(|v| v.as_str()) {
                    seen.insert(p.to_string());
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

async fn build_live_nexus_query_probe(
    nexus_url: Option<String>,
) -> anyhow::Result<LiveNexusQueryProbe> {
    let url = nexus_url
        .ok_or_else(|| anyhow::anyhow!("CORTEX_NEXUS_URL is required for --query probes"))?;
    let mut config = cortex_workers::graph::GraphConfig::from_env();
    config.nexus_url = url;
    let client = cortex_workers::graph::LiveNexusClient::new(config)
        .map_err(|e| anyhow::anyhow!("nexus client: {e}"))?;
    Ok(LiveNexusQueryProbe { client })
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}

fn emit_plan(pretty: bool, slice: PlanSlice) -> anyhow::Result<()> {
    use cortex_storage::{
        collections::COLLECTIONS,
        fulltext::INDEXES,
        graph::BOOTSTRAP_STATEMENTS,
        streams::{KV_NAMESPACES, STREAMS},
    };

    let mut out = serde_json::Map::new();
    if matches!(slice, PlanSlice::All | PlanSlice::Collections) {
        out.insert("collections".into(), serde_json::to_value(COLLECTIONS)?);
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Cypher) {
        out.insert("cypher".into(), serde_json::to_value(BOOTSTRAP_STATEMENTS)?);
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Indexes) {
        let rows: Vec<_> = INDEXES
            .iter()
            .map(|idx| {
                serde_json::json!({
                    "name": idx.name,
                    "primary_key": idx.primary_key,
                    "settings": serde_json::from_str::<serde_json::Value>(idx.settings_json).unwrap_or_default()
                })
            })
            .collect();
        out.insert("indexes".into(), serde_json::Value::Array(rows));
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Streams) {
        out.insert("streams".into(), serde_json::to_value(STREAMS)?);
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Kv) {
        out.insert("kv_namespaces".into(), serde_json::to_value(KV_NAMESPACES)?);
    }
    let value = serde_json::Value::Object(out);
    let rendered = if pretty {
        serde_json::to_string_pretty(&value)?
    } else {
        serde_json::to_string(&value)?
    };
    println!("{rendered}");
    Ok(())
}

/// Phase8d — `cortex-ops doctor-config`. Runs the cortex-api config
/// audit (read-only, static analysis) and renders either a plain-text
/// table or JSON. Exit codes match `Severity`: 0=ok, 1=warn, 2=critical.
fn doctor_config(workspace: Option<String>, adapter_toml: Option<String>, json: bool) -> ExitCode {
    use cortex_api::config_audit::{run_audit_with, AuditOptions, AuditPaths, Severity};

    let workspace = workspace
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let mut paths = AuditPaths::default_for_workspace(&workspace);
    if let Some(p) = adapter_toml {
        paths.adapter_toml = std::path::PathBuf::from(p);
    }
    // Phase8d — `full()` adds live-port + cargo-tree -d scans on
    // top of the file-only static analysis so the CLI surfaces the
    // 2026-04-28 incident class (config says :17010 but daemon
    // bound :15010).
    let audit = run_audit_with(&paths, AuditOptions::full());
    if json {
        match serde_json::to_string_pretty(&audit) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize audit: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops doctor-config");
        println!("workspace: {}", workspace.display());
        println!("surfaces read: {}\n", audit.surfaces_read);
        for f in &audit.findings {
            let marker = match f.severity {
                Severity::Ok => "ok      ",
                Severity::Warn => "WARN    ",
                Severity::Critical => "CRITICAL",
            };
            println!("{marker}  [{}] {}", f.source, f.message);
        }
        println!("\nworst severity: {:?}", audit.worst_severity());
    }
    match audit.worst_severity() {
        Severity::Ok => ExitCode::SUCCESS,
        Severity::Warn => ExitCode::from(1),
        Severity::Critical => ExitCode::from(2),
    }
}

/// Phase8e — `cortex-ops doctor-alerts`. Lists every persisted
/// silent-drop alert under `~/.cortex/alerts/<pair>.json` (or the
/// `--state-dir` override). Exit codes: `0` no Critical alerts
/// active, `2` at least one Critical.
fn doctor_alerts(state_dir: Option<String>, json: bool) -> ExitCode {
    use cortex_api::silent_drop::{AlertState, PairState};

    let dir: std::path::PathBuf = match state_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join(".cortex")
                .join("alerts")
        }
    };

    let mut rows: Vec<(String, PairState)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let state: PairState = match serde_json::from_str(&raw) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let pair = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            rows.push((pair, state));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let any_critical = rows
        .iter()
        .any(|(_, s)| matches!(s.alert, AlertState::Critical));

    if json {
        let payload = serde_json::json!({
            "state_dir": dir.display().to_string(),
            "any_critical": any_critical,
            "alerts": rows
                .iter()
                .map(|(p, s)| serde_json::json!({
                    "pair": p,
                    "state": &s.alert,
                    "recovery_streak": s.recovery_streak,
                }))
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize alerts: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops doctor-alerts");
        println!("state_dir:    {}", dir.display());
        println!("any_critical: {any_critical}\n");
        if rows.is_empty() {
            println!(
                "(no persisted alert state — silent-drop watcher idle or no alerts since boot)"
            );
        } else {
            for (pair, state) in &rows {
                let label = match state.alert {
                    AlertState::Ok => "ok      ",
                    AlertState::Warn { .. } => "WARN    ",
                    AlertState::Critical => "CRITICAL",
                };
                println!(
                    "{label}  {} (recovery_streak={})",
                    pair, state.recovery_streak
                );
            }
        }
    }
    if any_critical {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// Phase11e §2.3 — `cortex-ops doctor-coverage`. Hits the
/// `cortex-api` daemon's `/v1/health/coverage` endpoint and
/// renders the per-backend collection / index inventory diff.
///
/// Exit codes mirror the audit's `overall_severity`:
/// - `0` — every expected name present (severity = ok)
/// - `1` — at least one missing (severity = warn)
/// - `2` — nothing expected is present at all (severity =
///   critical), or the daemon is unreachable
fn doctor_coverage(api_url: Option<String>, json: bool) -> ExitCode {
    let url = api_url
        .or_else(|| std::env::var("CORTEX_API_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
    let endpoint = format!("{}/v1/health/coverage", url.trim_end_matches('/'));

    // The workspace's `reqwest` feature set excludes `blocking`, so
    // build a local single-thread tokio runtime for this one
    // call rather than asking the whole binary to be async.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-coverage: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let payload: serde_json::Value = match rt.block_on(async {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let resp = http.get(&endpoint).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{endpoint} returned HTTP {} {}",
                status.as_u16(),
                body.chars().take(200).collect::<String>()
            ));
        }
        let parsed: serde_json::Value = resp.json().await?;
        Ok::<_, anyhow::Error>(parsed)
    }) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("doctor-coverage: {e}");
            return ExitCode::from(2);
        }
    };

    let overall = payload
        .get("overall_severity")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if json {
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-coverage: serialize: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        let slugs = payload
            .get("slugs")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let families = payload
            .get("families")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!("cortex-ops doctor-coverage");
        println!("api_url:         {url}");
        println!("overall:         {overall}");
        println!(
            "expected:        {slugs} slugs × {families} families = {} names",
            slugs * families
        );
        println!();
        if let Some(backends) = payload.get("backends").and_then(|v| v.as_array()) {
            for backend in backends {
                let name = backend
                    .get("backend")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let sev = backend
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let expected = backend
                    .get("expected_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let present = backend
                    .get("present_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let missing = backend
                    .get("missing_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let unexpected = backend
                    .get("unexpected_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let base_url = backend
                    .get("base_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(not configured)");
                let ratio = if expected > 0 {
                    (present as f64 / expected as f64) * 100.0
                } else {
                    0.0
                };
                println!("[{sev:>4}] {name} @ {base_url}");
                println!(
                    "       {present}/{expected} present ({ratio:.0}%) · {missing} missing · {unexpected} orphan"
                );
                if let Some(missing_names) = backend.get("missing").and_then(|v| v.as_array()) {
                    let take = missing_names.iter().take(10);
                    for name in take {
                        if let Some(n) = name.as_str() {
                            println!("         missing: {n}");
                        }
                    }
                    if missing_names.len() > 10 {
                        println!(
                            "         …{} more missing — see /v1/health/coverage for full list",
                            missing_names.len() - 10
                        );
                    }
                }
                if let Some(err) = backend.get("error").and_then(|v| v.as_str()) {
                    println!("         error: {err}");
                }
                println!();
            }
        }
    }

    match overall {
        "ok" => ExitCode::SUCCESS,
        "warn" => ExitCode::from(1),
        "critical" | _ => ExitCode::from(2),
    }
}

/// Phase9f — `cortex-ops meili-prune`. Today's surface is a
/// synthetic preview against the in-memory backend; the production
/// pipeline (Meili `update_documents` task await) lands with
/// phase9k's cron scheduler.
fn meili_prune(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    batch_size: u32,
    json: bool,
) -> ExitCode {
    use cortex_retention::meili_prune::{run_meili_prune, MeiliDoc, MemoryMeiliBackend, PrunePlan};

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let mut plan = PrunePlan::default_for(now);
    plan.dry_run = dry_run;
    plan.rebuild = rebuild;
    plan.batch_size = batch_size;

    let backend = MemoryMeiliBackend::new();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Synthetic preview: 3 turns over 90 d + 1 fresh + 1 oversize.
    let preview_seed = vec![
        MeiliDoc {
            event_id: "01PREVIEW-T1".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(91),
            summary: "preview short summary 1".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-T2".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(120),
            summary: "preview short summary 2".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-FRESH".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(5),
            summary: "fresh — should be left alone".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-BIG".to_string(),
            index: "cortex_tool_calls".to_string(),
            occurred_at: now - chrono::Duration::days(100),
            summary: "x".repeat(8_000),
            already_pruned: false,
        },
    ];
    runtime.block_on(async {
        backend
            .seed("cortex_turns", preview_seed[..3].to_vec())
            .await;
        backend
            .seed("cortex_tool_calls", vec![preview_seed[3].clone()])
            .await;
    });

    let report = match runtime.block_on(run_meili_prune(&plan, &backend)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-prune: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops meili-prune (preview)");
        println!("now:               {}", now.to_rfc3339());
        println!("dry_run:           {dry_run}");
        println!("rebuild:           {rebuild}");
        println!("batch_size:        {batch_size}");
        println!("examined:          {}", report.examined);
        println!("pruned:            {}", report.pruned);
        println!("summaries_capped:  {}", report.summaries_capped);
        println!("skipped:           {}", report.skipped);
        for (idx, n) in &report.per_index {
            println!("  {idx}: {n}");
        }
    }
    ExitCode::SUCCESS
}

/// Phase9e — `cortex-ops turn-digest`. Today's surface is a
/// synthetic preview against the in-memory backend; the production
/// pipeline (Parquet walker → classifier → embedder → Nexus →
/// Parquet rewriter) lands with phase9k's cron scheduler. The CLI
/// prints the bucket plan + per-bucket outcomes so operators can
/// verify the spec contract before phase9k runs the live pipeline.
fn turn_digest(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    budget_cents: u64,
    json: bool,
) -> ExitCode {
    use cortex_retention::turn_digest::{run_turn_digest, DigestPlan, MemoryDigestBackend, Turn};

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let mut plan = DigestPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.rebuild = rebuild;
    plan.max_usd_cents_per_run = budget_cents;

    // Synthetic preview suite — 8 turns @ 60 days under (alpha,
    // ISO_week, "auth") plus 8 turns under (alpha, same week,
    // "ingestion"). Bucketize emits 2 buckets ≥ min_bucket_size=5.
    let mut turns = Vec::new();
    for topic in ["auth", "ingestion"] {
        for i in 0..8 {
            turns.push(Turn {
                event_id: format!("01PREVIEW-{topic}-{i}"),
                repo: "alpha".to_string(),
                occurred_at: now - chrono::Duration::days(60),
                top_topic: topic.to_string(),
                summarized_by: None,
            });
        }
    }

    let backend = MemoryDigestBackend::new();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match runtime.block_on(run_turn_digest(&plan, &backend, turns)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("turn-digest: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops turn-digest (preview)");
        println!("now:                  {}", now.to_rfc3339());
        println!("dry_run:              {dry_run}");
        println!("rebuild:              {rebuild}");
        println!("budget_cents:         {budget_cents}");
        println!("examined:             {}", report.examined);
        println!("buckets_done:         {}", report.buckets_done);
        println!("already_digested:     {}", report.already_digested);
        println!("buckets_pending:      {}", report.buckets_pending);
        println!("usd_cents:            {}", report.usd_cents);
        for o in &report.outcomes {
            let label = if o.digested {
                "OK      "
            } else if o.already_digested {
                "ALREADY "
            } else if o.error.is_some() {
                "FAILED  "
            } else {
                "PENDING "
            };
            println!("  {label}  {}", o.key);
        }
    }
    ExitCode::SUCCESS
}

/// Phase9d — `cortex-ops pii-enforce`. Today's surface is a
/// dry-run probe against the documented cohort matrix; the live
/// backend wiring (Vectorizer / Meili / CAS / classifier) lands
/// with phase9k's cron scheduler. The CLI prints the cohort
/// assignment for a synthetic suite so operators can verify the
/// matcher logic against the spec ladder before the production
/// run executes.
fn pii_enforce(
    time_travel: Option<String>,
    dry_run: bool,
    cohort: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::pii_enforce::{
        run_enforcement, EnforcementPlan, MemoryPiiBackend, PiiCohort, PiiRisk, PiiTarget,
    };

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let mut plan = EnforcementPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.cohort_filter = match cohort.as_deref() {
        None => None,
        Some("high") => Some(PiiCohort::High30d),
        Some("medium") => Some(PiiCohort::Medium90d),
        Some("null") | Some("null_safety") => Some(PiiCohort::NullSafety90d),
        Some(other) => {
            eprintln!("--cohort: unknown value `{other}` (expected high|medium|null)");
            return ExitCode::FAILURE;
        }
    };

    // Synthetic preview suite: one record per cohort + a fresh
    // record (no-op) + an already-redacted record (idempotence).
    // The synthetic shape lets operators verify the matcher
    // contract without a live archive read; the production walker
    // lands with phase9k.
    let targets = vec![
        PiiTarget {
            event_id: "01PREVIEW-HIGH".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(31),
            body_ref: Some("sha256:preview-high".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-MEDIUM".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::Medium),
            occurred_at: now - chrono::Duration::days(91),
            body_ref: Some("sha256:preview-medium".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-NULL".to_string(),
            kind: "turn".to_string(),
            pii_risk: None,
            occurred_at: now - chrono::Duration::days(95),
            body_ref: Some("sha256:preview-null".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-FRESH".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(5),
            body_ref: None,
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-DONE".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(200),
            body_ref: None,
            redacted: Some("pii_high_30d".to_string()),
        },
    ];

    let backend = MemoryPiiBackend::new();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match runtime.block_on(run_enforcement(&plan, &backend, targets)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pii-enforce: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops pii-enforce (preview)");
        println!("now:           {}", now.to_rfc3339());
        println!("dry_run:       {dry_run}");
        println!("examined:      {}", report.examined);
        println!("applied:       {}", report.applied);
        println!("skipped:       {}", report.skipped);
        println!("warnings:      {}", report.null_safety_warnings);
        if !report.cohort_counts.is_empty() {
            println!("cohort counts:");
            for (k, v) in &report.cohort_counts {
                println!("  {k}: {v}");
            }
        }
    }
    ExitCode::SUCCESS
}

/// Phase9c — `cortex-ops cas-vacuum`. Drops orphaned CAS blobs and
/// reclaims disk via SQLite `VACUUM` when the freelist warrants it.
fn cas_vacuum(
    time_travel: Option<String>,
    dry_run: bool,
    force: bool,
    cas_db: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::cas_vacuum::{open_store, run, VacuumError, VacuumOpts};

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let cas_path = cas_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("CORTEX_CAS_DB")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/cas.sqlite"))
                .unwrap_or_else(|| std::path::PathBuf::from(".cortex/cas.sqlite"))
        });

    let mut store = match open_store(&cas_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cas-vacuum: open ({}): {e}", cas_path.display());
            return ExitCode::FAILURE;
        }
    };

    let mut opts = VacuumOpts::default_for(now);
    opts.dry_run = dry_run;
    opts.force = force;

    match run(&mut store, &opts) {
        Ok(report) => {
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!("cortex-ops cas-vacuum");
                println!("now:           {}", now.to_rfc3339());
                println!("cas_db:        {}", cas_path.display());
                println!("dry_run:       {dry_run}");
                println!("total_blobs:   {}", report.total_blobs);
                println!("dropped:       {}", report.blobs_dropped);
                println!("reclaimed:     {} B", report.bytes_reclaimed);
                println!(
                    "free_ratio:    {:.2} (vacuum={})",
                    report.free_pages_ratio, report.did_vacuum
                );
                if report.did_vacuum {
                    println!("vacuum_ms:     {}", report.vacuum_ms);
                }
                if report.safeguard_tripped {
                    println!("WARN safeguard would trip on a live run (>50 % of total)");
                }
            }
            ExitCode::SUCCESS
        }
        Err(VacuumError::SafeguardTripped {
            would_drop,
            total_blobs,
        }) => {
            eprintln!(
                "cas-vacuum: safeguard tripped — would_drop={would_drop} > 50 % of total_blobs={total_blobs}; pass --force to override"
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("cas-vacuum: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Phase9b — `cortex-ops rollup`. Compacts the archive's hourly /
/// daily / monthly Parquet partitions per spec 19. Quarantines
/// `*.corrupted*` and orphan `*.tmp` files on entry so the working
/// tree is clean before compaction starts.
fn rollup(
    time_travel: Option<String>,
    dry_run: bool,
    granularity: RollupGranularityArg,
    archive_root: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::parquet_rollup::{
        apply_three_year_drop, compact_partition, enumerate_compactable, quarantine_pre_existing,
        Granularity, RollupCounts,
    };

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let archive = archive_root
        .or_else(|| std::env::var("CORTEX_ARCHIVE_ROOT").ok())
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let archive_path = std::path::PathBuf::from(&archive);

    let granularities: Vec<Granularity> = match granularity {
        RollupGranularityArg::All => vec![
            Granularity::HourlyToDaily,
            Granularity::DailyToMonthly,
            Granularity::ThreeYearDrop,
        ],
        RollupGranularityArg::HourlyToDaily => vec![Granularity::HourlyToDaily],
        RollupGranularityArg::DailyToMonthly => vec![Granularity::DailyToMonthly],
        RollupGranularityArg::ThreeYearDrop => vec![Granularity::ThreeYearDrop],
    };

    let mut totals = RollupCounts::default();
    // Pre-flight: quarantine corrupted + orphan tmp files.
    let qcounts = if dry_run {
        RollupCounts::default()
    } else {
        quarantine_pre_existing(&archive_path)
    };
    totals.merge(&qcounts);

    let mut per_granularity: Vec<(Granularity, RollupCounts, usize)> = Vec::new();
    let mut had_error = false;
    for g in granularities {
        let plans = enumerate_compactable(&archive_path, now, g);
        let plan_count = plans.len();
        let mut sub = RollupCounts::default();
        if !dry_run {
            for plan in &plans {
                let result = match g {
                    Granularity::ThreeYearDrop => apply_three_year_drop(&archive_path, plan),
                    _ => compact_partition(&archive_path, plan),
                };
                match result {
                    Ok(c) => sub.merge(&c),
                    Err(e) => {
                        had_error = true;
                        tracing::warn!(error = %e, "rollup: partition compaction failed");
                    }
                }
            }
        }
        per_granularity.push((g, sub.clone(), plan_count));
        totals.merge(&sub);
    }

    if json {
        let payload = serde_json::json!({
            "now": now.to_rfc3339(),
            "archive_root": archive,
            "dry_run": dry_run,
            "totals": totals,
            "per_granularity": per_granularity
                .iter()
                .map(|(g, c, n)| serde_json::json!({
                    "granularity": g.as_str(),
                    "plans": *n,
                    "counts": c,
                }))
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops rollup");
        println!("now:           {}", now.to_rfc3339());
        println!("archive_root:  {archive}");
        println!("dry_run:       {dry_run}");
        println!(
            "totals:        files_in={} files_out={} reclaimed={}B quarantined={} preserved={} dropped={}",
            totals.files_in,
            totals.files_out,
            totals.bytes_reclaimed,
            totals.quarantined,
            totals.records_preserved,
            totals.records_dropped,
        );
        for (g, c, plans) in &per_granularity {
            println!(
                "  {:<18} plans={plans} files_in={} files_out={} reclaimed={}B preserved={} dropped={}",
                g.as_str(),
                c.files_in,
                c.files_out,
                c.bytes_reclaimed,
                c.records_preserved,
                c.records_dropped,
            );
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Phase9a — `cortex-ops retention-sweep`. Runs one tier-transition
/// pass and exits. Idempotent + concurrency-safe via the
/// `retention_sweeps` table.
///
/// Production Vectorizer integration is wired through the
/// `VectorizerOps` trait; this CLI surface uses the in-memory ops
/// (`MemoryVectorizerOps`) so the dry-run path works without a
/// running Vectorizer server. Live ops integration ships in a
/// follow-up that adds the SDK adapter — keeping the trait surface
/// stable now means that switch is one line.
fn retention_sweep(
    time_travel: Option<String>,
    dry_run: bool,
    batch_size: u32,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::{run_sweep, MemoryVectorizerOps, SweepError, SweepPlan};
    use cortex_storage::MetadataStore;

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let mut plan = SweepPlan::default_for(now);
    plan.batch_size = batch_size;
    plan.dry_run = dry_run;

    let metadata_path = metadata_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("CORTEX_METADATA_DB")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join(".cortex")
                .join("metadata.sqlite")
        });

    let store = match MetadataStore::open(&metadata_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("metadata open ({}): {e}", metadata_path.display());
            return ExitCode::FAILURE;
        }
    };

    let sweep_id = cortex_retention::new_sweep_id();
    if let Err(e) = store.start_retention_sweep(&sweep_id, now, 3600) {
        eprintln!("retention-sweep: {e}");
        // Code 2 — another sweep in flight (per spec).
        return ExitCode::from(2);
    }

    // The MemoryVectorizerOps holds an empty store on a fresh CI
    // run, so the dry-run path emits a `0 demoted / 0 dropped` row
    // plus the canonical plan summary. Live Vectorizer integration
    // swaps `ops` for the SDK adapter.
    let ops = MemoryVectorizerOps::new();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    let outcome = runtime.block_on(run_sweep(&plan, &ops));
    let finished_at = chrono::Utc::now();
    let (status, exit) = match &outcome {
        Ok(_) => ("success", ExitCode::SUCCESS),
        Err(SweepError::ErrorRateExceeded { .. }) => ("failed", ExitCode::FAILURE),
        Err(SweepError::Vectorizer(_)) => ("failed", ExitCode::FAILURE),
    };

    let report = outcome.unwrap_or_default();
    if let Err(e) = store.finish_retention_sweep(
        &sweep_id,
        finished_at,
        report.records_demoted,
        report.records_dropped,
        &report.tier_transitions_json(),
        status,
    ) {
        eprintln!("retention-sweep: bookkeeping write failed: {e}");
        return ExitCode::FAILURE;
    }

    if json {
        let payload = serde_json::json!({
            "sweep_id": sweep_id,
            "started_at": now.to_rfc3339(),
            "finished_at": finished_at.to_rfc3339(),
            "status": status,
            "dry_run": dry_run,
            "records_demoted": report.records_demoted,
            "records_dropped": report.records_dropped,
            "tier_transitions": report.tier_transitions,
            "transitions": report.transitions,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops retention-sweep");
        println!("sweep_id:   {sweep_id}");
        println!("now:        {}", now.to_rfc3339());
        println!("dry_run:    {dry_run}");
        println!("status:     {status}");
        println!(
            "demoted:    {}    dropped: {}",
            report.records_demoted, report.records_dropped
        );
        if report.tier_transitions.is_empty() {
            println!("transitions: (none — every collection within thresholds)");
        } else {
            println!("transitions:");
            for (key, count) in &report.tier_transitions {
                println!("  {key}: {count}");
            }
        }
    }
    exit
}

/// Phase9h — `cortex-ops memory-consolidate`. Embeds, clusters, and
/// merges Claude Code's auto-memory directory. Today's binding wires
/// the deterministic in-process embedder + rule-based merger so the
/// CLI runs offline; the production Sonnet driver lands when the
/// classifier surface exposes a streaming agent client.
fn memory_consolidate(
    project: Option<String>,
    threshold: f32,
    drift_floor: f32,
    max_clusters: Option<usize>,
    apply: bool,
    memory_dir: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_cli::ops::memory_consolidate::{
        memory_dir_for, resolve_project_slug, run, ClusterOutcome, HashingEmbedder, Plan,
        RuleMerger,
    };

    let dir: std::path::PathBuf = match memory_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let slug = match project {
                Some(s) => s,
                None => {
                    let cwd = match std::env::current_dir() {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("memory-consolidate: cwd: {e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    resolve_project_slug(&cwd)
                }
            };
            let home = match home_dir() {
                Some(h) => h,
                None => {
                    eprintln!("memory-consolidate: HOME / USERPROFILE unset");
                    return ExitCode::FAILURE;
                }
            };
            memory_dir_for(&home, &slug)
        }
    };

    let plan = Plan {
        now: chrono::Utc::now(),
        threshold,
        drift_floor,
        max_clusters,
        apply,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let embedder = HashingEmbedder::default();
    let merger = RuleMerger;
    let report = match runtime.block_on(run(&dir, &plan, &embedder, &merger)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("memory-consolidate: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        let payload = serde_json::json!({
            "memory_dir": dir.display().to_string(),
            "files_in": report.files_in,
            "files_out": report.files_out,
            "applied": report.applied,
            "archive_dir": report.archive_dir.as_ref().map(|p| p.display().to_string()),
            "warnings": report
                .warnings
                .iter()
                .map(|(p, msg)| {
                    serde_json::json!({
                        "path": p.display().to_string(),
                        "reason": msg,
                    })
                })
                .collect::<Vec<_>>(),
            "clusters": report
                .clusters
                .iter()
                .map(|c| {
                    let outcome = match &c.outcome {
                        ClusterOutcome::Singleton => serde_json::json!({"kind": "singleton"}),
                        ClusterOutcome::Merged {
                            consolidated_filename,
                            frontmatter,
                        } => serde_json::json!({
                            "kind": "merged",
                            "consolidated_filename": consolidated_filename,
                            "frontmatter": {
                                "name": frontmatter.name,
                                "description": frontmatter.description,
                                "type": frontmatter.kind.as_str(),
                            },
                        }),
                        ClusterOutcome::SkippedDrift { min_cosine } => {
                            serde_json::json!({
                                "kind": "skipped_drift",
                                "min_cosine": min_cosine,
                            })
                        }
                        ClusterOutcome::SkippedAgentError { reason } => {
                            serde_json::json!({
                                "kind": "skipped_agent_error",
                                "reason": reason,
                            })
                        }
                    };
                    serde_json::json!({
                        "type": c.kind.as_str(),
                        "members": c.members,
                        "outcome": outcome,
                    })
                })
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops memory-consolidate");
        println!("memory_dir:  {}", dir.display());
        println!("files_in:    {}", report.files_in);
        println!("files_out:   {}", report.files_out);
        println!("applied:     {}", report.applied);
        if let Some(p) = &report.archive_dir {
            println!("archive_dir: {}", p.display());
        }
        if !report.warnings.is_empty() {
            println!("warnings:");
            for (p, msg) in &report.warnings {
                println!("  {} — {msg}", p.display());
            }
        }
        if report.clusters.is_empty() {
            println!("clusters:   (none)");
        } else {
            println!("clusters:");
            for c in &report.clusters {
                let detail = match &c.outcome {
                    ClusterOutcome::Singleton => "singleton".to_string(),
                    ClusterOutcome::Merged {
                        consolidated_filename,
                        ..
                    } => format!("merged → {consolidated_filename}"),
                    ClusterOutcome::SkippedDrift { min_cosine } => {
                        format!("skipped (drift, min_cosine={min_cosine:.2})")
                    }
                    ClusterOutcome::SkippedAgentError { reason } => {
                        format!("skipped (agent: {reason})")
                    }
                };
                println!(
                    "  [{}] {} files: {}",
                    c.kind.as_str(),
                    c.members.len(),
                    detail
                );
                for m in &c.members {
                    println!("    - {m}");
                }
            }
        }
    }
    ExitCode::SUCCESS
}

/// Phase9k — `cortex-ops schedule` dispatcher. Each subcommand
/// translates straight onto `cortex_cli::ops::scheduler` calls plus
/// a small JSON / plain-text renderer.
fn schedule(command: ScheduleCommand) -> ExitCode {
    use cortex_cli::ops::scheduler::{
        next_after, parse_schedule, run_now, seed_defaults, tick, MemoryRunner, ProcessRunner,
        Scheduler,
    };
    use cortex_storage::MetadataStore;

    fn resolve_db(arg: Option<String>) -> std::path::PathBuf {
        arg.map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("CORTEX_METADATA_DB")
                    .ok()
                    .map(std::path::PathBuf::from)
            })
            .unwrap_or_else(|| {
                home_dir()
                    .map(|h| h.join(".cortex").join("metadata.sqlite"))
                    .unwrap_or_else(|| std::path::PathBuf::from(".cortex/metadata.sqlite"))
            })
    }
    fn open(arg: Option<String>) -> Result<MetadataStore, ExitCode> {
        let path = resolve_db(arg);
        match MetadataStore::open(&path) {
            Ok(s) => Ok(s),
            Err(e) => {
                eprintln!("metadata open ({}): {e}", path.display());
                Err(ExitCode::FAILURE)
            }
        }
    }
    fn build_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => Ok(rt),
            Err(e) => {
                eprintln!("tokio runtime: {e}");
                Err(ExitCode::FAILURE)
            }
        }
    }
    match command {
        ScheduleCommand::List { json, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let jobs = match store.list_cron_jobs() {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("schedule list: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if json {
                let arr: Vec<serde_json::Value> = jobs
                    .iter()
                    .map(|j| {
                        serde_json::json!({
                            "name": j.name,
                            "schedule": j.schedule,
                            "command": j.command,
                            "enabled": j.enabled,
                            "next_run_at": j.next_run_at,
                            "last_run_at": j.last_run_at,
                            "last_status": j.last_status,
                            "failure_streak": j.failure_streak,
                        })
                    })
                    .collect();
                match serde_json::to_string_pretty(&arr) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!(
                    "{:<32} {:<14} {:<8} {:<25} {:<10}",
                    "name", "schedule", "enabled", "next_run_at", "last_status"
                );
                for j in &jobs {
                    println!(
                        "{:<32} {:<14} {:<8} {:<25} {:<10}",
                        j.name,
                        j.schedule,
                        if j.enabled { "yes" } else { "no" },
                        j.next_run_at.clone().unwrap_or_else(|| "—".to_string()),
                        j.last_status.clone().unwrap_or_else(|| "—".to_string()),
                    );
                }
            }
            ExitCode::SUCCESS
        }
        ScheduleCommand::Show {
            name,
            json,
            metadata_db,
        } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let job = match store.get_cron_job(&name) {
                Ok(Some(j)) => j,
                Ok(None) => {
                    eprintln!("unknown job: {name}");
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    eprintln!("schedule show: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if json {
                let payload = serde_json::json!({
                    "name": job.name,
                    "schedule": job.schedule,
                    "command": job.command,
                    "enabled": job.enabled,
                    "last_run_at": job.last_run_at,
                    "last_status": job.last_status,
                    "next_run_at": job.next_run_at,
                    "last_error": job.last_error,
                    "last_stdout": job.last_stdout,
                    "last_stderr": job.last_stderr,
                    "failure_streak": job.failure_streak,
                    "last_warning_at": job.last_warning_at,
                });
                match serde_json::to_string_pretty(&payload) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!("name:           {}", job.name);
                println!("schedule:       {}", job.schedule);
                println!("command:        {}", job.command);
                println!("enabled:        {}", job.enabled);
                println!("next_run_at:    {}", job.next_run_at.unwrap_or_default());
                println!("last_run_at:    {}", job.last_run_at.unwrap_or_default());
                println!("last_status:    {}", job.last_status.unwrap_or_default());
                println!("failure_streak: {}", job.failure_streak);
                if let Some(err) = job.last_error {
                    println!("last_error:     {err}");
                }
                if let Some(out) = job.last_stdout {
                    println!("--- stdout tail ---\n{out}");
                }
                if let Some(err) = job.last_stderr {
                    println!("--- stderr tail ---\n{err}");
                }
            }
            ExitCode::SUCCESS
        }
        ScheduleCommand::Enable { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match store.set_cron_job_enabled(&name, true) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: enabled");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule enable: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Disable { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match store.set_cron_job_enabled(&name, false) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: disabled");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule disable: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Set {
            name,
            cron,
            metadata_db,
        } => {
            if let Err(e) = parse_schedule(&cron) {
                eprintln!("invalid cron: {e}");
                return ExitCode::FAILURE;
            }
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let now = chrono::Utc::now();
            let next = match next_after(&cron, now) {
                Some(t) => t.to_rfc3339(),
                None => {
                    eprintln!("cron: no upcoming run derived from {cron:?}");
                    return ExitCode::FAILURE;
                }
            };
            match store.set_cron_job_schedule(&name, &cron, &next) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: schedule={cron} next_run_at={next}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule set: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::RunNow { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let runner = ProcessRunner;
            let scheduler = Scheduler::new();
            let runtime = match build_runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            let now = chrono::Utc::now();
            match runtime.block_on(run_now(&scheduler, &runner, &store, &name, now)) {
                Ok(out) => {
                    println!("{name}: {} ", out.status);
                    if let Some(err) = out.last_error {
                        eprintln!("last_error: {err}");
                    }
                    match out.status.as_str() {
                        "success" => ExitCode::SUCCESS,
                        "lock_held" => ExitCode::from(2),
                        _ => ExitCode::FAILURE,
                    }
                }
                Err(e) => {
                    eprintln!("schedule run-now: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::SeedDefaults { metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match seed_defaults(&store, chrono::Utc::now()) {
                Ok(n) => {
                    println!(
                        "seeded {n} default jobs ({} total)",
                        store.list_cron_jobs().map(|j| j.len()).unwrap_or(0)
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("seed-defaults: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Tick {
            metadata_db,
            time_travel,
        } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let now = match time_travel {
                Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
                    Ok(t) => t.with_timezone(&chrono::Utc),
                    Err(e) => {
                        eprintln!("--time-travel parse error: {e}");
                        return ExitCode::FAILURE;
                    }
                },
                None => chrono::Utc::now(),
            };
            // For ad-hoc tick invocation we use a fresh in-process
            // MemoryRunner so the tick is observable without
            // double-spawning real children — operators that want
            // to actually trigger sweeps should use `run-now`.
            let runtime = match build_runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            let runner = MemoryRunner::new();
            let scheduler = Scheduler::new();
            match runtime.block_on(tick(&scheduler, &runner, &store, now)) {
                Ok(report) => {
                    println!(
                        "tick: due={} success={} failed={} lock_held={}",
                        report.due, report.successes, report.failures, report.lock_held
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule tick: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Phase9g — `cortex-ops metadata-reap`. Aggregates aged SQLite rows
/// into rollup tables, deletes the sources, and rotates the operator
/// hook logs. Bookkeeping rides on the same `retention_sweeps` row
/// machinery as phase9a so the dashboard sees one entry per run.
fn metadata_reap(
    time_travel: Option<String>,
    dry_run: bool,
    target: MetadataReapTargetArg,
    metadata_db: Option<String>,
    log_dir: Option<String>,
    skip_logs: bool,
    json: bool,
) -> ExitCode {
    use cortex_cli::ops::{rotate_if_needed, LogRotateOpts, LogRotateOutcome};
    use cortex_retention::metadata_reap::{run, ReapPlan, ReapTarget};
    use cortex_storage::MetadataStore;

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let metadata_path = metadata_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("CORTEX_METADATA_DB")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex").join("metadata.sqlite"))
                .unwrap_or_else(|| std::path::PathBuf::from(".cortex/metadata.sqlite"))
        });

    let log_dir = log_dir.map(std::path::PathBuf::from).unwrap_or_else(|| {
        home_dir()
            .map(|h| h.join(".cortex"))
            .unwrap_or_else(|| std::path::PathBuf::from(".cortex"))
    });

    // SQLite rollup. The `Logs` target skips this branch entirely.
    let only_logs = matches!(target, MetadataReapTargetArg::Logs);
    let mut store = if only_logs {
        None
    } else {
        match MetadataStore::open(&metadata_path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("metadata open ({}): {e}", metadata_path.display());
                return ExitCode::FAILURE;
            }
        }
    };

    let sweep_id = cortex_retention::new_sweep_id();
    if let Some(s) = store.as_ref() {
        if let Err(e) = s.start_retention_sweep(&sweep_id, now, 3600) {
            eprintln!("metadata-reap: {e}");
            // Code 2 — another sweep in flight (per spec).
            return ExitCode::from(2);
        }
    }

    let mut plan = ReapPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.target = match target {
        MetadataReapTargetArg::All => ReapTarget::All,
        MetadataReapTargetArg::BootstrapJobs => ReapTarget::BootstrapJobs,
        MetadataReapTargetArg::Sessions => ReapTarget::Sessions,
        MetadataReapTargetArg::ClassifierSpend => ReapTarget::ClassifierSpend,
        MetadataReapTargetArg::Logs => ReapTarget::All, // unused (only_logs branch above)
    };

    // Apply `cortex.toml [retention.metadata]` overrides when the
    // file is present in `<home>/.cortex/cortex.toml`. Missing file,
    // missing section, missing keys all silently fall back to the
    // spec defaults.
    let cortex_toml = home_dir()
        .map(|h| h.join(".cortex").join("cortex.toml"))
        .filter(|p| p.exists());
    let mut log_overrides = LogConfigOverrides::default();
    if let Some(p) = cortex_toml.as_ref() {
        match std::fs::read_to_string(p) {
            Ok(body) => match body.parse::<toml::Value>() {
                Ok(root) => {
                    if let Some(section) = root
                        .get("retention")
                        .and_then(|v| v.get("metadata"))
                        .and_then(|v| v.as_table())
                    {
                        if let Some(v) = section
                            .get("bootstrap_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.bootstrap_retain_days = v;
                        }
                        if let Some(v) = section
                            .get("sessions_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.sessions_retain_days = v;
                        }
                        if let Some(v) = section
                            .get("spend_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.spend_retain_days = v;
                        }
                        if let Some(v) = section.get("log_max_bytes").and_then(|v| v.as_integer()) {
                            log_overrides.max_bytes = u64::try_from(v).ok();
                        }
                        if let Some(v) =
                            section.get("log_max_age_days").and_then(|v| v.as_integer())
                        {
                            log_overrides.max_age_days = u32::try_from(v).ok();
                        }
                        if let Some(v) = section
                            .get("log_keep_rotations")
                            .and_then(|v| v.as_integer())
                        {
                            log_overrides.keep_rotations = usize::try_from(v).ok();
                        }
                    }
                }
                Err(e) => tracing::warn!(error=%e, "cortex.toml parse failed"),
            },
            Err(e) => tracing::warn!(error=%e, "cortex.toml read failed"),
        }
    }

    let report = if let Some(s) = store.as_mut() {
        match run(s.conn_mut(), &plan) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("metadata-reap: {e}");
                let _ =
                    s.finish_retention_sweep(&sweep_id, chrono::Utc::now(), 0, 0, "{}", "failed");
                return ExitCode::FAILURE;
            }
        }
    } else {
        cortex_retention::metadata_reap::ReapReport::default()
    };

    // Log rotation. Always runs unless `--skip-logs`. The reaper
    // tolerates failures here (operator may have a read-only mount)
    // — they surface in the report payload but never fail the run.
    let mut log_outcomes: Vec<(std::path::PathBuf, LogRotateOutcome)> = Vec::new();
    if !skip_logs && !dry_run {
        let mut log_opts = LogRotateOpts::default_for(now);
        if let Some(v) = log_overrides.max_bytes {
            log_opts.max_bytes = v;
        }
        if let Some(v) = log_overrides.max_age_days {
            log_opts.max_age_days = v;
        }
        if let Some(v) = log_overrides.keep_rotations {
            log_opts.keep_rotations = v;
        }
        for name in ["hook-invocations.log", "hook-errors.log"] {
            let p = log_dir.join(name);
            match rotate_if_needed(&p, &log_opts) {
                Ok(o) => log_outcomes.push((p, o)),
                Err(e) => {
                    tracing::warn!(error=%e, path=%p.display(), "log rotation failed");
                }
            }
        }
    }

    // Bookkeeping. `records_demoted` carries the SUM of source rows
    // collapsed across all targets so the dashboard's existing
    // ribbon stays meaningful; `tier_transitions_json` carries the
    // per-target breakdown plus log-rotation counters.
    let collapsed_total =
        report.bootstrap_jobs_collapsed + report.sessions_collapsed + report.spend_collapsed;
    let bookkeeping_json = bookkeeping_payload(&report, &log_outcomes);
    let finished_at = chrono::Utc::now();
    if let Some(s) = store.as_ref() {
        let status = "success";
        if let Err(e) = s.finish_retention_sweep(
            &sweep_id,
            finished_at,
            collapsed_total,
            0,
            &bookkeeping_json,
            status,
        ) {
            eprintln!("metadata-reap: bookkeeping write failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    if json {
        let payload = serde_json::json!({
            "sweep_id": sweep_id,
            "started_at": now.to_rfc3339(),
            "finished_at": finished_at.to_rfc3339(),
            "dry_run": dry_run,
            "target": format!("{:?}", target).to_lowercase(),
            "metadata_reap": serde_json::from_str::<serde_json::Value>(
                &report.metadata_reap_json(),
            )
            .unwrap_or_else(|_| serde_json::json!({})),
            "log_rotations": log_outcomes
                .iter()
                .map(|(p, o)| {
                    serde_json::json!({
                        "path": p.display().to_string(),
                        "rotated": o.rotated,
                        "source_bytes": o.source_bytes,
                        "gz_path": o.gz_path.as_ref().map(|p| p.display().to_string()),
                        "pruned": o.pruned.iter().map(|p| p.display().to_string())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops metadata-reap");
        println!("sweep_id:        {sweep_id}");
        println!("now:             {}", now.to_rfc3339());
        println!("dry_run:         {dry_run}");
        println!("metadata_db:     {}", metadata_path.display());
        println!(
            "bootstrap_jobs:  collapsed={} buckets={}",
            report.bootstrap_jobs_collapsed, report.bootstrap_daily_buckets
        );
        println!(
            "sessions:        collapsed={} buckets={}",
            report.sessions_collapsed, report.sessions_monthly_buckets
        );
        println!(
            "classifier_spend:collapsed={} buckets={}",
            report.spend_collapsed, report.spend_monthly_buckets
        );
        println!(
            "free_pages:      ratio={:.2} did_vacuum={} vacuum_ms={}",
            report.free_pages_ratio, report.did_vacuum, report.vacuum_ms
        );
        if log_outcomes.is_empty() {
            println!("logs:            (skipped)");
        } else {
            println!("logs:");
            for (p, o) in &log_outcomes {
                if o.rotated {
                    println!(
                        "  rotated {} ({} B) → {}",
                        p.display(),
                        o.source_bytes,
                        o.gz_path
                            .as_ref()
                            .map(|q| q.display().to_string())
                            .unwrap_or_default()
                    );
                    if !o.pruned.is_empty() {
                        for old in &o.pruned {
                            println!("    pruned {}", old.display());
                        }
                    }
                } else {
                    println!("  skipped {} ({} B)", p.display(), o.source_bytes);
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn bookkeeping_payload(
    report: &cortex_retention::metadata_reap::ReapReport,
    log_outcomes: &[(std::path::PathBuf, cortex_cli::ops::LogRotateOutcome)],
) -> String {
    let metadata_reap = serde_json::from_str::<serde_json::Value>(&report.metadata_reap_json())
        .unwrap_or_else(|_| serde_json::json!({}));
    let logs: Vec<serde_json::Value> = log_outcomes
        .iter()
        .map(|(p, o)| {
            serde_json::json!({
                "path": p.display().to_string(),
                "rotated": o.rotated,
                "source_bytes": o.source_bytes,
            })
        })
        .collect();
    serde_json::json!({
        "metadata_reap": metadata_reap,
        "log_rotations": logs,
    })
    .to_string()
}

/// Phase8f — `cortex-ops canary`. Sends a synthetic frame through
/// the daemon's IPC and polls the archive for the marker. Exit
/// codes match `CanaryOutcome::exit_code()`.
fn canary(
    hook: String,
    ipc: Option<String>,
    api_url: Option<String>,
    deadline_secs: u64,
    json: bool,
) -> ExitCode {
    use cortex_api::canary::{run_canary_once, CanaryConfig};
    let mut cfg = CanaryConfig::default();
    cfg.deadline_secs = deadline_secs;
    cfg.ipc_path = ipc;
    if let Some(url) = api_url {
        cfg.api_url = url;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(run_canary_once(&cfg, &hook));
    if json {
        match serde_json::to_string_pretty(&outcome) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize outcome: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops canary --hook={hook}");
        println!("{}", outcome.describe());
    }
    match outcome.exit_code() {
        0 => ExitCode::SUCCESS,
        2 => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

fn doctor(
    vectorizer: Option<String>,
    nexus: Option<String>,
    synap: Option<String>,
    meili: Option<String>,
) -> ExitCode {
    // We intentionally do not pull in reqwest here: this binary should stay
    // dependency-light. Doctor delegates to `curl` which is present on every
    // Unix + Windows-with-modern-powershell host.
    let vectorizer = vectorizer.or_else(|| std::env::var("VECTORIZER_URL").ok());
    let nexus = nexus.or_else(|| std::env::var("NEXUS_URL").ok());
    let synap = synap.or_else(|| std::env::var("SYNAP_URL").ok());
    let meili = meili.or_else(|| std::env::var("MEILI_URL").ok());

    let checks: &[(&str, Option<String>, &str)] = &[
        ("vectorizer", vectorizer, "/health"),
        ("nexus", nexus, "/health"),
        ("synap", synap, "/health"),
        ("meilisearch", meili, "/health"),
    ];

    let mut any_failure = false;
    for (name, base, path) in checks {
        match base {
            Some(b) => {
                let url = format!("{}{}", b.trim_end_matches('/'), path);
                let ok = std::process::Command::new("curl")
                    .args(["-fsS", "--max-time", "3", "-o", "/dev/null", &url])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if ok {
                    println!("ok     {:<12} {url}", name);
                } else {
                    println!("fail   {:<12} {url}", name);
                    any_failure = true;
                }
            }
            None => {
                println!("skip   {:<12} (no URL configured)", name);
            }
        }
    }

    // phase10g — mounted-route smoke check. The audit caught the
    // GUI's Health tab returning empty bodies on every
    // `/v1/health/*` call against the live daemon because a
    // future refactor could drop the merge() that mounts them.
    // Doctor probes the five canonical routes against
    // `CORTEX_API_URL` so a missed registration shows up as a
    // doctor red BEFORE the operator opens the GUI.
    if let Some(api) = std::env::var("CORTEX_API_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        for path in [
            "/v1/health",
            "/v1/health/freshness",
            "/v1/health/divergence",
            "/v1/health/versions",
            "/v1/health/config",
        ] {
            let url = format!("{}{}", api.trim_end_matches('/'), path);
            let ok = std::process::Command::new("curl")
                .args(["-fsS", "--max-time", "3", "-o", "/dev/null", &url])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let label = format!("api{path}");
            if ok {
                println!("ok     {label:<28} {url}");
            } else {
                println!("fail   {label:<28} {url}");
                any_failure = true;
            }
        }
    } else {
        println!(
            "skip   {label:<28} (set CORTEX_API_URL to probe the daemon's /v1/health/* routes)",
            label = "api/v1/health/*"
        );
    }

    if any_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

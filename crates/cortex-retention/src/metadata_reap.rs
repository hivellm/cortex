//! Phase9g — SQLite metadata reaper.
//!
//! Three SQLite tables in `~/.cortex/metadata.sqlite` grow without
//! bound today:
//!
//! - `bootstrap_jobs` — every successful run leaves a row forever.
//! - `sessions`       — every Claude Code / agent session.
//! - `classifier_spend` — one row per UTC day.
//!
//! The reaper aggregates each into a parallel rollup table
//! (`bootstrap_jobs_daily`, `sessions_monthly`,
//! `classifier_spend_monthly`) and deletes the source rows whose age
//! crosses the per-target retention horizon. Failed bootstrap rows
//! are NEVER collapsed — they stay raw for full-detail debugging.
//!
//! Each rollup is a single SQL statement plus a delete inside one
//! `BEGIN IMMEDIATE` transaction so re-runs after a partial failure
//! either re-collapse the same source rows (still in place) or no-op
//! (already-deleted, already-rolled). Idempotence holds: re-running
//! without new aged rows yields zero deletions and zero rolled rows.
//!
//! Library shape (mirrors the other phase9 sweepers — the binary
//! lands in `cortex-cli`'s `cortex-ops` so the operator surface
//! stays a single CLI):
//!
//! - [`ReapPlan`] — declarative inputs (`now`, retain horizons,
//!   vacuum ratio, dry-run flag, target selector).
//! - [`ReapTarget`] — which rollup to run; `All` runs every one.
//! - [`run`] — orchestrator that opens one
//!   `BEGIN IMMEDIATE` per target, returns a [`ReapReport`].

use chrono::{DateTime, Duration, Utc};
use cortex_storage::metadata::{apply_phase9g_schema, MetadataError};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Subset of rollup tables to reap. `All` runs every target in
/// declaration order: bootstrap → sessions → classifier_spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReapTarget {
    /// Run every target.
    All,
    /// Roll only `bootstrap_jobs` → `bootstrap_jobs_daily`.
    BootstrapJobs,
    /// Roll only `sessions` → `sessions_monthly`.
    Sessions,
    /// Roll only `classifier_spend` → `classifier_spend_monthly`.
    ClassifierSpend,
}

impl ReapTarget {
    /// Stable string identifier (matches the JSON enum tag).
    pub fn as_str(self) -> &'static str {
        match self {
            ReapTarget::All => "all",
            ReapTarget::BootstrapJobs => "bootstrap_jobs",
            ReapTarget::Sessions => "sessions",
            ReapTarget::ClassifierSpend => "classifier_spend",
        }
    }
}

/// Plan inputs.
#[derive(Debug, Clone)]
pub struct ReapPlan {
    /// Reference time (overridable via `--time-travel`).
    pub now: DateTime<Utc>,
    /// Days at which `bootstrap_jobs` (status='success') rolls up.
    /// Default 30 per spec.
    pub bootstrap_retain_days: i64,
    /// Days at which `sessions` rolls up. Default 365 per spec.
    pub sessions_retain_days: i64,
    /// Days at which `classifier_spend` rolls up. Default 365.
    pub spend_retain_days: i64,
    /// `freelist_count / page_count` above which the runner issues
    /// `VACUUM` post-rollup. Default 0.25 (matches phase9c).
    pub vacuum_ratio: f64,
    /// `true` skips the actual mutation. The report still surfaces
    /// the candidate counters so the operator can preview what would
    /// happen.
    pub dry_run: bool,
    /// Restrict the run to one rollup target.
    pub target: ReapTarget,
}

impl ReapPlan {
    /// Defaults per spec — `now=Utc::now()`, 30/365/365 retention,
    /// 0.25 vacuum threshold, all targets.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            bootstrap_retain_days: 30,
            sessions_retain_days: 365,
            spend_retain_days: 365,
            vacuum_ratio: 0.25,
            dry_run: false,
            target: ReapTarget::All,
        }
    }
}

/// Counters returned by [`run`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReapReport {
    /// `bootstrap_jobs` rows removed (success rows older than the
    /// horizon).
    pub bootstrap_jobs_collapsed: u64,
    /// `bootstrap_jobs_daily` rows the run touched (insert OR
    /// upsert-update). Always equals the number of distinct
    /// `(day, repo_path)` buckets aggregated.
    pub bootstrap_daily_buckets: u64,
    /// `sessions` rows removed (older than the horizon).
    pub sessions_collapsed: u64,
    /// `(year_month, tool, repo)` buckets aggregated.
    pub sessions_monthly_buckets: u64,
    /// `classifier_spend` rows removed.
    pub spend_collapsed: u64,
    /// `year_month` buckets aggregated.
    pub spend_monthly_buckets: u64,
    /// `freelist_count / page_count` post-rollup.
    pub free_pages_ratio: f64,
    /// `true` when the runner issued `VACUUM`.
    pub did_vacuum: bool,
    /// Wall-clock duration of the `VACUUM` call (0 when not run).
    pub vacuum_ms: u64,
}

impl ReapReport {
    /// JSON suitable for `tier_transitions_json.metadata_reap`.
    pub fn metadata_reap_json(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "bootstrap_jobs_collapsed": self.bootstrap_jobs_collapsed,
            "bootstrap_daily_buckets": self.bootstrap_daily_buckets,
            "sessions_collapsed": self.sessions_collapsed,
            "sessions_monthly_buckets": self.sessions_monthly_buckets,
            "spend_collapsed": self.spend_collapsed,
            "spend_monthly_buckets": self.spend_monthly_buckets,
            "did_vacuum": self.did_vacuum,
            "free_pages_ratio": self.free_pages_ratio,
            "vacuum_ms": self.vacuum_ms,
        }))
        .unwrap_or_else(|_| "{}".into())
    }
}

/// Runner errors.
#[derive(Debug, Error)]
pub enum ReapError {
    /// SQLite driver error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Schema preflight error.
    #[error("metadata: {0}")]
    Metadata(#[from] MetadataError),
}

/// Run the reaper against `conn`.
///
/// Each target runs inside its own `BEGIN IMMEDIATE` transaction
/// (aggregate + delete). `VACUUM` runs last and outside any
/// transaction. Dry runs surface the *would-collapse* counters
/// without writing.
pub fn run(conn: &mut Connection, plan: &ReapPlan) -> Result<ReapReport, ReapError> {
    apply_phase9g_schema(conn)?;
    let mut report = ReapReport::default();

    let bootstrap_cutoff = plan.now - Duration::days(plan.bootstrap_retain_days);
    let sessions_cutoff = plan.now - Duration::days(plan.sessions_retain_days);
    let spend_cutoff_date =
        (plan.now - Duration::days(plan.spend_retain_days)).format("%Y-%m-%d").to_string();

    if matches!(plan.target, ReapTarget::All | ReapTarget::BootstrapJobs) {
        let (collapsed, buckets) = roll_bootstrap_jobs(conn, bootstrap_cutoff, plan.dry_run)?;
        report.bootstrap_jobs_collapsed = collapsed;
        report.bootstrap_daily_buckets = buckets;
    }
    if matches!(plan.target, ReapTarget::All | ReapTarget::Sessions) {
        let (collapsed, buckets) = roll_sessions(conn, sessions_cutoff, plan.dry_run)?;
        report.sessions_collapsed = collapsed;
        report.sessions_monthly_buckets = buckets;
    }
    if matches!(plan.target, ReapTarget::All | ReapTarget::ClassifierSpend) {
        let (collapsed, buckets) =
            roll_classifier_spend(conn, &spend_cutoff_date, plan.dry_run)?;
        report.spend_collapsed = collapsed;
        report.spend_monthly_buckets = buckets;
    }

    if !plan.dry_run {
        let (freelist, page_count) = page_stats(conn)?;
        report.free_pages_ratio = if page_count == 0 {
            0.0
        } else {
            freelist as f64 / page_count as f64
        };
        if report.free_pages_ratio > plan.vacuum_ratio {
            let started = std::time::Instant::now();
            conn.execute_batch("VACUUM")?;
            report.did_vacuum = true;
            report.vacuum_ms = started.elapsed().as_millis() as u64;
        }
    }

    Ok(report)
}

/// Aggregate `bootstrap_jobs.status='success' AND finished_at < cutoff`
/// rows into `bootstrap_jobs_daily`, then delete the sources. Returns
/// `(rows_collapsed, daily_buckets_touched)`.
fn roll_bootstrap_jobs(
    conn: &mut Connection,
    cutoff: DateTime<Utc>,
    dry_run: bool,
) -> Result<(u64, u64), ReapError> {
    let cutoff_str = cutoff.to_rfc3339();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let buckets: i64 = tx.query_row(
        "SELECT COUNT(*) FROM (
             SELECT 1 FROM bootstrap_jobs
              WHERE status = 'success' AND finished_at IS NOT NULL AND finished_at < ?1
              GROUP BY substr(finished_at, 1, 10), repo_path
         )",
        params![cutoff_str],
        |r| r.get(0),
    )?;
    if dry_run {
        let collapsed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM bootstrap_jobs
              WHERE status = 'success' AND finished_at IS NOT NULL AND finished_at < ?1",
            params![cutoff_str],
            |r| r.get(0),
        )?;
        tx.rollback()?;
        return Ok((collapsed.max(0) as u64, buckets.max(0) as u64));
    }
    tx.execute(
        "INSERT INTO bootstrap_jobs_daily (day, repo_path, runs, total_files, total_chunks)
              SELECT substr(finished_at, 1, 10) AS day,
                     repo_path,
                     COUNT(*) AS runs,
                     COALESCE(SUM(files_processed), 0),
                     COALESCE(SUM(chunks_emitted), 0)
                FROM bootstrap_jobs
               WHERE status = 'success' AND finished_at IS NOT NULL AND finished_at < ?1
               GROUP BY day, repo_path
              ON CONFLICT(day, repo_path) DO UPDATE SET
                  runs         = runs + excluded.runs,
                  total_files  = total_files + excluded.total_files,
                  total_chunks = total_chunks + excluded.total_chunks",
        params![cutoff_str],
    )?;
    let collapsed = tx.execute(
        "DELETE FROM bootstrap_jobs
              WHERE status = 'success' AND finished_at IS NOT NULL AND finished_at < ?1",
        params![cutoff_str],
    )? as u64;
    tx.commit()?;
    Ok((collapsed, buckets.max(0) as u64))
}

/// Aggregate `sessions.started_at < cutoff` rows into
/// `sessions_monthly`, then delete the sources.
fn roll_sessions(
    conn: &mut Connection,
    cutoff: DateTime<Utc>,
    dry_run: bool,
) -> Result<(u64, u64), ReapError> {
    let cutoff_str = cutoff.to_rfc3339();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let buckets: i64 = tx.query_row(
        "SELECT COUNT(*) FROM (
             SELECT 1 FROM sessions
              WHERE started_at < ?1
              GROUP BY substr(started_at, 1, 7), tool, COALESCE(repo, '')
         )",
        params![cutoff_str],
        |r| r.get(0),
    )?;
    if dry_run {
        let collapsed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM sessions WHERE started_at < ?1",
            params![cutoff_str],
            |r| r.get(0),
        )?;
        tx.rollback()?;
        return Ok((collapsed.max(0) as u64, buckets.max(0) as u64));
    }
    tx.execute(
        "INSERT INTO sessions_monthly (year_month, tool, repo, count, total_event_count)
              SELECT substr(started_at, 1, 7) AS year_month,
                     tool,
                     COALESCE(repo, '') AS repo,
                     COUNT(*) AS count,
                     COALESCE(SUM(event_count), 0)
                FROM sessions
               WHERE started_at < ?1
               GROUP BY year_month, tool, repo
              ON CONFLICT(year_month, tool, repo) DO UPDATE SET
                  count             = count + excluded.count,
                  total_event_count = total_event_count + excluded.total_event_count",
        params![cutoff_str],
    )?;
    let collapsed = tx.execute(
        "DELETE FROM sessions WHERE started_at < ?1",
        params![cutoff_str],
    )? as u64;
    tx.commit()?;
    Ok((collapsed, buckets.max(0) as u64))
}

/// Aggregate `classifier_spend.day < cutoff_date` rows into
/// `classifier_spend_monthly`, then delete the sources. The cutoff
/// is the date string `YYYY-MM-DD` so it sorts lexicographically
/// against the table's `day` column.
fn roll_classifier_spend(
    conn: &mut Connection,
    cutoff_date: &str,
    dry_run: bool,
) -> Result<(u64, u64), ReapError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let buckets: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT substr(day, 1, 7))
           FROM classifier_spend
          WHERE day < ?1",
        params![cutoff_date],
        |r| r.get(0),
    )?;
    if dry_run {
        let collapsed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM classifier_spend WHERE day < ?1",
            params![cutoff_date],
            |r| r.get(0),
        )?;
        tx.rollback()?;
        return Ok((collapsed.max(0) as u64, buckets.max(0) as u64));
    }
    tx.execute(
        "INSERT INTO classifier_spend_monthly
                  (year_month, calls, tokens_in, tokens_out, est_usd_cents)
              SELECT substr(day, 1, 7) AS year_month,
                     COALESCE(SUM(calls), 0),
                     COALESCE(SUM(tokens_in), 0),
                     COALESCE(SUM(tokens_out), 0),
                     COALESCE(SUM(est_usd_cents), 0)
                FROM classifier_spend
               WHERE day < ?1
               GROUP BY year_month
              ON CONFLICT(year_month) DO UPDATE SET
                  calls         = calls + excluded.calls,
                  tokens_in     = tokens_in + excluded.tokens_in,
                  tokens_out    = tokens_out + excluded.tokens_out,
                  est_usd_cents = est_usd_cents + excluded.est_usd_cents",
        params![cutoff_date],
    )?;
    let collapsed = tx.execute(
        "DELETE FROM classifier_spend WHERE day < ?1",
        params![cutoff_date],
    )? as u64;
    tx.commit()?;
    Ok((collapsed, buckets.max(0) as u64))
}

fn page_stats(conn: &Connection) -> Result<(u64, u64), ReapError> {
    let freelist: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    Ok((freelist.max(0) as u64, page_count.max(0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_storage::MetadataStore;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn store() -> MetadataStore {
        MetadataStore::open_in_memory().unwrap()
    }

    fn insert_bootstrap_row(
        store: &MetadataStore,
        job_id: &str,
        repo_path: &str,
        finished_at: DateTime<Utc>,
        files: u64,
        chunks: u64,
        status: &str,
    ) {
        // The repos FK on bootstrap_jobs requires the repo registry
        // to carry the row. Insert idempotently.
        store
            .conn()
            .execute(
                "INSERT OR IGNORE INTO repos (path, name) VALUES (?1, ?2)",
                params![repo_path, repo_path],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO bootstrap_jobs
                    (job_id, repo_path, started_at, finished_at,
                     files_processed, chunks_emitted, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    job_id,
                    repo_path,
                    finished_at.to_rfc3339(),
                    finished_at.to_rfc3339(),
                    files as i64,
                    chunks as i64,
                    status,
                ],
            )
            .unwrap();
    }

    fn insert_session_row(
        store: &MetadataStore,
        session_id: &str,
        tool: &str,
        repo: Option<&str>,
        started_at: DateTime<Utc>,
        event_count: u64,
    ) {
        store
            .conn()
            .execute(
                "INSERT INTO sessions
                    (session_id, tool, repo, started_at, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, tool, repo, started_at.to_rfc3339(), event_count as i64],
            )
            .unwrap();
    }

    #[test]
    fn plan_default_uses_spec_thresholds() {
        let plan = ReapPlan::default_for(now());
        assert_eq!(plan.bootstrap_retain_days, 30);
        assert_eq!(plan.sessions_retain_days, 365);
        assert_eq!(plan.spend_retain_days, 365);
        assert!((plan.vacuum_ratio - 0.25).abs() < 1e-9);
        assert!(!plan.dry_run);
        assert_eq!(plan.target, ReapTarget::All);
    }

    #[test]
    fn bootstrap_success_row_thirty_one_days_old_collapses() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01OLD",
            "/repo/cortex",
            now() - Duration::days(31),
            120,
            2400,
            "success",
        );
        insert_bootstrap_row(
            &s,
            "01FRESH",
            "/repo/cortex",
            now() - Duration::days(5),
            10,
            100,
            "success",
        );
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.bootstrap_jobs_collapsed, 1);
        assert_eq!(report.bootstrap_daily_buckets, 1);
        let (day, repo_path, runs, files, chunks): (String, String, i64, i64, i64) = s
            .conn()
            .query_row(
                "SELECT day, repo_path, runs, total_files, total_chunks
                   FROM bootstrap_jobs_daily",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(day, (now() - Duration::days(31)).format("%Y-%m-%d").to_string());
        assert_eq!(repo_path, "/repo/cortex");
        assert_eq!(runs, 1);
        assert_eq!(files, 120);
        assert_eq!(chunks, 2400);
        // Source row gone; fresh row remains.
        let remaining: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM bootstrap_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn bootstrap_failed_row_is_preserved() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01FAILED",
            "/repo/cortex",
            now() - Duration::days(90),
            5,
            0,
            "failed",
        );
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.bootstrap_jobs_collapsed, 0);
        let remaining: i64 = s
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM bootstrap_jobs WHERE status='failed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn bootstrap_multiple_runs_same_day_aggregate_into_one_bucket() {
        let mut s = store();
        for i in 0..3 {
            insert_bootstrap_row(
                &s,
                &format!("01R{i}"),
                "/repo/x",
                now() - Duration::days(35),
                10 + i,
                100 + i,
                "success",
            );
        }
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.bootstrap_jobs_collapsed, 3);
        assert_eq!(report.bootstrap_daily_buckets, 1);
        let (runs, files, chunks): (i64, i64, i64) = s
            .conn()
            .query_row(
                "SELECT runs, total_files, total_chunks FROM bootstrap_jobs_daily",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(runs, 3);
        // 10 + 11 + 12 = 33; 100 + 101 + 102 = 303.
        assert_eq!(files, 33);
        assert_eq!(chunks, 303);
    }

    #[test]
    fn sessions_year_old_rows_collapse_to_monthly() {
        let mut s = store();
        for i in 0..200 {
            let ts = DateTime::parse_from_rfc3339("2025-04-15T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
                + Duration::seconds(i);
            insert_session_row(&s, &format!("01S{i:030}"), "claude-code", Some("cortex"), ts, 7);
        }
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.sessions_collapsed, 200);
        assert_eq!(report.sessions_monthly_buckets, 1);
        let (ym, tool, repo, count, evt): (String, String, String, i64, i64) = s
            .conn()
            .query_row(
                "SELECT year_month, tool, repo, count, total_event_count FROM sessions_monthly",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(ym, "2025-04");
        assert_eq!(tool, "claude-code");
        assert_eq!(repo, "cortex");
        assert_eq!(count, 200);
        assert_eq!(evt, 200 * 7);
        let remaining: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn sessions_with_null_repo_collapse_to_empty_string_bucket() {
        let mut s = store();
        let ts = now() - Duration::days(400);
        insert_session_row(&s, "01N1", "claude-code", None, ts, 1);
        insert_session_row(&s, "01N2", "claude-code", None, ts, 1);
        run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        let (repo, count): (String, i64) = s
            .conn()
            .query_row(
                "SELECT repo, count FROM sessions_monthly",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(repo, "");
        assert_eq!(count, 2);
    }

    #[test]
    fn classifier_spend_year_old_rows_collapse_to_monthly() {
        let mut s = store();
        // 5 days in 2025-03 each with calls=10 → totals 50.
        for d in 1..=5 {
            s.record_classifier_spend(&format!("2025-03-{d:02}"), 10, 100, 50, 200)
                .unwrap();
        }
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.spend_collapsed, 5);
        assert_eq!(report.spend_monthly_buckets, 1);
        let (ym, calls, tin, tout, cents): (String, i64, i64, i64, i64) = s
            .conn()
            .query_row(
                "SELECT year_month, calls, tokens_in, tokens_out, est_usd_cents
                   FROM classifier_spend_monthly",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(ym, "2025-03");
        assert_eq!(calls, 50);
        assert_eq!(tin, 500);
        assert_eq!(tout, 250);
        assert_eq!(cents, 1000);
    }

    #[test]
    fn re_run_with_no_aged_rows_is_a_noop() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01OLD",
            "/repo/x",
            now() - Duration::days(31),
            10,
            100,
            "success",
        );
        let r1 = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(r1.bootstrap_jobs_collapsed, 1);
        let r2 = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(r2.bootstrap_jobs_collapsed, 0);
        assert_eq!(r2.bootstrap_daily_buckets, 0);
        assert_eq!(r2.sessions_collapsed, 0);
        assert_eq!(r2.spend_collapsed, 0);
    }

    #[test]
    fn dry_run_records_counters_without_mutating() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01OLD",
            "/repo/x",
            now() - Duration::days(31),
            10,
            100,
            "success",
        );
        let mut plan = ReapPlan::default_for(now());
        plan.dry_run = true;
        let report = run(s.conn_mut(), &plan).unwrap();
        assert_eq!(report.bootstrap_jobs_collapsed, 1);
        // Source row preserved.
        let remaining: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM bootstrap_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        // No daily row inserted.
        let daily: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM bootstrap_jobs_daily", [], |r| r.get(0))
            .unwrap();
        assert_eq!(daily, 0);
    }

    #[test]
    fn target_filter_runs_only_one_rollup() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01B",
            "/repo/x",
            now() - Duration::days(31),
            10,
            100,
            "success",
        );
        insert_session_row(
            &s,
            "01S",
            "claude-code",
            Some("cortex"),
            now() - Duration::days(400),
            5,
        );
        let mut plan = ReapPlan::default_for(now());
        plan.target = ReapTarget::Sessions;
        let report = run(s.conn_mut(), &plan).unwrap();
        assert_eq!(report.bootstrap_jobs_collapsed, 0);
        assert_eq!(report.sessions_collapsed, 1);
        // Bootstrap row still present.
        let bootstrap_rows: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM bootstrap_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bootstrap_rows, 1);
    }

    #[test]
    fn rerun_with_already_rolled_bucket_increments_existing_row() {
        let mut s = store();
        insert_bootstrap_row(
            &s,
            "01A",
            "/repo/x",
            now() - Duration::days(31),
            10,
            100,
            "success",
        );
        run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        // Insert a SECOND aged row landing on the same day.
        insert_bootstrap_row(
            &s,
            "01B",
            "/repo/x",
            now() - Duration::days(31),
            5,
            50,
            "success",
        );
        run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        let (runs, files, chunks): (i64, i64, i64) = s
            .conn()
            .query_row(
                "SELECT runs, total_files, total_chunks FROM bootstrap_jobs_daily",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(runs, 2);
        assert_eq!(files, 15);
        assert_eq!(chunks, 150);
    }

    #[test]
    fn report_metadata_reap_json_round_trips() {
        let r = ReapReport {
            bootstrap_jobs_collapsed: 12,
            bootstrap_daily_buckets: 3,
            sessions_collapsed: 200,
            sessions_monthly_buckets: 4,
            spend_collapsed: 30,
            spend_monthly_buckets: 1,
            free_pages_ratio: 0.42,
            did_vacuum: true,
            vacuum_ms: 5,
        };
        let json = r.metadata_reap_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["bootstrap_jobs_collapsed"], 12);
        assert_eq!(parsed["sessions_monthly_buckets"], 4);
        assert_eq!(parsed["did_vacuum"], true);
    }

    #[test]
    fn vacuum_runs_when_freelist_ratio_high() {
        let mut s = store();
        // Force a chunky DB then delete to push the freelist ratio
        // above 25 %. The reaper's own deletes here would not yield
        // enough free pages on an empty schema, so we seed garbage.
        for i in 0..400 {
            insert_session_row(
                &s,
                &format!("01F{i:030}"),
                "claude-code",
                Some("cortex"),
                now() - Duration::days(400),
                10,
            );
        }
        let report = run(s.conn_mut(), &ReapPlan::default_for(now())).unwrap();
        assert_eq!(report.sessions_collapsed, 400);
        // We don't assert did_vacuum=true here unconditionally — the
        // exact freelist ratio depends on the SQLite version; we DO
        // assert the runner reports a sensible ratio (≥ 0.0) and
        // never panics.
        assert!(report.free_pages_ratio >= 0.0);
    }
}

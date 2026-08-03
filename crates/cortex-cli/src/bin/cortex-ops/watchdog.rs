//! phase0 §5 — `cortex-ops watchdog` coverage / health alarm.
//!
//! The `/v1/health/coverage` endpoint surfaces the archive-watcher and
//! pruner signals as *data*, but its `overall_severity` only reflects
//! the backend inventory diff (collections present / missing) — a blind
//! watcher, a stalled ingest flush, or a sweep that never ran do NOT
//! raise it. This command closes that gap: it reads the same signals and
//! raises explicit alarms when coverage degrades, returning a non-zero
//! exit code (`1` warn / `2` critical) the retention daemon's cron tick
//! records as `last_status`. Run it on a cadence via the `health.watchdog`
//! seed job so silent ingestion / retention failures surface on their own.
//!
//! The alarm logic ([`evaluate`]) is a pure function over [`WatchdogSignals`]
//! so it is fully unit-testable; the I/O wrapper ([`watchdog`]) is
//! best-effort — an unreachable watcher or unreadable status file degrades
//! to an alarm, never a panic.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::helpers::home_dir;

/// Staleness budgets (seconds). Defaults: ingest flush 1 h, retention
/// sweep ~25 h (daily cron + slack), consolidation 48 h (nightly + slack).
#[derive(Debug, Clone, Copy)]
pub(super) struct WatchdogThresholds {
    pub ingest_staleness_secs: i64,
    pub sweep_staleness_secs: i64,
    pub consolidation_staleness_secs: i64,
}

impl Default for WatchdogThresholds {
    fn default() -> Self {
        Self {
            ingest_staleness_secs: 3_600,
            sweep_staleness_secs: 90_000,
            consolidation_staleness_secs: 172_800,
        }
    }
}

/// Pure input to [`evaluate`]. All timestamps are epoch milliseconds;
/// `0` / `None` means "never observed".
#[derive(Debug, Clone, Default)]
pub(super) struct WatchdogSignals {
    pub now_ms: i64,
    /// `false` when the watcher `/healthz` probe failed.
    pub watcher_reachable: bool,
    /// `.jsonl` files the watcher's most recent walk surfaced.
    pub files_watched: usize,
    /// Epoch ms of the watcher's most recent emitter flush (`0` = none).
    pub last_flush_ms: i64,
    /// Epoch ms of the most recent finished retention sweep.
    pub last_sweep_ms: Option<i64>,
    /// Epoch ms of the most recent consolidation/pruner run.
    pub last_consolidation_ms: Option<i64>,
}

/// Alarm severity. Ordering matters: the report severity is the max.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WatchdogSeverity {
    Ok,
    Warn,
    Critical,
}

impl WatchdogSeverity {
    /// Process exit code: `0` ok, `1` warn, `2` critical — mirrors
    /// `cortex_api::health::coverage::CoverageSeverity::exit_code`.
    pub fn exit_code(self) -> u8 {
        match self {
            WatchdogSeverity::Ok => 0,
            WatchdogSeverity::Warn => 1,
            WatchdogSeverity::Critical => 2,
        }
    }
}

/// One raised alarm.
#[derive(Debug, Clone, Serialize)]
pub(super) struct WatchdogAlarm {
    pub code: &'static str,
    pub severity: WatchdogSeverity,
    pub detail: String,
}

/// The watchdog verdict.
#[derive(Debug, Clone, Serialize)]
pub(super) struct WatchdogReport {
    pub severity: WatchdogSeverity,
    pub alarms: Vec<WatchdogAlarm>,
}

fn age_secs(now_ms: i64, ts_ms: i64) -> i64 {
    (now_ms - ts_ms) / 1_000
}

/// Pure alarm evaluation. No I/O — every decision is a function of
/// `sig` + `th` so the matrix is unit-tested below.
pub(super) fn evaluate(sig: &WatchdogSignals, th: &WatchdogThresholds) -> WatchdogReport {
    let mut alarms: Vec<WatchdogAlarm> = Vec::new();

    // (a) archive watcher — blindness is the worst case.
    if !sig.watcher_reachable {
        alarms.push(WatchdogAlarm {
            code: "archive_watcher_unreachable",
            severity: WatchdogSeverity::Warn,
            detail: "watcher /healthz probe failed".to_string(),
        });
    } else if sig.files_watched == 0 {
        alarms.push(WatchdogAlarm {
            code: "archive_watcher_blind",
            severity: WatchdogSeverity::Critical,
            detail: "watcher healthy but files_watched == 0 (mount empty or unmounted)".to_string(),
        });
    } else if sig.last_flush_ms > 0 {
        let age = age_secs(sig.now_ms, sig.last_flush_ms);
        if age > th.ingest_staleness_secs {
            alarms.push(WatchdogAlarm {
                code: "ingest_stale",
                severity: WatchdogSeverity::Warn,
                detail: format!(
                    "no emitter flush in {age}s (budget {}s)",
                    th.ingest_staleness_secs
                ),
            });
        }
    }

    // (c1) retention sweep recency.
    match sig.last_sweep_ms {
        None => alarms.push(WatchdogAlarm {
            code: "sweep_missing",
            severity: WatchdogSeverity::Warn,
            detail: "no finished retention sweep on record".to_string(),
        }),
        Some(ts) => {
            let age = age_secs(sig.now_ms, ts);
            if age > th.sweep_staleness_secs {
                alarms.push(WatchdogAlarm {
                    code: "sweep_stale",
                    severity: WatchdogSeverity::Warn,
                    detail: format!(
                        "last retention sweep {age}s ago (budget {}s)",
                        th.sweep_staleness_secs
                    ),
                });
            }
        }
    }

    // (c2) consolidation / pruner recency.
    match sig.last_consolidation_ms {
        None => alarms.push(WatchdogAlarm {
            code: "consolidation_missing",
            severity: WatchdogSeverity::Warn,
            detail: "no consolidation/pruner run on record".to_string(),
        }),
        Some(ts) => {
            let age = age_secs(sig.now_ms, ts);
            if age > th.consolidation_staleness_secs {
                alarms.push(WatchdogAlarm {
                    code: "consolidation_stale",
                    severity: WatchdogSeverity::Warn,
                    detail: format!(
                        "last consolidation {age}s ago (budget {}s)",
                        th.consolidation_staleness_secs
                    ),
                });
            }
        }
    }

    let severity = alarms
        .iter()
        .map(|a| a.severity)
        .max()
        .unwrap_or(WatchdogSeverity::Ok);
    WatchdogReport { severity, alarms }
}

/// CLI entry. Resolves the live signals best-effort, evaluates, prints
/// (human or `--json`), and exits with the severity code.
#[allow(clippy::too_many_arguments)]
pub(super) fn watchdog(
    watcher_url: Option<String>,
    ingest_staleness_secs: Option<i64>,
    sweep_staleness_secs: Option<i64>,
    consolidation_staleness_secs: Option<i64>,
    home: Option<String>,
    json: bool,
) -> ExitCode {
    let defaults = WatchdogThresholds::default();
    let th = WatchdogThresholds {
        ingest_staleness_secs: ingest_staleness_secs.unwrap_or(defaults.ingest_staleness_secs),
        sweep_staleness_secs: sweep_staleness_secs.unwrap_or(defaults.sweep_staleness_secs),
        consolidation_staleness_secs: consolidation_staleness_secs
            .unwrap_or(defaults.consolidation_staleness_secs),
    };

    let now_ms = Utc::now().timestamp_millis();
    let (watcher_reachable, files_watched, last_flush_ms) = fetch_watcher(watcher_url.as_deref());
    let last_sweep_ms = latest_sweep_ms(home.as_deref());
    let last_consolidation_ms = latest_consolidation_ms(home.as_deref());

    let sig = WatchdogSignals {
        now_ms,
        watcher_reachable,
        files_watched,
        last_flush_ms,
        last_sweep_ms,
        last_consolidation_ms,
    };
    let report = evaluate(&sig, &th);

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("watchdog: serialize: {e}"),
        }
    } else if report.alarms.is_empty() {
        println!("watchdog: ok — no alarms");
    } else {
        for a in &report.alarms {
            println!("watchdog: {:?} [{}] {}", a.severity, a.code, a.detail);
        }
    }

    ExitCode::from(report.severity.exit_code())
}

/// Probe the archive watcher `/healthz`. Returns
/// `(reachable, files_watched, last_flush_ms)`; on any transport/parse
/// failure returns `(false, 0, 0)` so [`evaluate`] raises the
/// unreachable alarm.
fn fetch_watcher(url: Option<&str>) -> (bool, usize, i64) {
    let base = url
        .map(str::to_string)
        .or_else(|| {
            // ADR-016 — read `CORTEX_ARCHIVE_WATCHER_URLS` through the
            // typed Config layer (comma-separated; take the first entry),
            // never `std::env::var` directly.
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.archive_watcher_urls)
                .and_then(|v| v.split(',').next().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "http://localhost:17030".to_string());
    let endpoint = format!("{}/healthz", base.trim_end_matches('/'));

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, 0, 0),
    };
    let resp = match client.get(&endpoint).send() {
        Ok(r) if r.status().is_success() => r,
        _ => return (false, 0, 0),
    };
    let body: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(_) => return (false, 0, 0),
    };
    let files_watched = body
        .get("files_watched")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let last_flush_ms = body
        .get("last_flush_ts")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    (true, files_watched, last_flush_ms)
}

/// Most-recent finished retention sweep, as epoch ms. `None` when the
/// metadata DB is absent / unreadable or no sweep has a parseable
/// `finished_at`.
fn latest_sweep_ms(home: Option<&str>) -> Option<i64> {
    let path = resolve_metadata_db(home)?;
    let store = cortex_storage::MetadataStore::open(&path).ok()?;
    let rows = store.list_recent_sweeps(50).ok()?;
    rows.iter()
        .filter_map(|r| r.finished_at.as_deref())
        .filter_map(parse_rfc3339_ms)
        .max()
}

/// Most-recent consolidation/pruner run, as epoch ms, from the
/// pruner-status file (`last_run_ts`, epoch ms). `None` when missing.
fn latest_consolidation_ms(home: Option<&str>) -> Option<i64> {
    let path = resolve_pruner_status(home)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("last_run_ts")
        .and_then(|x| x.as_i64())
        .filter(|ts| *ts > 0)
}

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

/// Resolve `metadata.sqlite`: explicit `--home` → config `metadata_db`
/// → `<home>/metadata.sqlite` → `<resolved home>/metadata.sqlite`.
fn resolve_metadata_db(cli_home: Option<&str>) -> Option<PathBuf> {
    if let Some(h) = cli_home.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(h).join("metadata.sqlite"));
    }
    if let Ok(cfg) = cortex_config::Config::load() {
        if let Some(p) = cfg.ingestion.metadata_db.filter(|s| !s.is_empty()) {
            return Some(PathBuf::from(p));
        }
        if let Some(h) = cfg.ingestion.home.filter(|s| !s.is_empty()) {
            return Some(PathBuf::from(h).join("metadata.sqlite"));
        }
    }
    home_dir().map(|h| h.join(".cortex").join("metadata.sqlite"))
}

/// Resolve the pruner-status file: explicit `--home` →
/// config `pruner_status_file` → `<home>/.cortex/pruner-status.json`.
fn resolve_pruner_status(cli_home: Option<&str>) -> Option<PathBuf> {
    if let Some(h) = cli_home.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(h).join(".cortex").join("pruner-status.json"));
    }
    if let Ok(cfg) = cortex_config::Config::load() {
        if let Some(p) = cfg.ingestion.pruner_status_file.filter(|s| !s.is_empty()) {
            return Some(PathBuf::from(p));
        }
    }
    home_dir().map(|h| h.join(".cortex").join("pruner-status.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000; // fixed epoch ms

    fn healthy_signals() -> WatchdogSignals {
        WatchdogSignals {
            now_ms: NOW,
            watcher_reachable: true,
            files_watched: 203,
            last_flush_ms: NOW - 60_000,          // 60 s ago
            last_sweep_ms: Some(NOW - 3_600_000), // 1 h ago
            last_consolidation_ms: Some(NOW - 3_600_000),
        }
    }

    #[test]
    fn all_fresh_is_ok() {
        let r = evaluate(&healthy_signals(), &WatchdogThresholds::default());
        assert_eq!(r.severity, WatchdogSeverity::Ok);
        assert!(r.alarms.is_empty());
    }

    #[test]
    fn files_watched_zero_is_critical() {
        let mut s = healthy_signals();
        s.files_watched = 0;
        let r = evaluate(&s, &WatchdogThresholds::default());
        assert_eq!(r.severity, WatchdogSeverity::Critical);
        assert_eq!(r.alarms[0].code, "archive_watcher_blind");
    }

    #[test]
    fn unreachable_watcher_is_warn_not_blind() {
        let mut s = healthy_signals();
        s.watcher_reachable = false;
        s.files_watched = 0;
        let r = evaluate(&s, &WatchdogThresholds::default());
        // unreachable takes precedence over the blind check
        assert_eq!(r.severity, WatchdogSeverity::Warn);
        assert_eq!(r.alarms[0].code, "archive_watcher_unreachable");
    }

    #[test]
    fn stale_ingest_flush_warns() {
        let mut s = healthy_signals();
        s.last_flush_ms = NOW - 7_200_000; // 2 h ago, budget 1 h
        let r = evaluate(&s, &WatchdogThresholds::default());
        assert_eq!(r.severity, WatchdogSeverity::Warn);
        assert!(r.alarms.iter().any(|a| a.code == "ingest_stale"));
    }

    #[test]
    fn missing_sweep_and_consolidation_warn() {
        let mut s = healthy_signals();
        s.last_sweep_ms = None;
        s.last_consolidation_ms = None;
        let r = evaluate(&s, &WatchdogThresholds::default());
        assert_eq!(r.severity, WatchdogSeverity::Warn);
        assert!(r.alarms.iter().any(|a| a.code == "sweep_missing"));
        assert!(r.alarms.iter().any(|a| a.code == "consolidation_missing"));
    }

    #[test]
    fn stale_sweep_warns() {
        let mut s = healthy_signals();
        s.last_sweep_ms = Some(NOW - 100_000_000); // > 90_000 s budget
        let r = evaluate(&s, &WatchdogThresholds::default());
        assert!(r.alarms.iter().any(|a| a.code == "sweep_stale"));
    }

    #[test]
    fn critical_outranks_warn_in_severity() {
        let mut s = healthy_signals();
        s.files_watched = 0; // critical
        s.last_sweep_ms = None; // warn
        let r = evaluate(&s, &WatchdogThresholds::default());
        assert_eq!(r.severity, WatchdogSeverity::Critical);
        assert!(r.alarms.len() >= 2);
    }

    #[test]
    fn parse_rfc3339_ms_round_trips() {
        let ms = parse_rfc3339_ms("2027-01-15T00:00:00Z").unwrap();
        assert_eq!(
            DateTime::parse_from_rfc3339("2027-01-15T00:00:00Z")
                .unwrap()
                .timestamp_millis(),
            ms
        );
        assert!(parse_rfc3339_ms("garbage").is_none());
    }
}

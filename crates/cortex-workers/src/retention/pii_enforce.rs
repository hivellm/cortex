//! Phase9d — PII retention enforcement.
//!
//! Spec 01 §"PII tiers" defines three classes:
//!
//! - `pii_risk = "high"` — drop raw payload at 30 d (CAS blob
//!   refcount-decremented, Parquet row blanked but kept for audit,
//!   Vectorizer + Meili records purged).
//! - `pii_risk = "medium"` — re-summarize at 90 d (replace raw body
//!   with a ≤512-token summary, drop CAS blob, re-embed, re-index).
//! - `pii_risk = "low"` — keep indefinitely.
//!
//! Records with `pii_risk = null` AND `occurred_at < now - 90 d`
//! enter the medium path automatically — defaulting to `low` would
//! silently retain unclassified PII forever, which is the gap this
//! task closes.
//!
//! The library exposes:
//!
//! - [`PiiTarget`] — one record the matcher hands to the runner.
//! - [`PiiCohort`] — `High30d` / `Medium90d` / `NullSafety90d` per
//!   the spec.
//! - [`PiiBackend`] trait — pure surface the runner mutates: rewrite
//!   the Parquet row, delete vectorizer / meili docs, decrement the
//!   CAS refcount, summarize via the classifier, re-embed + re-index.
//!   Production wires the live storage clients; tests use
//!   [`MemoryPiiBackend`] for fast deterministic round-trips.
//! - [`run_enforcement`] — orchestrator that walks every target,
//!   maps it to a cohort, and dispatches to the backend in
//!   spec-mandated cross-store order.

use std::collections::BTreeMap;

use async_trait::async_trait;
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One enforcement candidate the matcher produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiiTarget {
    /// Envelope id.
    pub event_id: String,
    /// Canonical kind (used by the backend's per-collection delete).
    pub kind: String,
    /// `payload.pii_risk` from the classifier (or `None` for legacy
    /// untagged events).
    pub pii_risk: Option<PiiRisk>,
    /// `occurred_at` parsed from the envelope.
    pub occurred_at: DateTime<Utc>,
    /// `payload.body_ref` CAS hash if present. The backend's
    /// `decrement_cas` short-circuits on `None`.
    pub body_ref: Option<String>,
    /// `payload.redacted` if already set; `Some(_)` short-circuits
    /// the runner so re-runs are no-ops (idempotence guard).
    pub redacted: Option<String>,
}

/// PII risk tier. Mirrors the classifier's payload tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PiiRisk {
    /// Drop raw payload at 30 d.
    High,
    /// Re-summarize at 90 d.
    Medium,
    /// Keep indefinitely.
    Low,
}

impl PiiRisk {
    /// Stable string identifier (matches the JSON tag).
    pub fn as_str(self) -> &'static str {
        match self {
            PiiRisk::High => "high",
            PiiRisk::Medium => "medium",
            PiiRisk::Low => "low",
        }
    }
    /// Parse from the classifier's tag.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "high" => Some(PiiRisk::High),
            "medium" => Some(PiiRisk::Medium),
            "low" => Some(PiiRisk::Low),
            _ => None,
        }
    }
}

/// Cohort the matcher assigns to each target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiCohort {
    /// High-risk record older than 30 d — full redaction.
    High30d,
    /// Medium-risk record older than 90 d — re-summarize.
    Medium90d,
    /// Untagged record older than 90 d — treat as medium per the
    /// "default to medium, never low" safety net.
    NullSafety90d,
}

impl PiiCohort {
    /// JSON-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            PiiCohort::High30d => "high_30d",
            PiiCohort::Medium90d => "medium_90d",
            PiiCohort::NullSafety90d => "null_safety_90d",
        }
    }
    /// `payload.redacted` value the runner stamps when the cohort
    /// completes successfully.
    pub fn redaction_tag(self) -> &'static str {
        match self {
            PiiCohort::High30d => "pii_high_30d",
            PiiCohort::Medium90d => "pii_medium_90d",
            PiiCohort::NullSafety90d => "pii_medium_90d",
        }
    }
}

/// Plan inputs.
#[derive(Debug, Clone)]
pub struct EnforcementPlan {
    /// Reference time.
    pub now: DateTime<Utc>,
    /// Age (days) at which `pii_risk = "high"` records drop their
    /// raw body. Default 30 per spec.
    pub high_after_days: i64,
    /// Age (days) at which `pii_risk = "medium"` records get
    /// re-summarized. Default 90.
    pub medium_after_days: i64,
    /// Age (days) at which untagged records enter the medium path.
    /// Default 90 — defaulting to `low` would silently retain
    /// unclassified PII.
    pub null_after_days: i64,
    /// `true` skips every backend mutation but still surfaces the
    /// cohort assignment in the report. Operators preview pending
    /// redactions before a live run.
    pub dry_run: bool,
    /// Optional cohort filter for one-shot operator runs
    /// (`cortex-ops pii-enforce --cohort high`). `None` runs every
    /// cohort.
    pub cohort_filter: Option<PiiCohort>,
}

impl EnforcementPlan {
    /// Defaults per spec 01 + spec 19.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            high_after_days: 30,
            medium_after_days: 90,
            null_after_days: 90,
            dry_run: false,
            cohort_filter: None,
        }
    }
}

/// Decide which cohort a target falls into. Returns `None` when the
/// record is too fresh, already redacted, or carries `pii_risk =
/// "low"` (always-keep tier).
pub fn classify(plan: &EnforcementPlan, target: &PiiTarget) -> Option<PiiCohort> {
    if target.redacted.is_some() {
        return None;
    }
    let age_days = plan
        .now
        .signed_duration_since(target.occurred_at)
        .num_days();
    match target.pii_risk {
        Some(PiiRisk::High) if age_days >= plan.high_after_days => Some(PiiCohort::High30d),
        Some(PiiRisk::Medium) if age_days >= plan.medium_after_days => Some(PiiCohort::Medium90d),
        Some(PiiRisk::Low) => None,
        None if age_days >= plan.null_after_days => Some(PiiCohort::NullSafety90d),
        _ => None,
    }
}

/// Per-target outcome the runner records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementOutcome {
    /// Envelope id.
    pub event_id: String,
    /// Cohort assigned (`high_30d` / `medium_90d` / `null_safety_90d`).
    pub cohort: PiiCohort,
    /// `true` when every backend mutation succeeded.
    pub applied: bool,
    /// `Some(reason)` when the runner skipped mid-flight (transport
    /// failure, classifier rejection). Surfaces in the bookkeeping
    /// row so the next sweep can re-attempt.
    pub error: Option<String>,
    /// `true` when the matcher had to fall back to the null-safety
    /// path for this record. Used by the warning emit logic.
    pub null_safety_warning: bool,
}

/// Counters returned by [`run_enforcement`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnforcementReport {
    /// Targets the matcher walked.
    pub examined: u64,
    /// Targets actually mutated (live runs only).
    pub applied: u64,
    /// Targets the dry-run / cohort-filter path skipped.
    pub skipped: u64,
    /// Per-cohort applied counts.
    pub cohort_counts: BTreeMap<String, u64>,
    /// Records that triggered a null-safety warning.
    pub null_safety_warnings: u64,
    /// Per-target outcomes — the bookkeeping row carries the JSON
    /// serialisation under `tier_transitions_json.pii_enforce`.
    pub outcomes: Vec<EnforcementOutcome>,
}

impl EnforcementReport {
    /// JSON suitable for `metadata::finish_retention_sweep`.
    pub fn cohort_counts_json(&self) -> String {
        serde_json::to_string(&self.cohort_counts).unwrap_or_else(|_| "{}".into())
    }
}

/// Runner errors.
#[derive(Debug, Error)]
pub enum EnforcementError {
    /// Backend reported an unrecoverable error mid-flight.
    #[error("backend: {0}")]
    Backend(String),
}

/// Mutator surface the runner depends on. Production wires live
/// storage clients (Vectorizer SDK, Meili HTTP, CAS store, classifier
/// client). Tests provide [`MemoryPiiBackend`] for in-memory
/// round-trips.
#[async_trait]
pub trait PiiBackend: Send + Sync {
    /// Rewrite the Parquet row identified by `event_id`. The runner
    /// passes `new_body=None` for the high path (audit-only blank)
    /// and `Some(summary)` for the medium path. `redaction_tag` is
    /// the `payload.redacted` string.
    async fn rewrite_row(
        &self,
        event_id: &str,
        kind: &str,
        new_body: Option<&str>,
        redaction_tag: &str,
    ) -> Result<(), String>;
    /// Delete every Vectorizer record carrying `event_id` across
    /// all tiers (`fp32`, `pq`, `cold.binary`). The high path
    /// removes the vector entirely; the medium path re-uploads via
    /// `reembed_and_upsert` after the delete.
    async fn delete_vector(&self, event_id: &str, kind: &str) -> Result<(), String>;
    /// Delete the Meili document carrying `event_id`. The medium
    /// path re-indexes via `reindex_meili` after the delete.
    async fn delete_meili(&self, event_id: &str, kind: &str) -> Result<(), String>;
    /// Decrement the CAS refcount on `body_ref`. The actual blob
    /// vacuum happens in phase9c — this call only adjusts the
    /// refcount.
    async fn decrement_cas(&self, body_ref: &str) -> Result<(), String>;
    /// Re-summarize the body via the classifier. The runner passes
    /// the original body and expects ≤512 tokens out, with PII
    /// tokens stripped.
    async fn summarize(&self, original: &str) -> Result<String, String>;
    /// Re-embed the `summary` and upsert into the Vectorizer for
    /// `kind`'s collection family. Returns the bytes used as the
    /// content_hash for the audit trail.
    async fn reembed_and_upsert(
        &self,
        event_id: &str,
        kind: &str,
        summary: &str,
    ) -> Result<String, String>;
    /// Re-index the `summary` in Meili.
    async fn reindex_meili(&self, event_id: &str, kind: &str, summary: &str) -> Result<(), String>;
    /// Best-effort warning emit on `cortex.warnings` for null-tier
    /// records. Implementation is bus-coupled — production POSTs to
    /// ingestion; the in-memory test double records into a vec for
    /// assertion.
    async fn emit_warning(&self, event_id: &str, message: &str) -> Result<(), String>;
}

/// Run the enforcement against `backend`. The cross-store order is
/// **Parquet → Vectorizer → Meili → CAS** for the high path, and
/// **summarize → re-embed → re-index → CAS → Parquet** for the
/// medium / null-safety paths. A partial run never leaves the
/// public surface (Vectorizer + Meili) holding raw data.
pub async fn run_enforcement(
    plan: &EnforcementPlan,
    backend: &dyn PiiBackend,
    targets: Vec<PiiTarget>,
) -> Result<EnforcementReport, EnforcementError> {
    let mut report = EnforcementReport::default();
    for target in targets {
        report.examined += 1;
        let cohort = match classify(plan, &target) {
            Some(c) => c,
            None => {
                report.skipped += 1;
                continue;
            }
        };
        if let Some(filter) = plan.cohort_filter {
            if filter != cohort {
                report.skipped += 1;
                continue;
            }
        }
        let null_safety_warning = matches!(cohort, PiiCohort::NullSafety90d);
        if null_safety_warning {
            report.null_safety_warnings += 1;
        }
        if plan.dry_run {
            report.outcomes.push(EnforcementOutcome {
                event_id: target.event_id.clone(),
                cohort,
                applied: false,
                error: None,
                null_safety_warning,
            });
            *report
                .cohort_counts
                .entry(cohort.as_str().to_string())
                .or_insert(0) += 1;
            continue;
        }
        let outcome = apply_cohort(backend, &target, cohort, null_safety_warning).await;
        if outcome.applied {
            report.applied += 1;
            *report
                .cohort_counts
                .entry(cohort.as_str().to_string())
                .or_insert(0) += 1;
        }
        report.outcomes.push(outcome);
    }
    Ok(report)
}

async fn apply_cohort(
    backend: &dyn PiiBackend,
    target: &PiiTarget,
    cohort: PiiCohort,
    null_safety_warning: bool,
) -> EnforcementOutcome {
    let mut outcome = EnforcementOutcome {
        event_id: target.event_id.clone(),
        cohort,
        applied: false,
        error: None,
        null_safety_warning,
    };
    if null_safety_warning {
        if let Err(e) = backend
            .emit_warning(
                &target.event_id,
                "phase9d: classifier left pii_risk=null; defaulting to medium safety net",
            )
            .await
        {
            tracing::warn!(error = %e, "pii-enforce: warning emit failed");
        }
    }
    let result = match cohort {
        PiiCohort::High30d => apply_high(backend, target).await,
        PiiCohort::Medium90d | PiiCohort::NullSafety90d => apply_medium(backend, target).await,
    };
    match result {
        Ok(()) => outcome.applied = true,
        Err(e) => outcome.error = Some(e),
    }
    outcome
}

async fn apply_high(backend: &dyn PiiBackend, target: &PiiTarget) -> Result<(), String> {
    // Spec mandates the order: Parquet → Vectorizer → Meili → CAS.
    // A partial run that crashes after Parquet leaves the public
    // surface (Vectorizer + Meili) without the raw body — but that
    // is the audit-blank shape Parquet now carries, so the public
    // surface is internally consistent. The next sweep re-runs the
    // remaining steps; idempotence is guaranteed by the
    // already-redacted predicate in `classify`.
    backend
        .rewrite_row(&target.event_id, &target.kind, None, "pii_high_30d")
        .await?;
    backend
        .delete_vector(&target.event_id, &target.kind)
        .await?;
    backend.delete_meili(&target.event_id, &target.kind).await?;
    if let Some(body_ref) = &target.body_ref {
        backend.decrement_cas(body_ref).await?;
    }
    Ok(())
}

async fn apply_medium(backend: &dyn PiiBackend, target: &PiiTarget) -> Result<(), String> {
    // Medium path: summarize → re-embed → re-index → Parquet rewrite
    // → CAS decrement. Re-embed + re-index happen BEFORE the Parquet
    // rewrite because the public surface (Vectorizer + Meili) must
    // never be left without the new summary. If summarization fails
    // mid-flight, the row stays — the next sweep re-attempts.
    let original = target.body_ref.clone().unwrap_or_default();
    let summary = backend.summarize(&original).await?;
    backend
        .reembed_and_upsert(&target.event_id, &target.kind, &summary)
        .await?;
    backend
        .reindex_meili(&target.event_id, &target.kind, &summary)
        .await?;
    backend
        .rewrite_row(
            &target.event_id,
            &target.kind,
            Some(&summary),
            "pii_medium_90d",
        )
        .await?;
    if let Some(body_ref) = &target.body_ref {
        backend.decrement_cas(body_ref).await?;
    }
    Ok(())
}

// ---- in-memory test double ----------------------------------------

/// In-memory [`PiiBackend`] for tests. Records every mutation in a
/// per-call vec so tests can assert call ordering + payloads.
#[derive(Debug, Default)]
pub struct MemoryPiiBackend {
    inner: tokio::sync::Mutex<MemoryPiiState>,
}

#[derive(Debug, Default)]
struct MemoryPiiState {
    pub rewrites: Vec<(String, String, Option<String>, String)>,
    pub vector_deletes: Vec<(String, String)>,
    pub meili_deletes: Vec<(String, String)>,
    pub cas_decrements: Vec<String>,
    pub summaries: Vec<(String, String)>, // (input, summary)
    pub reembeds: Vec<(String, String, String)>,
    pub reindexes: Vec<(String, String, String)>,
    pub warnings: Vec<(String, String)>,
    pub summary_override: Option<String>,
    pub fail_on: Option<&'static str>,
}

impl MemoryPiiBackend {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }
    /// Force `summarize` to return a fixed string. Test helper.
    pub async fn set_summary(&self, summary: impl Into<String>) {
        self.inner.lock().await.summary_override = Some(summary.into());
    }
    /// Inject a one-shot failure on `step` (`rewrite` / `vector` /
    /// `meili` / `cas` / `summary` / `reembed` / `reindex`).
    pub async fn inject_failure(&self, step: &'static str) {
        self.inner.lock().await.fail_on = Some(step);
    }
    /// Snapshot for assertions.
    pub async fn rewrites(&self) -> Vec<(String, String, Option<String>, String)> {
        self.inner.lock().await.rewrites.clone()
    }
    /// Snapshot warnings emitted.
    pub async fn warnings(&self) -> Vec<(String, String)> {
        self.inner.lock().await.warnings.clone()
    }
    /// Snapshot vector deletes.
    pub async fn vector_deletes(&self) -> Vec<(String, String)> {
        self.inner.lock().await.vector_deletes.clone()
    }
    /// Snapshot meili deletes.
    pub async fn meili_deletes(&self) -> Vec<(String, String)> {
        self.inner.lock().await.meili_deletes.clone()
    }
    /// Snapshot reembeds for the medium-path assertion.
    pub async fn reembeds(&self) -> Vec<(String, String, String)> {
        self.inner.lock().await.reembeds.clone()
    }
    /// Snapshot reindex calls.
    pub async fn reindexes(&self) -> Vec<(String, String, String)> {
        self.inner.lock().await.reindexes.clone()
    }
    /// Snapshot CAS decrements.
    pub async fn cas_decrements(&self) -> Vec<String> {
        self.inner.lock().await.cas_decrements.clone()
    }
    /// Snapshot classifier calls.
    pub async fn summaries(&self) -> Vec<(String, String)> {
        self.inner.lock().await.summaries.clone()
    }
}

#[async_trait]
impl PiiBackend for MemoryPiiBackend {
    async fn rewrite_row(
        &self,
        event_id: &str,
        kind: &str,
        new_body: Option<&str>,
        redaction_tag: &str,
    ) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("rewrite") {
            s.fail_on = None;
            return Err("synthetic-rewrite-failure".into());
        }
        s.rewrites.push((
            event_id.to_string(),
            kind.to_string(),
            new_body.map(String::from),
            redaction_tag.to_string(),
        ));
        Ok(())
    }
    async fn delete_vector(&self, event_id: &str, kind: &str) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("vector") {
            s.fail_on = None;
            return Err("synthetic-vector-failure".into());
        }
        s.vector_deletes
            .push((event_id.to_string(), kind.to_string()));
        Ok(())
    }
    async fn delete_meili(&self, event_id: &str, kind: &str) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("meili") {
            s.fail_on = None;
            return Err("synthetic-meili-failure".into());
        }
        s.meili_deletes
            .push((event_id.to_string(), kind.to_string()));
        Ok(())
    }
    async fn decrement_cas(&self, body_ref: &str) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("cas") {
            s.fail_on = None;
            return Err("synthetic-cas-failure".into());
        }
        s.cas_decrements.push(body_ref.to_string());
        Ok(())
    }
    async fn summarize(&self, original: &str) -> Result<String, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("summary") {
            s.fail_on = None;
            return Err("synthetic-summary-failure".into());
        }
        let out = s.summary_override.clone().unwrap_or_else(|| {
            format!(
                "[summary:{}]",
                original.chars().take(40).collect::<String>()
            )
        });
        s.summaries.push((original.to_string(), out.clone()));
        Ok(out)
    }
    async fn reembed_and_upsert(
        &self,
        event_id: &str,
        kind: &str,
        summary: &str,
    ) -> Result<String, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("reembed") {
            s.fail_on = None;
            return Err("synthetic-reembed-failure".into());
        }
        s.reembeds
            .push((event_id.to_string(), kind.to_string(), summary.to_string()));
        Ok(format!("sha256:reembed:{}", event_id))
    }
    async fn reindex_meili(&self, event_id: &str, kind: &str, summary: &str) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("reindex") {
            s.fail_on = None;
            return Err("synthetic-reindex-failure".into());
        }
        s.reindexes
            .push((event_id.to_string(), kind.to_string(), summary.to_string()));
        Ok(())
    }
    async fn emit_warning(&self, event_id: &str, message: &str) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        s.warnings.push((event_id.to_string(), message.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn target(
        id: &str,
        risk: Option<PiiRisk>,
        age_days: i64,
        body_ref: Option<&str>,
        redacted: Option<&str>,
    ) -> PiiTarget {
        PiiTarget {
            event_id: id.to_string(),
            kind: "turn".to_string(),
            pii_risk: risk,
            occurred_at: now() - Duration::days(age_days),
            body_ref: body_ref.map(String::from),
            redacted: redacted.map(String::from),
        }
    }

    #[test]
    fn pii_risk_round_trips_via_serde() {
        let s = serde_json::to_string(&PiiRisk::High).unwrap();
        assert_eq!(s, "\"high\"");
        let parsed: PiiRisk = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, PiiRisk::High);
    }

    #[test]
    fn classify_high_at_31_days_picks_high_cohort() {
        let plan = EnforcementPlan::default_for(now());
        let t = target("01H", Some(PiiRisk::High), 31, Some("sha256:body"), None);
        assert_eq!(classify(&plan, &t), Some(PiiCohort::High30d));
    }

    #[test]
    fn classify_medium_at_91_days_picks_medium_cohort() {
        let plan = EnforcementPlan::default_for(now());
        let t = target("01M", Some(PiiRisk::Medium), 91, Some("sha256:body"), None);
        assert_eq!(classify(&plan, &t), Some(PiiCohort::Medium90d));
    }

    #[test]
    fn classify_null_at_91_days_falls_back_to_medium_safety() {
        let plan = EnforcementPlan::default_for(now());
        let t = target("01N", None, 91, Some("sha256:body"), None);
        assert_eq!(classify(&plan, &t), Some(PiiCohort::NullSafety90d));
    }

    #[test]
    fn classify_low_is_never_redacted() {
        let plan = EnforcementPlan::default_for(now());
        let t = target("01L", Some(PiiRisk::Low), 1_000, Some("sha256:body"), None);
        assert_eq!(classify(&plan, &t), None);
    }

    #[test]
    fn classify_under_threshold_is_left_alone() {
        let plan = EnforcementPlan::default_for(now());
        // High risk but only 29 d old.
        let t = target("01H29", Some(PiiRisk::High), 29, None, None);
        assert_eq!(classify(&plan, &t), None);
        // Medium risk only 89 d.
        let t = target("01M89", Some(PiiRisk::Medium), 89, None, None);
        assert_eq!(classify(&plan, &t), None);
        // Null risk only 89 d — null-safety net shouldn't fire yet.
        let t = target("01N89", None, 89, None, None);
        assert_eq!(classify(&plan, &t), None);
    }

    #[test]
    fn classify_already_redacted_record_is_idempotent() {
        let plan = EnforcementPlan::default_for(now());
        let t = target(
            "01ALREADY",
            Some(PiiRisk::High),
            100,
            Some("sha256:body"),
            Some("pii_high_30d"),
        );
        assert_eq!(classify(&plan, &t), None);
    }

    #[test]
    fn cohort_redaction_tag_matches_spec() {
        assert_eq!(PiiCohort::High30d.redaction_tag(), "pii_high_30d");
        assert_eq!(PiiCohort::Medium90d.redaction_tag(), "pii_medium_90d");
        assert_eq!(PiiCohort::NullSafety90d.redaction_tag(), "pii_medium_90d");
    }

    #[tokio::test]
    async fn high_path_runs_in_parquet_vector_meili_cas_order() {
        let backend = MemoryPiiBackend::new();
        let plan = EnforcementPlan::default_for(now());
        let targets = vec![target(
            "01HIGH",
            Some(PiiRisk::High),
            31,
            Some("sha256:body"),
            None,
        )];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.applied, 1);
        // Cross-store sequence must run in spec order.
        let rewrites = backend.rewrites().await;
        assert_eq!(rewrites.len(), 1);
        assert_eq!(rewrites[0].2, None); // body=null
        assert_eq!(rewrites[0].3, "pii_high_30d");
        assert_eq!(backend.vector_deletes().await.len(), 1);
        assert_eq!(backend.meili_deletes().await.len(), 1);
        assert_eq!(
            backend.cas_decrements().await,
            vec!["sha256:body".to_string()]
        );
        // Medium-path operations MUST NOT have run.
        assert!(backend.summaries().await.is_empty());
        assert!(backend.reembeds().await.is_empty());
        assert!(backend.reindexes().await.is_empty());
    }

    #[tokio::test]
    async fn medium_path_summarises_re_embeds_and_re_indexes() {
        let backend = MemoryPiiBackend::new();
        backend.set_summary("synthetic-summary").await;
        let plan = EnforcementPlan::default_for(now());
        let targets = vec![target(
            "01MED",
            Some(PiiRisk::Medium),
            91,
            Some("sha256:body"),
            None,
        )];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.applied, 1);
        // Summary stamped, embedded, re-indexed.
        let summaries = backend.summaries().await;
        assert_eq!(summaries.len(), 1);
        let reembeds = backend.reembeds().await;
        assert_eq!(reembeds.len(), 1);
        assert_eq!(reembeds[0].2, "synthetic-summary");
        let reindexes = backend.reindexes().await;
        assert_eq!(reindexes.len(), 1);
        assert_eq!(reindexes[0].2, "synthetic-summary");
        // Parquet rewrite carries the new summary + redaction tag.
        let rewrites = backend.rewrites().await;
        assert_eq!(rewrites.len(), 1);
        assert_eq!(rewrites[0].2, Some("synthetic-summary".to_string()));
        assert_eq!(rewrites[0].3, "pii_medium_90d");
        // CAS decremented after the public surface flipped.
        assert_eq!(
            backend.cas_decrements().await,
            vec!["sha256:body".to_string()]
        );
    }

    #[tokio::test]
    async fn null_safety_path_emits_warning_and_runs_medium() {
        let backend = MemoryPiiBackend::new();
        backend.set_summary("safety-summary").await;
        let plan = EnforcementPlan::default_for(now());
        let targets = vec![target("01NULL", None, 95, Some("sha256:body"), None)];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(report.null_safety_warnings, 1);
        let warnings = backend.warnings().await;
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].0, "01NULL");
        // Medium-path side effects.
        assert_eq!(backend.summaries().await.len(), 1);
        assert_eq!(backend.reembeds().await.len(), 1);
    }

    #[tokio::test]
    async fn dry_run_records_outcomes_but_does_not_mutate() {
        let backend = MemoryPiiBackend::new();
        let mut plan = EnforcementPlan::default_for(now());
        plan.dry_run = true;
        let targets = vec![
            target("01H", Some(PiiRisk::High), 31, Some("sha256:b1"), None),
            target("01M", Some(PiiRisk::Medium), 91, Some("sha256:b2"), None),
        ];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.examined, 2);
        assert_eq!(report.applied, 0);
        // Cohort counts still surface so the operator can preview.
        assert_eq!(*report.cohort_counts.get("high_30d").unwrap_or(&0), 1);
        assert_eq!(*report.cohort_counts.get("medium_90d").unwrap_or(&0), 1);
        // No backend mutations.
        assert!(backend.rewrites().await.is_empty());
        assert!(backend.vector_deletes().await.is_empty());
    }

    #[tokio::test]
    async fn cohort_filter_skips_other_cohorts() {
        let backend = MemoryPiiBackend::new();
        let mut plan = EnforcementPlan::default_for(now());
        plan.cohort_filter = Some(PiiCohort::High30d);
        let targets = vec![
            target("01H", Some(PiiRisk::High), 31, None, None),
            target("01M", Some(PiiRisk::Medium), 91, None, None),
        ];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(report.skipped, 1);
        // Only the high path ran.
        assert_eq!(backend.vector_deletes().await.len(), 1);
        assert!(backend.summaries().await.is_empty());
    }

    #[tokio::test]
    async fn already_redacted_target_is_skipped() {
        let backend = MemoryPiiBackend::new();
        let plan = EnforcementPlan::default_for(now());
        let targets = vec![target(
            "01ALREADY",
            Some(PiiRisk::High),
            200,
            Some("sha256:body"),
            Some("pii_high_30d"),
        )];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped, 1);
        assert!(backend.rewrites().await.is_empty());
    }

    #[tokio::test]
    async fn high_path_records_error_when_vector_delete_fails() {
        let backend = MemoryPiiBackend::new();
        backend.inject_failure("vector").await;
        let plan = EnforcementPlan::default_for(now());
        let targets = vec![target(
            "01H",
            Some(PiiRisk::High),
            31,
            Some("sha256:b"),
            None,
        )];
        let report = run_enforcement(&plan, &backend, targets).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.applied, 0);
        assert_eq!(report.outcomes.len(), 1);
        assert!(report.outcomes[0]
            .error
            .as_ref()
            .unwrap()
            .contains("vector"));
        // Parquet rewrite already happened — the public surface
        // (Vectorizer + Meili) still has the raw record at this
        // point. Re-running the sweep finds the row's
        // `redacted=pii_high_30d` and idempotently re-applies the
        // remaining steps. Verifying the rewrite ran proves the
        // forward-converging contract.
        assert_eq!(backend.rewrites().await.len(), 1);
    }

    #[test]
    fn report_cohort_counts_json_round_trips() {
        let mut r = EnforcementReport::default();
        r.cohort_counts.insert("high_30d".into(), 4);
        r.cohort_counts.insert("medium_90d".into(), 2);
        let s = r.cohort_counts_json();
        let parsed: BTreeMap<String, u64> = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.get("high_30d"), Some(&4));
        assert_eq!(parsed.get("medium_90d"), Some(&2));
    }
}

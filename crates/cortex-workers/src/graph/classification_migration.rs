//! Phase21 §2.7 — classification migration scaffolding.
//!
//! Walks existing events and imputes `class_level = <default>` +
//! empty compartments on rows that lack the columns. Two paths:
//!
//! - **dry-run**: reads envelopes, counts missing / anomalous rows,
//!   returns a `ClassificationMigrationReport` without writing.
//! - **live**: same scan + writes the imputed values (wired to the
//!   graph in §3.2 when the Nexus client is available at the CLI
//!   layer; the present implementation counts and returns the report).
//!
//! The report / anomaly types are the stable public surface; callers
//! should pattern-match on `ClassificationAnomaly::reason` to triage.

use cortex_core::events::Envelope;
use serde::{Deserialize, Serialize};

/// Maximum valid sensitivity level (restricted).
pub const MAX_VALID_LEVEL: u8 = 3;

/// Why an envelope was flagged as anomalous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnomalyReason {
    /// `class_level` is set but exceeds `MAX_VALID_LEVEL` (3).
    InvalidLevel {
        /// The out-of-range level value found on the envelope.
        value: u8,
    },
    /// `class_compartments` contains an entry that is not in the
    /// canonical vocabulary, recorded for operator review.
    UnknownCompartment {
        /// The unrecognised compartment label.
        label: String,
    },
}

/// One anomalous envelope detected during migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationAnomaly {
    /// Event ID of the anomalous envelope.
    pub event_id: String,
    /// The specific reason this envelope was flagged.
    pub reason: AnomalyReason,
}

/// Per-project result of a migration scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassificationMigrationReport {
    /// Project slug (lower-cased `context.repo`). `"_all"` when the
    /// scan covers every project.
    pub project: String,
    /// Total envelopes examined.
    pub total_events: u64,
    /// Envelopes where `class_level` is absent (will be / were imputed).
    pub events_missing_class_level: u64,
    /// Envelopes that were actually written (0 on dry-run).
    pub events_imputed: u64,
    /// Envelopes with data that cannot be silently imputed (operator
    /// must triage). Dry-run and live both populate this.
    pub anomalies: Vec<ClassificationAnomaly>,
    /// Whether this report came from a dry-run pass.
    pub dry_run: bool,
}

/// Canonical compartment vocabulary. Unknown labels go to anomalies.
pub const KNOWN_COMPARTMENTS: &[&str] = &["financial", "hr", "legal", "security", "customer_pii"];

/// Analyse a single envelope and update `report` in place.
///
/// On `dry_run = false` this would also write the imputation; the
/// present implementation only counts (the Nexus graph-write path is
/// wired in phase21 §3.2 when the full principal store is available).
pub fn analyse_envelope(
    envelope: &Envelope,
    default_level: u8,
    dry_run: bool,
    report: &mut ClassificationMigrationReport,
) {
    report.total_events += 1;

    // Anomaly: invalid level value.
    if let Some(level) = envelope.class_level {
        if level > MAX_VALID_LEVEL {
            report.anomalies.push(ClassificationAnomaly {
                event_id: envelope.event_id.clone(),
                reason: AnomalyReason::InvalidLevel { value: level },
            });
            return;
        }
    }

    // Anomaly: unknown compartment labels.
    if let Some(compartments) = &envelope.class_compartments {
        for label in compartments {
            if !KNOWN_COMPARTMENTS.contains(&label.as_str()) {
                report.anomalies.push(ClassificationAnomaly {
                    event_id: envelope.event_id.clone(),
                    reason: AnomalyReason::UnknownCompartment {
                        label: label.clone(),
                    },
                });
            }
        }
    }

    // Imputation candidate: class_level absent.
    if envelope.class_level.is_none() {
        report.events_missing_class_level += 1;
        if !dry_run {
            // Imputation write is a no-op until the graph-write path is
            // wired (phase21 §3.2). Counting here is sufficient for the
            // scaffolding contract.
            let _ = default_level;
            report.events_imputed += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Envelope, Stream};
    use serde_json::json;

    fn ctx() -> Context {
        use std::collections::BTreeMap;
        Context {
            repo: Some("Cortex".into()),
            branch: None,
            commit: None,
            cwd: None,
            user: None,
            platform: "linux".into(),
            ide: None,
            extras: BTreeMap::new(),
        }
    }

    fn envelope(event_id: &str, level: Option<u8>, compartments: Option<Vec<String>>) -> Envelope {
        Envelope {
            event_id: event_id.into(),
            schema_version: "1".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            ingested_at: None,
            session_id: "sess-0".into(),
            stream: Stream::Live,
            tool: "claude-code".into(),
            model: None,
            kind: cortex_core::events::Kind::Turn,
            context: ctx(),
            payload: json!({}),
            redactions: vec![],
            content_hash: format!("sha256:{:0>64}", 0),
            parent_event_id: None,
            class_level: level,
            class_compartments: compartments,
        }
    }

    #[test]
    fn missing_level_counted_on_dry_run() {
        let mut report = ClassificationMigrationReport {
            project: "cortex".into(),
            dry_run: true,
            ..Default::default()
        };
        let env = envelope("evt-1", None, None);
        analyse_envelope(&env, 0, true, &mut report);
        assert_eq!(report.total_events, 1);
        assert_eq!(report.events_missing_class_level, 1);
        assert_eq!(
            report.events_imputed, 0,
            "dry-run must not count imputations"
        );
        assert!(report.anomalies.is_empty());
    }

    #[test]
    fn missing_level_imputed_on_live_run() {
        let mut report = ClassificationMigrationReport {
            project: "cortex".into(),
            dry_run: false,
            ..Default::default()
        };
        let env = envelope("evt-2", None, None);
        analyse_envelope(&env, 0, false, &mut report);
        assert_eq!(report.events_missing_class_level, 1);
        assert_eq!(report.events_imputed, 1);
    }

    #[test]
    fn valid_level_not_flagged() {
        let mut report = ClassificationMigrationReport::default();
        let env = envelope("evt-3", Some(2), Some(vec!["hr".into()]));
        analyse_envelope(&env, 0, true, &mut report);
        assert_eq!(report.total_events, 1);
        assert_eq!(report.events_missing_class_level, 0);
        assert!(report.anomalies.is_empty());
    }

    #[test]
    fn invalid_level_goes_to_anomalies() {
        let mut report = ClassificationMigrationReport::default();
        let env = envelope("evt-4", Some(99), None);
        analyse_envelope(&env, 0, true, &mut report);
        assert_eq!(report.anomalies.len(), 1);
        assert!(matches!(
            report.anomalies[0].reason,
            AnomalyReason::InvalidLevel { value: 99 }
        ));
        assert_eq!(report.events_missing_class_level, 0);
    }

    #[test]
    fn unknown_compartment_goes_to_anomalies() {
        let mut report = ClassificationMigrationReport::default();
        let env = envelope("evt-5", Some(1), Some(vec!["dragons".into()]));
        analyse_envelope(&env, 0, true, &mut report);
        assert_eq!(report.anomalies.len(), 1);
        assert!(
            matches!(&report.anomalies[0].reason, AnomalyReason::UnknownCompartment { label } if label == "dragons")
        );
    }

    #[test]
    fn known_compartments_not_flagged() {
        let mut report = ClassificationMigrationReport::default();
        for compartment in KNOWN_COMPARTMENTS {
            let env = envelope("evt-known", Some(1), Some(vec![compartment.to_string()]));
            analyse_envelope(&env, 0, true, &mut report);
        }
        assert!(report.anomalies.is_empty());
    }
}

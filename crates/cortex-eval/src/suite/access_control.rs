//! Phase21 §9.1 — access-control golden suite.
//!
//! Evaluates the Bell-LaPadula `can_read` predicate over a matrix of
//! `(principal_clearance, compartment_grants) × (fact_level, fact_compartments)`
//! cases. The acceptance gate is **zero false-grants**: a fact labelled
//! `hidden` MUST NOT appear visible to its associated principal.
//!
//! Golden CSV shape:
//!
//! ```csv
//! id,clearance_level,compartment_grants,fact_level,fact_compartments,expected
//! ac-001,1,,0,,visible
//! ac-002,0,,1,,hidden
//! ```
//!
//! `compartment_grants` and `fact_compartments` are `;`-delimited strings;
//! empty string means no compartments.
//!
//! ## False-grant vs recall
//!
//! | Outcome | Security impact | CI gate |
//! |---|---|---|
//! | `expected=hidden` → result=visible | **Leak — critical** | Fails CI |
//! | `expected=visible` → result=hidden | Recall loss — tolerable | Does NOT fail CI |

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::report::{MetricRow, SuiteReport};
use crate::suite::AcceptanceVerdict;

// ---------------------------------------------------------------------------
// Predicate (inline — cortex-eval does not depend on cortex-api as a crate)
// ---------------------------------------------------------------------------

/// Minimal principal representation for eval purposes.
#[derive(Debug, Clone)]
pub struct EvalPrincipal {
    /// Bell-LaPadula clearance level (0=public … 3=restricted).
    pub clearance_level: u8,
    /// Set of compartment labels the principal holds (e.g. `"financial"`, `"hr"`).
    pub compartment_grants: Vec<String>,
    /// If `true`, bypasses compartment check (but NOT the level gate).
    pub is_acl_admin: bool,
}

/// Bell-LaPadula read predicate mirroring
/// `cortex_api::security::principal::can_read`.
///
/// Duplicated here so the eval crate stays HTTP-only without pulling
/// the cortex-api crate tree as a build dependency.
pub fn can_read_local(
    principal: &EvalPrincipal,
    fact_level: u8,
    fact_compartments: &[String],
) -> bool {
    // No-read-up: level gate is absolute.
    if principal.clearance_level < fact_level {
        return false;
    }
    // acl_admin bypasses compartments, not levels.
    if principal.is_acl_admin {
        return true;
    }
    // Need-to-know: every fact compartment must be granted.
    for c in fact_compartments {
        if !principal.compartment_grants.contains(c) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// CSV row type
// ---------------------------------------------------------------------------

/// One row in `tests/golden/access_control.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AccessControlRow {
    /// Stable row identifier.
    pub id: String,
    /// Principal's Bell-LaPadula clearance level (0–3).
    pub clearance_level: u8,
    /// `;`-delimited compartment grants; empty string = no grants.
    pub compartment_grants: String,
    /// Classification level of the fact (0–3).
    pub fact_level: u8,
    /// `;`-delimited compartments required by the fact; empty = public.
    pub fact_compartments: String,
    /// `"visible"` = principal CAN read this fact; `"hidden"` = principal
    /// MUST NOT read this fact.
    pub expected: String,
    /// `"true"` if the principal holds the `acl_admin` role; empty or
    /// `"false"` otherwise.
    #[serde(default)]
    pub is_acl_admin: String,
}

impl AccessControlRow {
    fn principal(&self) -> EvalPrincipal {
        let compartment_grants: Vec<String> = if self.compartment_grants.is_empty() {
            vec![]
        } else {
            self.compartment_grants
                .split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        EvalPrincipal {
            clearance_level: self.clearance_level,
            compartment_grants,
            is_acl_admin: self.is_acl_admin.trim() == "true",
        }
    }

    fn fact_compartments_vec(&self) -> Vec<String> {
        if self.fact_compartments.is_empty() {
            vec![]
        } else {
            self.fact_compartments
                .split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
    }

    /// Evaluate the row using the inline predicate. Returns `true` when
    /// `can_read_local` says the principal can see the fact.
    pub fn evaluate(&self) -> bool {
        let principal = self.principal();
        let comps = self.fact_compartments_vec();
        can_read_local(&principal, self.fact_level, &comps)
    }
}

// ---------------------------------------------------------------------------
// CSV loader
// ---------------------------------------------------------------------------

/// Read every [`AccessControlRow`] from the golden CSV at `path`.
pub fn load_csv(path: &Path) -> Result<Vec<AccessControlRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("open access_control golden csv {}", path.display()))?;
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        let r: AccessControlRow = row.context("parse access_control csv row")?;
        out.push(r);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Report builder
// ---------------------------------------------------------------------------

/// Build a [`SuiteReport`] from per-row `observed` visibility flags.
///
/// - `observed[i] = true` means the fact was **visible** to the principal.
/// - A row labelled `"hidden"` but observed as `true` → **false-grant** (leak).
/// - A row labelled `"visible"` but observed as `false` → recall miss (tolerable).
///
/// **Acceptance gate**: `false_grant_count == 0`.
pub fn build_report(rows: &[AccessControlRow], observed: &[bool]) -> SuiteReport {
    assert_eq!(
        rows.len(),
        observed.len(),
        "row/observation length mismatch"
    );
    let mut false_grants: u32 = 0;
    let mut recall_hits: u32 = 0;
    let mut recall_total: u32 = 0;

    let mut per_row = std::collections::BTreeMap::new();
    for (row, &vis) in rows.iter().zip(observed.iter()) {
        let expected_visible = row.expected.trim() == "visible";
        let false_grant = !expected_visible && vis;
        let recall_miss = expected_visible && !vis;
        if false_grant {
            false_grants += 1;
        }
        if expected_visible {
            recall_total += 1;
            if !recall_miss {
                recall_hits += 1;
            }
        }
        per_row.insert(
            row.id.clone(),
            serde_json::json!({
                "clearance_level": row.clearance_level,
                "fact_level": row.fact_level,
                "expected": row.expected,
                "observed_visible": vis,
                "false_grant": false_grant,
                "recall_miss": recall_miss,
                "ok": expected_visible == vis,
            }),
        );
    }

    let grant_recall = if recall_total == 0 {
        1.0_f64
    } else {
        recall_hits as f64 / recall_total as f64
    };

    SuiteReport {
        suite: "access_control".into(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        rows_total: rows.len() as u32,
        rows_errored: 0,
        metrics: vec![
            MetricRow {
                name: "false_grant_count".into(),
                value: false_grants as f64,
                // Zero is the only acceptable value — any leak fails CI.
                floor: Some(0.0),
                passed: false_grants == 0,
            },
            MetricRow {
                name: "grant_recall".into(),
                value: grant_recall,
                // Recall loss does not fail CI (tolerable false-negative).
                floor: None,
                passed: true,
            },
        ],
        per_row,
    }
}

// ---------------------------------------------------------------------------
// Acceptance gate
// ---------------------------------------------------------------------------

/// Phase21 §9.1 acceptance gate for the access-control suite.
///
/// The suite passes if and only if `false_grant_count == 0`.
/// A non-zero recall loss is recorded but does NOT fail the gate.
pub fn access_control_acceptance(report: &SuiteReport) -> AcceptanceVerdict {
    let false_grants = report
        .metric("false_grant_count")
        .map(|m| m.value as u64)
        .unwrap_or(1); // missing metric → assume failure
    AcceptanceVerdict {
        passed: false_grants == 0,
        failed_metrics: if false_grants == 0 {
            vec![]
        } else {
            vec!["false_grant_count".into()]
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: &str,
        clearance: u8,
        grants: &str,
        fact_level: u8,
        comps: &str,
        expected: &str,
    ) -> AccessControlRow {
        AccessControlRow {
            id: id.into(),
            clearance_level: clearance,
            compartment_grants: grants.into(),
            fact_level,
            fact_compartments: comps.into(),
            expected: expected.into(),
            is_acl_admin: String::new(),
        }
    }

    // --- can_read_local ---

    #[test]
    fn public_fact_visible_to_any_clearance() {
        let p = EvalPrincipal {
            clearance_level: 0,
            compartment_grants: vec![],
            is_acl_admin: false,
        };
        assert!(can_read_local(&p, 0, &[]));
    }

    #[test]
    fn level_gate_blocks_above_clearance() {
        let p = EvalPrincipal {
            clearance_level: 1,
            compartment_grants: vec![],
            is_acl_admin: false,
        };
        assert!(!can_read_local(&p, 2, &[]));
        assert!(can_read_local(&p, 1, &[]));
    }

    #[test]
    fn compartment_gate_blocks_missing_grant() {
        let p = EvalPrincipal {
            clearance_level: 3,
            compartment_grants: vec!["financial".into()],
            is_acl_admin: false,
        };
        assert!(!can_read_local(&p, 1, &["legal".into()]));
        assert!(can_read_local(&p, 1, &["financial".into()]));
    }

    #[test]
    fn acl_admin_bypasses_compartments_not_levels() {
        let p = EvalPrincipal {
            clearance_level: 2,
            compartment_grants: vec![],
            is_acl_admin: true,
        };
        // Compartment bypass works.
        assert!(can_read_local(&p, 2, &["top-secret-compart".into()]));
        // Level gate still applies.
        assert!(!can_read_local(&p, 3, &[]));
    }

    // --- build_report ---

    #[test]
    fn zero_false_grants_passes() {
        let rows = vec![
            row("a", 1, "", 0, "", "visible"),
            row("b", 0, "", 1, "", "hidden"),
            row("c", 2, "financial", 1, "financial", "visible"),
        ];
        // Correct observations: a=visible, b=hidden→false, c=visible=true
        let observed = vec![true, false, true];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("false_grant_count").unwrap().value, 0.0);
        assert_eq!(r.metric("grant_recall").unwrap().value, 1.0);
        let v = access_control_acceptance(&r);
        assert!(v.passed);
    }

    #[test]
    fn false_grant_fails_acceptance() {
        let rows = vec![
            row("x", 0, "", 3, "", "hidden"), // should be hidden
        ];
        // Observed as visible → false-grant (security leak)
        let observed = vec![true];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("false_grant_count").unwrap().value, 1.0);
        let v = access_control_acceptance(&r);
        assert!(!v.passed);
        assert!(v.failed_metrics.contains(&"false_grant_count".to_string()));
    }

    #[test]
    fn recall_miss_does_not_fail_acceptance() {
        let rows = vec![
            row("y", 3, "financial", 1, "financial", "visible"), // should be visible
        ];
        // Observed as hidden → recall miss (NOT a security issue)
        let observed = vec![false];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("false_grant_count").unwrap().value, 0.0);
        assert_eq!(r.metric("grant_recall").unwrap().value, 0.0);
        let v = access_control_acceptance(&r);
        assert!(v.passed, "recall miss must not fail the acceptance gate");
    }

    #[test]
    fn empty_input_passes_with_zero_false_grants() {
        let r = build_report(&[], &[]);
        assert_eq!(r.metric("false_grant_count").unwrap().value, 0.0);
        let v = access_control_acceptance(&r);
        assert!(v.passed);
    }

    // --- row evaluate ---

    #[test]
    fn row_evaluate_matches_can_read_local() {
        let r = row("t", 1, "financial", 1, "financial", "visible");
        assert!(r.evaluate());

        let r2 = row("t2", 0, "", 2, "", "hidden");
        assert!(!r2.evaluate());

        let r3 = AccessControlRow {
            id: "t3".into(),
            clearance_level: 2,
            compartment_grants: String::new(),
            fact_level: 1,
            fact_compartments: "hr".into(),
            expected: "hidden".into(),
            is_acl_admin: "true".into(),
        };
        // acl_admin bypasses compartment → visible
        assert!(r3.evaluate());
    }

    // --- load_csv (round-trip via tempfile) ---

    #[test]
    fn load_csv_round_trips_basic_row() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "id,clearance_level,compartment_grants,fact_level,fact_compartments,expected,is_acl_admin"
        )
        .unwrap();
        writeln!(tmp, "ac-t,1,financial,0,,visible,").unwrap();
        let rows = load_csv(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ac-t");
        assert_eq!(rows[0].clearance_level, 1);
        assert_eq!(rows[0].fact_level, 0);
        assert_eq!(rows[0].expected, "visible");
    }
}

//! Phase21 §4.1 — Principal type and the Bell-LaPadula `can_read` predicate.
//!
//! A `Principal` represents an authenticated caller's access clearance:
//! - `clearance_level` — maximum sensitivity level readable (no-read-up rule).
//! - `compartment_grants` — set of need-to-know compartment labels granted.
//! - `roles` — RBAC role labels used for `acl_admin` gate and future extensions.
//!
//! The `can_read(principal, fact_level, fact_compartments)` predicate enforces
//! the lattice: the principal must hold clearance ≥ fact level AND every
//! fact compartment must be in their grants. A super-admin role (`acl_admin`)
//! bypasses compartment checks but still observes the level gate.

use serde::{Deserialize, Serialize};

/// A lightweight reference to a principal by id.
///
/// Threaded through `QueryRequest.principal` so the orchestrator can route
/// to the correct enforcement point without re-resolving from storage. The
/// full `Principal` is resolved once per request by the auth middleware
/// via [`PrincipalStore`](super::principal_store::PrincipalStore).
pub type PrincipalRef = String;

/// Authenticated caller identity + access clearance.
///
/// Produced by the auth middleware from the API key → role → clearance
/// binding chain; consumed by every enforcement point (Meili filter, vector
/// pre-filter, graph WHERE, post-fusion wedge, bundle filter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// Stable principal identifier (typically the API key id or user id).
    pub id: String,
    /// Maximum sensitivity level this principal is permitted to read.
    /// Clearance 3 = restricted; 0 = public only.
    pub clearance_level: u8,
    /// Compartment labels the principal holds a need-to-know grant for.
    /// An empty set means no compartment-gated facts are accessible.
    pub compartment_grants: Vec<String>,
    /// RBAC role labels. The `acl_admin` role bypasses compartment checks
    /// but not the level gate (Bell-LaPadula no-read-up is absolute).
    pub roles: Vec<String>,
}

impl Principal {
    /// Build a public-only principal that can read nothing above level 0.
    pub fn public_only(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            clearance_level: 0,
            compartment_grants: Vec::new(),
            roles: Vec::new(),
        }
    }

    /// Build a fully-cleared super-admin principal (clearance 3, all
    /// compartments, `acl_admin` role). Used for operator tooling; never
    /// handed out to end-user API keys unless explicitly configured.
    pub fn super_admin(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            clearance_level: 3,
            compartment_grants: vec![
                "financial".into(),
                "hr".into(),
                "legal".into(),
                "security".into(),
                "customer_pii".into(),
            ],
            roles: vec!["acl_admin".into()],
        }
    }

    /// Returns `true` when the principal holds the `acl_admin` role.
    pub fn is_acl_admin(&self) -> bool {
        self.roles.iter().any(|r| r == "acl_admin")
    }
}

/// Bell-LaPadula read predicate.
///
/// Returns `true` when the principal is allowed to read a fact whose
/// sensitivity is `(fact_level, fact_compartments)`.
///
/// Rules (ADR-032):
/// - `clearance_level >= fact_level` (no-read-up).
/// - Every compartment in `fact_compartments` must appear in
///   `principal.compartment_grants` (need-to-know). A principal with
///   the `acl_admin` role is exempt from the compartment check but
///   not from the level check.
/// - If `fact_level == 0` and `fact_compartments` is empty (public),
///   all principals may read.
pub fn can_read(principal: &Principal, fact_level: u8, fact_compartments: &[String]) -> bool {
    // No-read-up: level gate is absolute.
    if principal.clearance_level < fact_level {
        return false;
    }
    // acl_admin bypasses compartment check (but cannot bypass level).
    if principal.is_acl_admin() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(clearance: u8, compartments: Vec<&str>, roles: Vec<&str>) -> Principal {
        Principal {
            id: "test-principal".into(),
            clearance_level: clearance,
            compartment_grants: compartments.iter().map(|s| s.to_string()).collect(),
            roles: roles.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ---- public fact (level=0, no compartments) ----

    #[test]
    fn public_fact_is_readable_by_any_clearance() {
        for cl in 0..=3u8 {
            let p = principal(cl, vec![], vec![]);
            assert!(can_read(&p, 0, &[]), "clearance {cl} must read public fact");
        }
    }

    #[test]
    fn public_fact_readable_by_public_only_principal() {
        let p = Principal::public_only("anon");
        assert!(can_read(&p, 0, &[]));
    }

    // ---- level gate (no-read-up) ----

    #[test]
    fn principal_cannot_read_above_clearance() {
        let p = principal(1, vec![], vec![]);
        assert!(!can_read(&p, 2, &[]), "internal(1) cannot read confidential(2)");
        assert!(!can_read(&p, 3, &[]), "internal(1) cannot read restricted(3)");
    }

    #[test]
    fn principal_can_read_at_or_below_clearance() {
        let p = principal(2, vec![], vec![]);
        assert!(can_read(&p, 0, &[]), "confidential(2) reads public(0)");
        assert!(can_read(&p, 1, &[]), "confidential(2) reads internal(1)");
        assert!(can_read(&p, 2, &[]), "confidential(2) reads confidential(2)");
        assert!(!can_read(&p, 3, &[]), "confidential(2) cannot read restricted(3)");
    }

    // ---- compartment gate (need-to-know) ----

    #[test]
    fn missing_compartment_grant_denies_access() {
        let p = principal(3, vec!["financial"], vec![]);
        let comps = vec!["financial".into(), "hr".into()];
        assert!(!can_read(&p, 2, &comps), "missing hr grant → deny");
    }

    #[test]
    fn all_compartment_grants_present_allows_access() {
        let p = principal(3, vec!["financial", "hr", "legal"], vec![]);
        let comps = vec!["financial".into(), "hr".into()];
        assert!(can_read(&p, 2, &comps), "all grants present → allow");
    }

    #[test]
    fn empty_fact_compartments_only_level_gate_applies() {
        let p = principal(2, vec![], vec![]);
        assert!(can_read(&p, 2, &[]), "no fact compartments → only level matters");
    }

    #[test]
    fn extra_principal_grants_do_not_interfere() {
        let p = principal(3, vec!["financial", "hr", "security"], vec![]);
        let comps = vec!["financial".into()];
        assert!(can_read(&p, 3, &comps), "extra grants are fine; required grant present");
    }

    // ---- acl_admin bypass ----

    #[test]
    fn acl_admin_bypasses_compartment_check() {
        let p = principal(3, vec![], vec!["acl_admin"]);
        let comps = vec!["financial".into(), "hr".into(), "security".into()];
        assert!(
            can_read(&p, 3, &comps),
            "acl_admin bypasses compartment check at clearance=3"
        );
    }

    #[test]
    fn acl_admin_still_obeys_level_gate() {
        // Even acl_admin cannot read above their clearance level.
        let p = principal(2, vec![], vec!["acl_admin"]);
        assert!(!can_read(&p, 3, &[]), "acl_admin with clearance=2 cannot read restricted(3)");
        assert!(can_read(&p, 2, &[]), "acl_admin with clearance=2 reads confidential(2)");
    }

    #[test]
    fn super_admin_reads_everything_at_level() {
        let p = Principal::super_admin("ops");
        for level in 0..=3u8 {
            let comps: Vec<String> = vec!["financial".into(), "security".into()];
            assert!(can_read(&p, level, &comps), "super_admin reads level={level}");
        }
    }

    // ---- edge cases ----

    #[test]
    fn public_only_principal_denied_any_compartment_fact() {
        let p = Principal::public_only("anon");
        assert!(!can_read(&p, 1, &[]), "public_only cannot read internal");
        assert!(!can_read(&p, 0, &["financial".into()]), "public_only denied compartment fact");
    }

    #[test]
    fn is_acl_admin_detects_role_correctly() {
        let admin = principal(3, vec![], vec!["acl_admin", "viewer"]);
        let viewer = principal(1, vec![], vec!["viewer"]);
        assert!(admin.is_acl_admin());
        assert!(!viewer.is_acl_admin());
    }
}

//! Phase21 §4.2 — PrincipalStore: role bindings and principal resolution.
//!
//! A `RoleBinding` maps a role label to a clearance level + compartment set.
//! `PrincipalStore` holds a table of role bindings and a default principal,
//! resolving a set of role labels into a merged `Principal` via the
//! escalate-only merge rule (max level, union compartments).
//!
//! The store is admin-mutable at runtime (via the ACL admin API in §6.2)
//! and is held behind an `Arc<RwLock<PrincipalStore>>` so updates are
//! visible without a restart.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::principal::Principal;

/// The clearance + compartment grants that a RBAC role confers.
///
/// A principal's effective clearance is the escalate-only merge over all
/// roles they hold (max level, union compartments). See `PrincipalStore::resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBinding {
    /// Clearance level this role confers (0–3).
    pub clearance_level: u8,
    /// Compartment labels this role grants need-to-know access to.
    #[serde(default)]
    pub compartments: Vec<String>,
}

/// Resolves role labels → a `Principal` with merged clearance + compartments.
///
/// Built at daemon startup from `cortex.toml` `[access_control]` (§7.1) and
/// hot-patched by the ACL admin API (§6.2). Stored behind
/// `Arc<RwLock<PrincipalStore>>` in `QueryService`.
///
/// Invariant: the store always contains at least a `default` principal so that
/// unauthenticated callers (or callers whose key has no role binding) get a
/// deterministic effective clearance.
#[derive(Debug, Clone)]
pub struct PrincipalStore {
    /// Role label → binding.  Key is the normalised lowercase role name.
    bindings: HashMap<String, RoleBinding>,
    /// Principal returned for callers with no role assignment.
    default_principal: Principal,
}

impl PrincipalStore {
    /// Create a store from a binding table and a default principal.
    pub fn new(
        bindings: impl IntoIterator<Item = (String, RoleBinding)>,
        default_principal: Principal,
    ) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
            default_principal,
        }
    }

    /// Build the minimal "deny-by-default" store: every caller that lacks
    /// an explicit role binding is treated as public-only (level=0, no
    /// compartments). Used when `access_control.enabled = true` but no
    /// bindings have been configured yet.
    pub fn deny_by_default() -> Self {
        Self {
            bindings: HashMap::new(),
            default_principal: Principal::public_only("default"),
        }
    }

    /// Build a permissive "pass-through" store: the default principal has
    /// full clearance (level=3, all compartments, `acl_admin`). Used when
    /// `access_control.enabled = false` so every caller is effectively
    /// unrestricted without needing to configure bindings.
    pub fn pass_through() -> Self {
        Self {
            bindings: HashMap::new(),
            default_principal: Principal::super_admin("default"),
        }
    }

    /// Return the default principal (used for unauthenticated / no-binding callers).
    pub fn default_principal(&self) -> &Principal {
        &self.default_principal
    }

    /// Replace the default principal (admin-mutable).
    pub fn set_default_principal(&mut self, p: Principal) {
        self.default_principal = p;
    }

    /// Insert or replace a role binding.
    pub fn upsert_binding(&mut self, role: impl Into<String>, binding: RoleBinding) {
        self.bindings.insert(role.into(), binding);
    }

    /// Remove a role binding. Returns the old binding if it existed.
    pub fn remove_binding(&mut self, role: &str) -> Option<RoleBinding> {
        self.bindings.remove(role)
    }

    /// Snapshot the current binding table (for the `whoami` + list APIs).
    pub fn bindings(&self) -> &HashMap<String, RoleBinding> {
        &self.bindings
    }

    /// Resolve a set of role labels + a principal id into a `Principal`.
    ///
    /// The effective clearance is computed as the escalate-only merge over
    /// every matching binding: `level = max(all bindings)`,
    /// `compartments = union(all bindings)`.
    ///
    /// If `roles` is empty or none of the labels match a binding, the
    /// `default_principal` is returned (with its `id` replaced by `principal_id`).
    pub fn resolve(&self, principal_id: impl Into<String>, roles: &[String]) -> Principal {
        let id = principal_id.into();

        // Collect bindings for roles the caller actually holds.
        let mut matched: Vec<&RoleBinding> = roles
            .iter()
            .filter_map(|r| self.bindings.get(r.as_str()))
            .collect();

        if matched.is_empty() {
            // Fall through to the default principal but use the caller's own id.
            let mut p = self.default_principal.clone();
            p.id = id;
            return p;
        }

        // Escalate-only merge across all matched bindings.
        let clearance_level = matched.iter().map(|b| b.clearance_level).max().unwrap_or(0);
        let mut compartments: Vec<String> = Vec::new();
        for b in &mut matched {
            for c in &b.compartments {
                if !compartments.contains(c) {
                    compartments.push(c.clone());
                }
            }
        }

        Principal {
            id,
            clearance_level,
            compartment_grants: compartments,
            roles: roles.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(level: u8, compartments: Vec<&str>) -> RoleBinding {
        RoleBinding {
            clearance_level: level,
            compartments: compartments.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn store() -> PrincipalStore {
        PrincipalStore::new(
            [
                ("viewer".to_string(), binding(0, vec![])),
                ("analyst".to_string(), binding(1, vec!["financial"])),
                ("hr_specialist".to_string(), binding(2, vec!["hr", "financial"])),
                ("security_eng".to_string(), binding(3, vec!["security"])),
                ("acl_admin".to_string(), binding(3, vec![])),
            ],
            Principal::public_only("default"),
        )
    }

    // ---- default fallback ----

    #[test]
    fn empty_roles_returns_default_with_caller_id() {
        let s = store();
        let p = s.resolve("caller-1", &[]);
        assert_eq!(p.id, "caller-1");
        assert_eq!(p.clearance_level, 0);
        assert!(p.compartment_grants.is_empty());
    }

    #[test]
    fn unknown_role_falls_back_to_default() {
        let s = store();
        let p = s.resolve("caller-2", &["nonexistent".to_string()]);
        assert_eq!(p.clearance_level, 0);
        assert_eq!(p.id, "caller-2");
    }

    // ---- single-role resolution ----

    #[test]
    fn single_role_resolves_to_its_binding() {
        let s = store();
        let p = s.resolve("u1", &["analyst".to_string()]);
        assert_eq!(p.clearance_level, 1);
        assert_eq!(p.compartment_grants, ["financial"]);
    }

    // ---- multi-role merge (escalate-only) ----

    #[test]
    fn multi_role_takes_max_level() {
        let s = store();
        let p = s.resolve("u2", &["viewer".to_string(), "security_eng".to_string()]);
        assert_eq!(p.clearance_level, 3);
    }

    #[test]
    fn multi_role_unions_compartments() {
        let s = store();
        let p = s.resolve(
            "u3",
            &["analyst".to_string(), "hr_specialist".to_string()],
        );
        assert_eq!(p.clearance_level, 2);
        // Union of {financial} + {hr, financial} = {financial, hr}
        let mut grants = p.compartment_grants.clone();
        grants.sort();
        assert_eq!(grants, ["financial", "hr"]);
    }

    #[test]
    fn acl_admin_role_confers_max_level_no_compartments() {
        let s = store();
        let p = s.resolve("admin-u", &["acl_admin".to_string()]);
        assert_eq!(p.clearance_level, 3);
        // The binding has no compartments; the role label is present in roles
        assert!(p.roles.contains(&"acl_admin".to_string()));
    }

    // ---- admin mutations ----

    #[test]
    fn upsert_binding_adds_new_role() {
        let mut s = store();
        s.upsert_binding("legal_counsel", binding(2, vec!["legal"]));
        let p = s.resolve("u4", &["legal_counsel".to_string()]);
        assert_eq!(p.clearance_level, 2);
        assert_eq!(p.compartment_grants, ["legal"]);
    }

    #[test]
    fn upsert_binding_replaces_existing() {
        let mut s = store();
        s.upsert_binding("viewer", binding(1, vec!["financial"]));
        let p = s.resolve("u5", &["viewer".to_string()]);
        assert_eq!(p.clearance_level, 1);
        assert_eq!(p.compartment_grants, ["financial"]);
    }

    #[test]
    fn remove_binding_falls_back_to_default() {
        let mut s = store();
        let old = s.remove_binding("analyst");
        assert!(old.is_some());
        let p = s.resolve("u6", &["analyst".to_string()]);
        assert_eq!(p.clearance_level, 0, "removed role → default");
    }

    #[test]
    fn set_default_principal_changes_fallback() {
        let mut s = store();
        s.set_default_principal(Principal::super_admin("default"));
        let p = s.resolve("anon", &[]);
        assert_eq!(p.clearance_level, 3);
    }

    // ---- pass-through and deny-by-default constructors ----

    #[test]
    fn pass_through_store_default_is_super_admin() {
        let s = PrincipalStore::pass_through();
        let p = s.resolve("any", &[]);
        assert_eq!(p.clearance_level, 3);
        assert!(p.is_acl_admin());
    }

    #[test]
    fn deny_by_default_store_default_is_public_only() {
        let s = PrincipalStore::deny_by_default();
        let p = s.resolve("any", &[]);
        assert_eq!(p.clearance_level, 0);
        assert!(p.compartment_grants.is_empty());
    }

    // ---- serde round-trip for RoleBinding ----

    #[test]
    fn role_binding_serde_round_trips() {
        let b = binding(2, vec!["financial", "hr"]);
        let json = serde_json::to_string(&b).unwrap();
        let back: RoleBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn role_binding_compartments_default_to_empty_when_absent() {
        let json = r#"{"clearance_level":1}"#;
        let b: RoleBinding = serde_json::from_str(json).unwrap();
        assert!(b.compartments.is_empty());
    }
}

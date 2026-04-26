//! Per-caller ACL store. Spec 11 §Security / privacy: every request
//! re-validates `scope.repo` against the caller's allowed repo list;
//! out-of-scope requests get a 403 `reason=scope_forbidden`.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

/// Outcome of an ACL check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AclDecision {
    /// Caller is allowed.
    Allow,
    /// Caller is denied with the spec-11 `scope_forbidden` reason.
    Deny,
    /// Caller is unknown (no ACL configured).
    Unknown,
}

/// Per-caller allow-list keyed by caller name (e.g.
/// `cortex-adapter-claude`, `dashboard`, `analysis`).
#[derive(Debug, Default)]
pub struct AclStore {
    inner: RwLock<HashMap<String, HashSet<String>>>,
}

impl AclStore {
    /// Build a fresh, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `repo` as allowed for `caller`. Idempotent.
    pub fn allow(&self, caller: &str, repo: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.entry(caller.to_string())
                .or_default()
                .insert(repo.to_string());
        }
    }

    /// Replace the allow-list for `caller` with `repos`.
    pub fn set_allowed(&self, caller: &str, repos: Vec<String>) {
        if let Ok(mut g) = self.inner.write() {
            g.insert(caller.to_string(), repos.into_iter().collect());
        }
    }

    /// Resolve the ACL decision for a request.
    ///
    /// `repo` is `None` when the request carries no scope; the spec
    /// allows that path through (the orchestrator returns
    /// scope-agnostic results) so we treat it as `Allow`.
    pub fn decide(&self, caller: &str, repo: Option<&str>) -> AclDecision {
        let g = match self.inner.read() {
            Ok(g) => g,
            Err(_) => return AclDecision::Deny,
        };
        let allowed = match g.get(caller) {
            Some(s) => s,
            None => return AclDecision::Unknown,
        };
        match repo {
            None => AclDecision::Allow,
            Some(r) => {
                if allowed.contains("*") || allowed.contains(r) {
                    AclDecision::Allow
                } else {
                    AclDecision::Deny
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_caller_decides_unknown() {
        let acl = AclStore::new();
        assert_eq!(acl.decide("nobody", Some("R")), AclDecision::Unknown);
    }

    #[test]
    fn caller_without_scope_passes() {
        let acl = AclStore::new();
        acl.allow("dash", "R");
        assert_eq!(acl.decide("dash", None), AclDecision::Allow);
    }

    #[test]
    fn unscoped_repo_for_known_caller_denies() {
        let acl = AclStore::new();
        acl.allow("dash", "R1");
        assert_eq!(acl.decide("dash", Some("R2")), AclDecision::Deny);
    }

    #[test]
    fn wildcard_caller_allows_everything() {
        let acl = AclStore::new();
        acl.allow("admin", "*");
        assert_eq!(acl.decide("admin", Some("anything")), AclDecision::Allow);
    }

    #[test]
    fn set_allowed_replaces_the_full_list() {
        let acl = AclStore::new();
        acl.allow("c", "OldRepo");
        acl.set_allowed("c", vec!["NewRepo".into()]);
        assert_eq!(acl.decide("c", Some("OldRepo")), AclDecision::Deny);
        assert_eq!(acl.decide("c", Some("NewRepo")), AclDecision::Allow);
    }
}

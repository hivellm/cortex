//! Phase21 §5.6 — pre-thinking bundle ACL filter.
//!
//! Applies the Bell-LaPadula `can_read` predicate to every snippet in a
//! `QueryResponse` before the bundle formatter assembles the system-prompt
//! context. Prevents classified facts from reaching the LLM prompt even
//! when the upstream orchestrator's §5.5 wedge has already filtered the
//! `LaneHit` stream (defense-in-depth; this layer operates at the
//! `Snippet` level after the API response is received).
//!
//! ## Why operate at the Snippet level
//!
//! The pre-thinking pipeline receives a `QueryResponse` via `QueryFn`.
//! In production, `QueryFn` calls the API over HTTP and the server's
//! §5.5 orchestrator wedge drops classified hits before they reach
//! `Snippet`. For test doubles and future in-process adapters, the
//! double may return unfiltered hits, so this secondary check is
//! necessary.

use cortex_api::principal::can_read;
use cortex_api::types::Snippet;
use cortex_api::Principal;

/// Phase21 §5.6 — filter `snippets` in-place, removing any snippet
/// that the principal may not read per the Bell-LaPadula predicate.
///
/// When `principal` is `None` the function is a no-op (backward compat:
/// pre-thinking callers that do not supply a principal see all snippets).
pub fn filter_snippets_by_acl(snippets: &mut Vec<Snippet>, principal: Option<&Principal>) {
    let acl = match principal {
        Some(p) => p,
        None => return,
    };

    snippets.retain(|s| {
        can_read(acl, s.class_level.unwrap_or(0), &s.class_compartments)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_api::types::Snippet;

    fn make_snippet(id: &str, level: u8, compartments: &[&str]) -> Snippet {
        Snippet {
            rank: 1,
            source: "keyword".into(),
            collection: None,
            repo: None,
            path: Some(format!("{id}.rs")),
            symbol: None,
            content_hash: None,
            text: id.into(),
            body_truncated: false,
            score: 1.0,
            why: None,
            verified: None,
            verdict: None,
            class_level: if level == 0 { None } else { Some(level) },
            class_compartments: compartments.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn principal(clearance: u8, compartments: &[&str]) -> Principal {
        Principal {
            id: "tester".into(),
            clearance_level: clearance,
            compartment_grants: compartments.iter().map(|s| s.to_string()).collect(),
            roles: vec![],
        }
    }

    #[test]
    fn filter_passes_unclassified_for_any_principal() {
        let mut snippets = vec![make_snippet("public", 0, &[])];
        filter_snippets_by_acl(&mut snippets, Some(&principal(0, &[])));
        assert_eq!(snippets.len(), 1);
    }

    #[test]
    fn filter_drops_snippet_above_clearance() {
        let mut snippets = vec![
            make_snippet("internal", 1, &[]),
            make_snippet("confidential", 2, &[]),
        ];
        filter_snippets_by_acl(&mut snippets, Some(&principal(1, &[])));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].text, "internal");
    }

    #[test]
    fn filter_drops_snippet_with_missing_compartment() {
        let mut snippets = vec![
            make_snippet("no-compartment", 1, &[]),
            make_snippet("hr-doc", 1, &["hr"]),
        ];
        filter_snippets_by_acl(&mut snippets, Some(&principal(2, &["financial"])));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].text, "no-compartment");
    }

    #[test]
    fn no_principal_passes_all_snippets_through() {
        let mut snippets = vec![
            make_snippet("public", 0, &[]),
            make_snippet("restricted", 3, &["security"]),
        ];
        filter_snippets_by_acl(&mut snippets, None);
        assert_eq!(snippets.len(), 2);
    }

    #[test]
    fn acl_admin_bypasses_compartment_check_but_not_level_gate() {
        let mut admin = principal(2, &[]);
        admin.roles = vec!["acl_admin".into()];
        let mut snippets = vec![
            make_snippet("public", 0, &[]),
            make_snippet("confidential-hr", 2, &["hr"]),
            // restricted(3) is ABOVE clearance=2 — must be dropped even for admin
            make_snippet("restricted-sec", 3, &["security"]),
        ];
        filter_snippets_by_acl(&mut snippets, Some(&admin));
        assert_eq!(snippets.len(), 2, "admin skips compartment but not level");
        assert!(snippets.iter().any(|s| s.text == "public"));
        assert!(snippets.iter().any(|s| s.text == "confidential-hr"));
    }

    #[test]
    fn super_admin_sees_all_at_clearance_3() {
        let admin = Principal::super_admin("ops");
        let mut snippets = vec![
            make_snippet("public", 0, &[]),
            make_snippet("restricted", 3, &["security"]),
        ];
        filter_snippets_by_acl(&mut snippets, Some(&admin));
        assert_eq!(snippets.len(), 2);
    }
}

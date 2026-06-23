//! Phase21 §5.1 — per-lane ACL filter renderers.
//!
//! Pure, stateless functions that produce backend-native filter
//! expressions from a caller's clearance level and compartment
//! grants. Three surfaces:
//!
//! - [`meili_acl_filter`] — Meilisearch filter grammar string (ANDed
//!   with existing scope clauses).
//! - [`vectorizer_acl_matches`] — exact Bell-LaPadula predicate for
//!   client-side post-projection filtering (Vectorizer has no
//!   server-side filter parameter).
//! - [`nexus_acl_where`] — Cypher WHERE fragment for Nexus templates.
//!
//! ## Defense-in-depth note
//!
//! The Meili and Nexus renderers use an OR-ANY approximation for the
//! compartment dimension (a document passes if any of its compartments
//! appears in the principal's grants, rather than requiring all). The
//! exact need-to-know predicate is applied in the post-fusion ACL
//! wedge (§5.5); these renderers reduce the result set before fusion
//! so the wedge processes fewer rows. No false-grants can originate
//! here because the wedge is the authoritative gate.
//!
//! ## Wildcard principal
//!
//! A compartment_grants list containing `"*"` (super-admin) bypasses
//! all compartment checks. The level check is always applied.

/// Escape a string literal for Meilisearch's filter grammar.
///
/// Meilisearch uses single-quoted string literals; inner `'` must be
/// escaped as `\'`. The function also escapes `\` so the grammar
/// parser does not interpret it as an escape sequence start.
fn quote_meili(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

/// Returns `true` if the grants list contains the wildcard `"*"`.
fn is_wildcard(compartment_grants: &[String]) -> bool {
    compartment_grants.iter().any(|g| g == "*")
}

/// Phase21 §5.1 — build the Meilisearch ACL filter clause for a
/// principal with `clearance_level` and `compartment_grants`.
///
/// Returned string is ready to AND into an existing Meili filter
/// expression. Always returns a non-empty string (at minimum the
/// level clause).
///
/// Filter semantics (approximation for the backend pre-filter):
/// - `class_level IS EMPTY OR class_level <= N` — pass unclassified
///   documents and documents the principal's level allows.
/// - AND `class_compartments IS EMPTY OR class_compartments IN [g1,g2,...]`
///   — pass compartment-free documents or documents that share at
///   least one compartment with the principal's grants (OR-ANY).
///
/// When the grants list is empty (and no wildcard), only
/// compartment-free documents pass the compartment clause.
/// When the grants list contains `"*"`, the compartment clause is
/// omitted entirely (super-admin pass-through).
pub fn meili_acl_filter(clearance_level: u8, compartment_grants: &[String]) -> String {
    let level_clause = format!(
        "(class_level IS EMPTY OR class_level <= {clearance_level})"
    );

    if is_wildcard(compartment_grants) {
        return level_clause;
    }

    let compartment_clause = if compartment_grants.is_empty() {
        "class_compartments IS EMPTY".to_string()
    } else {
        let list = compartment_grants
            .iter()
            .map(|g| quote_meili(g))
            .collect::<Vec<_>>()
            .join(", ");
        format!("(class_compartments IS EMPTY OR class_compartments IN [{list}])")
    };

    format!("{level_clause} AND {compartment_clause}")
}

/// Phase21 §5.1 — exact Bell-LaPadula predicate for Vectorizer
/// client-side post-projection filtering.
///
/// Returns `true` when the principal may read a document with
/// `fact_level` and `fact_compartments`. This is the authoritative
/// `can_read` predicate without the OR-ANY approximation used by the
/// Meili renderer.
///
/// - `fact_level = None` → treated as `0` (public).
/// - `fact_compartments` empty → no need-to-know check.
/// - Wildcard `"*"` in grants → compartment check skipped.
pub fn vectorizer_acl_matches(
    clearance_level: u8,
    compartment_grants: &[String],
    fact_level: Option<u8>,
    fact_compartments: &[String],
) -> bool {
    let level = fact_level.unwrap_or(0);

    // No-read-up: absolute level gate.
    if clearance_level < level {
        return false;
    }

    // Wildcard bypasses compartment check.
    if is_wildcard(compartment_grants) {
        return true;
    }

    // Need-to-know: every fact compartment must be in the grants.
    for compartment in fact_compartments {
        if !compartment_grants.contains(compartment) {
            return false;
        }
    }

    true
}

/// Phase21 §5.4 — build the Cypher WHERE fragment for Nexus graph
/// templates with ACL gating, using a caller-supplied `node_alias`.
///
/// The fragment is intended to be appended to an existing WHERE clause
/// with ` AND `. Different templates use different primary-node aliases
/// (`a`, `d`, `an`, `v`, …); pass the correct one for the template.
///
/// Compartment semantics match the Meili renderer (OR-ANY
/// approximation); the post-fusion wedge applies the exact predicate.
///
/// Parameters injected as Cypher literals (not bound parameters)
/// because the Nexus inline-literal workaround (nexus#3) applies to
/// all class filter extensions.
pub fn nexus_acl_where_for_alias(
    clearance_level: u8,
    compartment_grants: &[String],
    node_alias: &str,
) -> String {
    let level_clause = format!(
        "({node_alias}.class_level IS NULL OR {node_alias}.class_level <= {clearance_level})"
    );

    if is_wildcard(compartment_grants) {
        return level_clause;
    }

    let compartment_clause = if compartment_grants.is_empty() {
        format!(
            "({node_alias}.class_compartments IS NULL OR \
             SIZE({node_alias}.class_compartments) = 0)"
        )
    } else {
        let list = compartment_grants
            .iter()
            .map(|g| {
                let escaped = g.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "({node_alias}.class_compartments IS NULL OR \
             SIZE({node_alias}.class_compartments) = 0 OR \
             ANY(c IN {node_alias}.class_compartments WHERE c IN [{list}]))"
        )
    };

    format!("{level_clause} AND {compartment_clause}")
}

/// Phase21 §5.1 — build the Cypher WHERE fragment using the default
/// node alias `n`. Thin wrapper around [`nexus_acl_where_for_alias`].
pub fn nexus_acl_where(clearance_level: u8, compartment_grants: &[String]) -> String {
    nexus_acl_where_for_alias(clearance_level, compartment_grants, "n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── meili_acl_filter ────────────────────────────────────────────

    #[test]
    fn meili_level_only_for_wildcard_principal() {
        let out = meili_acl_filter(3, &["*".to_string()]);
        assert_eq!(out, "(class_level IS EMPTY OR class_level <= 3)");
    }

    #[test]
    fn meili_empty_grants_allows_only_compartment_free_docs() {
        let out = meili_acl_filter(2, &[]);
        assert!(out.contains("class_compartments IS EMPTY"), "got: {out}");
        assert!(!out.contains("IN ["), "got: {out}");
    }

    #[test]
    fn meili_single_compartment_in_list() {
        let out = meili_acl_filter(1, &["financial".to_string()]);
        assert!(out.contains("class_compartments IN ['financial']"), "got: {out}");
    }

    #[test]
    fn meili_multiple_compartments_joined() {
        let out = meili_acl_filter(2, &["financial".to_string(), "hr".to_string()]);
        assert!(out.contains("'financial'"), "got: {out}");
        assert!(out.contains("'hr'"), "got: {out}");
        assert!(out.contains("class_level <= 2"), "got: {out}");
    }

    #[test]
    fn meili_quotes_escape_single_quotes() {
        let out = meili_acl_filter(1, &["it's-special".to_string()]);
        assert!(out.contains("'it\\'s-special'"), "got: {out}");
    }

    #[test]
    fn meili_quotes_escape_backslash() {
        let out = meili_acl_filter(1, &["back\\slash".to_string()]);
        assert!(out.contains("'back\\\\slash'"), "got: {out}");
    }

    // ── vectorizer_acl_matches ──────────────────────────────────────

    #[test]
    fn vectorizer_denies_when_level_too_low() {
        assert!(!vectorizer_acl_matches(0, &[], Some(1), &[]));
        assert!(!vectorizer_acl_matches(1, &[], Some(2), &[]));
    }

    #[test]
    fn vectorizer_grants_when_level_sufficient_and_no_compartments() {
        assert!(vectorizer_acl_matches(2, &[], Some(2), &[]));
        assert!(vectorizer_acl_matches(3, &[], Some(0), &[]));
    }

    #[test]
    fn vectorizer_missing_level_treated_as_public() {
        assert!(vectorizer_acl_matches(0, &[], None, &[]));
    }

    #[test]
    fn vectorizer_grants_when_all_compartments_held() {
        let grants = vec!["financial".to_string(), "hr".to_string()];
        assert!(vectorizer_acl_matches(2, &grants, Some(1), &["financial".to_string()]));
        assert!(vectorizer_acl_matches(
            2,
            &grants,
            Some(1),
            &["financial".to_string(), "hr".to_string()]
        ));
    }

    #[test]
    fn vectorizer_denies_when_compartment_missing() {
        let grants = vec!["financial".to_string()];
        assert!(!vectorizer_acl_matches(
            2,
            &grants,
            Some(1),
            &["financial".to_string(), "hr".to_string()]
        ));
    }

    #[test]
    fn vectorizer_wildcard_bypasses_compartment_check() {
        assert!(vectorizer_acl_matches(2, &["*".to_string()], Some(2), &[
            "financial".to_string(),
            "restricted_ops".to_string()
        ]));
    }

    #[test]
    fn vectorizer_wildcard_still_respects_level_gate() {
        assert!(!vectorizer_acl_matches(1, &["*".to_string()], Some(2), &[]));
    }

    // ── nexus_acl_where ─────────────────────────────────────────────

    #[test]
    fn nexus_level_only_for_wildcard() {
        let out = nexus_acl_where(3, &["*".to_string()]);
        assert_eq!(
            out,
            "(n.class_level IS NULL OR n.class_level <= 3)"
        );
    }

    #[test]
    fn nexus_empty_grants_restricts_to_unclassified() {
        let out = nexus_acl_where(1, &[]);
        assert!(
            out.contains("SIZE(n.class_compartments) = 0"),
            "got: {out}"
        );
        assert!(!out.contains("ANY("), "got: {out}");
    }

    #[test]
    fn nexus_single_compartment_any_clause() {
        let out = nexus_acl_where(2, &["legal".to_string()]);
        assert!(out.contains("ANY(c IN n.class_compartments WHERE c IN [\"legal\"])"), "got: {out}");
    }

    #[test]
    fn nexus_escapes_double_quotes_in_compartment() {
        let out = nexus_acl_where(1, &["has\"quote".to_string()]);
        assert!(out.contains("\"has\\\"quote\""), "got: {out}");
    }

    #[test]
    fn nexus_level_clause_present_in_all_variants() {
        for grants in [
            vec![],
            vec!["financial".to_string()],
            vec!["*".to_string()],
        ] {
            let out = nexus_acl_where(2, &grants);
            assert!(
                out.contains("n.class_level IS NULL OR n.class_level <= 2"),
                "missing level clause for grants={grants:?}, got: {out}"
            );
        }
    }
}

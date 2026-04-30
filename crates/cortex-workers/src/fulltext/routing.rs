//! Per-kind, per-repo index routing.
//!
//! Spec 08 §Indexes lays out the family-to-index mapping. Index names
//! embed the owning repo's slug so each project lives in isolation —
//! `cortex-{repo_slug}-{family}` — and queries can scope deterministically
//! to a single project.

use cortex_core::events::Kind;
use cortex_storage::names::{slug_for_repo, UNKNOWN_REPO_SLUG};

use crate::embedder::EnrichedEvent;

/// Path extensions that route an artifact to the `code` family.
/// Source-code formats only — extensions whose contents are typically
/// imperative or declarative program text rather than prose.
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "rb",
    "java", "kt", "scala", "c", "cc", "cpp", "h", "hpp", "cs", "swift",
    "php", "lua", "sh", "bash", "zsh", "ps1", "fish", "sql", "proto",
];

/// Path extensions that route an artifact to the `docs` family —
/// prose-leaning formats. Anything not in either list lands in `misc`
/// when topic hints are also absent (honest middle-ground rather than
/// silently piling everything into `docs`).
const DOC_EXTENSIONS: &[&str] = &[
    "md", "mdx", "markdown", "rst", "adoc", "asciidoc", "txt", "rtf",
    "tex", "org",
];

/// Family suffix for the index a `Kind` routes to when no richer
/// signals (topics, path) are available. Spec 08 §Indexes:
/// `tool_call` ⇒ `code`, `decision` ⇒ `decisions`, `turn` ⇒ `turns`,
/// `law_violation` ⇒ `governance`, `analysis` ⇒ `analyses`,
/// `artifact` ⇒ `misc` (forced into the topic-aware path when called
/// via [`family_for_event`]), anything else ⇒ `misc`.
pub fn family_for(kind: Kind) -> &'static str {
    match kind {
        // tool_call sits alongside `code` because the body is almost always
        // a Bash / Edit / Write payload — searching for a command name
        // belongs in the same lane as searching for the source file it
        // touched.
        Kind::ToolCall => "code",
        Kind::Decision => "decisions",
        // agent_call rides with `turn` per spec-08's routing matrix —
        // both are conversational dispatches, and folding them into
        // `cortex-turns` keeps "what did the assistant do this turn"
        // queries deterministic.
        Kind::Turn | Kind::AgentCall => "turns",
        Kind::LawViolation => "governance",
        // Analysis events get their own family so audit / deep-analysis
        // reports do not get diluted in the catch-all `misc` bucket and
        // the dashboard's Analysis view can scope to a single index.
        Kind::Analysis => "analyses",
        Kind::Artifact => "misc",
        Kind::Memory => "misc",
        // phase10e — dedicated families so the orchestrator can
        // fan out to them on `pre_change_context` /
        // `decision_lookup` without diluting the catch-all `misc`
        // bucket.
        Kind::Knowledge => "knowledge",
        Kind::Learning => "learnings",
    }
}

/// Family routing that reads topics + path before falling back to the
/// kind-only matrix. The 2026-04-27 audit showed the kind-only matrix
/// piles every `artifact` event into `docs` regardless of whether it
/// is source code or prose — `cortex-code` ended up empty, `cortex-docs`
/// got 8 285 mixed-bag documents. This function fixes the routing so
/// `cortex-code`, `cortex-docs`, and `cortex-misc` each get their fair
/// slice of artifact events.
pub fn family_for_event(
    kind: Kind,
    topics: &[String],
    context_path: Option<&str>,
) -> &'static str {
    // Non-artifact kinds keep the kind-only routing — their family
    // is unambiguous from the kind alone.
    if !matches!(kind, Kind::Artifact) {
        return family_for(kind);
    }
    // Artifact: prefer the path extension because it is the most
    // reliable signal we have (the file itself tells us what it is).
    if let Some(ext) = context_path
        .and_then(|p| std::path::Path::new(p).extension())
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
    {
        if CODE_EXTENSIONS.contains(&ext.as_str()) {
            return "code";
        }
        if DOC_EXTENSIONS.contains(&ext.as_str()) {
            return "docs";
        }
    }
    // Fall back to classifier topics. The static classifier always
    // pushes "code" for artifacts today (see cortex-classifier
    // statics.rs); this branch covers richer classifier outputs and
    // future doc-aware static rules without re-touching this file.
    if topics.iter().any(|t| t == "code") {
        return "code";
    }
    if topics.iter().any(|t| t == "doc" || t == "documentation") {
        return "docs";
    }
    // Neither path nor topics gave us a clean signal — `misc` keeps
    // the artifact retrievable without lying about its category.
    "misc"
}

/// Compose the full index name from a prefix, repo slug, and family.
/// `prefix` is the deployment namespace (default `"cortex-"`); the
/// trailing dash is honoured so the result is `cortex-{slug}-{family}`.
///
/// The result is invariably a three-token name when split on `-`:
/// `cortex` / `{repo_slug}` / `{family}`. The `debug_assert!`
/// catches drift in dev / test builds; the runtime sweep
/// ([`is_canonical_index_name`]) catches it in production by
/// matching every existing Meili index against the same shape.
pub fn index_name(prefix: &str, repo_slug: &str, family: &str) -> String {
    let trimmed = prefix.trim_end_matches('-');
    let name = format!("{trimmed}-{repo_slug}-{family}");
    debug_assert!(
        is_canonical_index_name_for_prefix(&name, trimmed),
        "index_name produced a non-canonical name: {name:?} \
         (prefix={prefix:?}, repo_slug={repo_slug:?}, family={family:?})",
    );
    name
}

/// True when `name` matches the canonical
/// `cortex-{repo_slug}-{family}` shape — three non-empty tokens
/// separated by `-`, with the first token literally `cortex` and
/// the third one of [`FAMILIES`].
///
/// Phase4a §3 — the boot-time sweep walks every Meilisearch index
/// and routes empty non-canonical names into `delete_index`. The
/// regex-equivalent split-based check keeps the matcher pure (no
/// dependency on the `regex` crate) and lets the worker run with
/// the same vocabulary the routing code uses. The production
/// cluster always uses the `cortex` prefix; deployments that pass
/// a custom prefix to [`index_name`] use
/// [`is_canonical_index_name_for_prefix`] directly.
pub fn is_canonical_index_name(name: &str) -> bool {
    is_canonical_index_name_for_prefix(name, "cortex")
}

/// Like [`is_canonical_index_name`] but parameterised on the
/// deployment-namespace prefix. Callers from `index_name` use
/// this so the debug invariant accepts non-default deployments
/// (e.g. `staging-`); the production sweep keeps the `"cortex"`
/// default through [`is_canonical_index_name`].
pub fn is_canonical_index_name_for_prefix(name: &str, prefix: &str) -> bool {
    // The repo slug itself can contain `-` (e.g. `cortex-mcp`), so a
    // simple `splitn(3, '-')` would miss compound slugs. Instead we
    // check (a) starts with `<prefix>-`, (b) ends with `-{family}` for
    // some `family ∈ FAMILIES`, and (c) the middle slug is non-empty.
    let head = format!("{prefix}-");
    let rest = match name.strip_prefix(&head) {
        Some(r) => r,
        None => return false,
    };
    for family in FAMILIES.iter() {
        let suffix = format!("-{family}");
        if let Some(slug) = rest.strip_suffix(&suffix) {
            if !slug.is_empty()
                && slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !slug.starts_with('-')
                && !slug.ends_with('-')
            {
                return true;
            }
        }
    }
    false
}

/// Compose the full index name for a `Kind` + repo. `repo_id = None`
/// (or empty) falls back to [`UNKNOWN_REPO_SLUG`] so the produced
/// name is always well-formed.
pub fn index_for(prefix: &str, kind: Kind, repo_id: Option<&str>) -> String {
    let slug = repo_id
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    index_name(prefix, &slug, family_for(kind))
}

/// Compose the full index name from a complete [`EnrichedEvent`].
/// Reads `event.kind`, `event.classifier.topics`, and
/// `event.context_path` so artifact events route to `cortex-*-code`
/// or `cortex-*-docs` based on the file extension first, classifier
/// topics second.
pub fn index_for_event(prefix: &str, event: &EnrichedEvent) -> String {
    let slug = event
        .context_repo
        .as_deref()
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    let family = family_for_event(
        event.kind,
        &event.classifier.topics,
        event.context_path.as_deref(),
    );
    index_name(prefix, &slug, family)
}

/// All known family suffixes in deterministic order. Used by tests that
/// need to iterate every family. Bootstrap is now lazy per-repo, so
/// this is no longer driven into a fixed `ensure_index` loop.
pub const FAMILIES: &[&str] = &[
    "code",
    "docs",
    "decisions",
    "turns",
    "governance",
    "analyses",
    "misc",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_for_artifact_kind_only_lands_in_misc() {
        // Kind-only routing (no path / topics) cannot honestly tell
        // a code file from a doc — the caller is expected to use
        // `index_for_event` for that. The kind-only path lands in
        // `misc` rather than silently funnelling everything to `docs`.
        assert_eq!(
            index_for("cortex-", Kind::Artifact, Some("Cortex")),
            "cortex-cortex-misc"
        );
        assert_eq!(
            index_for("cortex-", Kind::Artifact, Some("Tml")),
            "cortex-tml-misc"
        );
    }

    #[test]
    fn index_for_falls_back_to_unknown_slug() {
        assert_eq!(
            index_for("cortex-", Kind::ToolCall, None),
            "cortex-unknown-code"
        );
        assert_eq!(
            index_for("cortex-", Kind::ToolCall, Some("")),
            "cortex-unknown-code"
        );
    }

    #[test]
    fn prefix_without_trailing_dash_still_works() {
        assert_eq!(
            index_for("cortex", Kind::Turn, Some("Cortex")),
            "cortex-cortex-turns"
        );
    }

    #[test]
    fn family_for_event_uses_path_extension_for_artifacts() {
        assert_eq!(
            family_for_event(Kind::Artifact, &[], Some("src/lib.rs")),
            "code"
        );
        assert_eq!(
            family_for_event(Kind::Artifact, &[], Some("docs/spec-08.md")),
            "docs"
        );
    }

    #[test]
    fn family_for_event_falls_back_to_topics_when_extension_unknown() {
        assert_eq!(
            family_for_event(Kind::Artifact, &["code".to_string()], Some("Cargo.toml")),
            "code"
        );
        assert_eq!(
            family_for_event(
                Kind::Artifact,
                &["doc".to_string()],
                Some("README.no-ext"),
            ),
            "docs"
        );
    }

    #[test]
    fn family_for_event_lands_in_misc_when_no_signal() {
        assert_eq!(
            family_for_event(Kind::Artifact, &[], Some("path/to/binary.bin")),
            "misc"
        );
        assert_eq!(family_for_event(Kind::Artifact, &[], None), "misc");
    }

    #[test]
    fn family_for_event_keeps_kind_routing_for_non_artifacts() {
        assert_eq!(family_for_event(Kind::Decision, &[], None), "decisions");
        assert_eq!(family_for_event(Kind::Turn, &[], None), "turns");
        assert_eq!(
            family_for_event(Kind::LawViolation, &[], None),
            "governance"
        );
        assert_eq!(family_for_event(Kind::ToolCall, &[], None), "code");
        // agent_call rides with `turn` per spec-08 — both are
        // conversational dispatches.
        assert_eq!(family_for_event(Kind::AgentCall, &[], None), "turns");
        // Analysis events route to their own dedicated family so the
        // dashboard Analysis view can scope to a single index.
        assert_eq!(family_for_event(Kind::Analysis, &[], None), "analyses");
    }

    #[test]
    fn analysis_index_uses_analyses_family_per_repo() {
        assert_eq!(
            index_for("cortex-", Kind::Analysis, Some("Cortex")),
            "cortex-cortex-analyses"
        );
        assert_eq!(
            index_for("cortex-", Kind::Analysis, Some("Rulebook")),
            "cortex-rulebook-analyses"
        );
    }

    #[test]
    fn is_canonical_index_name_matches_three_token_shape() {
        // Canonical: cortex-{slug}-{family} with a known family.
        assert!(is_canonical_index_name("cortex-cortex-code"));
        assert!(is_canonical_index_name("cortex-rulebook-decisions"));
        assert!(is_canonical_index_name("cortex-cortex-mcp-code"));
        assert!(is_canonical_index_name("cortex-tml-analyses"));
        // Non-canonical: missing slug, missing family, unknown family.
        assert!(!is_canonical_index_name("cortex-code"));
        assert!(!is_canonical_index_name("cortex-decisions"));
        assert!(!is_canonical_index_name("cortex-cortex-bogus"));
        assert!(!is_canonical_index_name("legacy-foo"));
        assert!(!is_canonical_index_name("cortex-"));
        assert!(!is_canonical_index_name("cortex-cortex-"));
    }

    #[test]
    fn known_stale_audit_indexes_are_non_canonical() {
        // The seven names captured in the 2026-04-27 audit + the
        // post-audit `cortex-analyses` orphan that landed before the
        // per-project naming migration was applied to the analyses
        // family. Each must fail the canonical check so the sweep
        // targets them.
        for name in [
            "cortex-code",
            "cortex-decisions",
            "cortex-docs",
            "cortex-governance",
            "cortex-misc",
            "cortex-turns",
            "cortex-analyses",
        ] {
            assert!(
                !is_canonical_index_name(name),
                "expected {name} to be non-canonical",
            );
        }
    }

    #[test]
    fn index_name_always_round_trips_through_canonical_check() {
        // Property-style sweep: every (repo_slug, family) pair in the
        // canonical vocabulary produces a name that the canonical
        // matcher accepts. Catches drift between writer and sweeper.
        let slugs = ["cortex", "rulebook", "nexus", "cortex-mcp", "tml_v2"];
        for slug in &slugs {
            for family in FAMILIES {
                let name = index_name("cortex-", slug, family);
                assert!(
                    is_canonical_index_name(&name),
                    "round-trip failed for slug={slug:?} family={family:?} → {name:?}",
                );
            }
        }
    }

    #[test]
    fn matrix_covers_every_event_kind_per_spec_08() {
        // Spec-08 §Routing matrix lists six destinations. Pin every
        // Kind variant to a known family so a future Kind addition
        // forces an explicit routing decision instead of silently
        // landing in `misc`. Non-artifact branches use kind-only
        // routing; the artifact tie-break is covered separately.
        for (kind, family) in [
            (Kind::Decision, "decisions"),
            (Kind::LawViolation, "governance"),
            (Kind::Turn, "turns"),
            (Kind::AgentCall, "turns"),
            (Kind::ToolCall, "code"),
            (Kind::Memory, "misc"),
            (Kind::Analysis, "analyses"),
            (Kind::Artifact, "misc"),
        ] {
            assert_eq!(family_for(kind), family, "kind={kind:?} drift");
        }
    }
}

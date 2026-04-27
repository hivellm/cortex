//! Per-kind, per-repo index routing.
//!
//! Spec 08 §Indexes lays out the family-to-index mapping. Index names
//! embed the owning repo's slug so each project lives in isolation —
//! `cortex-{repo_slug}-{family}` — and queries can scope deterministically
//! to a single project.

use cortex_core::events::Kind;
use cortex_storage::names::{slug_for_repo, UNKNOWN_REPO_SLUG};

use crate::EnrichedEvent;

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
/// `law_violation` ⇒ `governance`, `artifact` ⇒ `misc` (forced into
/// the topic-aware path when called via [`family_for_event`]),
/// anything else ⇒ `misc`.
pub fn family_for(kind: Kind) -> &'static str {
    match kind {
        Kind::ToolCall | Kind::AgentCall => "code",
        Kind::Decision => "decisions",
        Kind::Turn => "turns",
        Kind::LawViolation => "governance",
        Kind::Artifact => "misc",
        Kind::Memory | Kind::Analysis => "misc",
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
pub fn index_name(prefix: &str, repo_slug: &str, family: &str) -> String {
    let trimmed = prefix.trim_end_matches('-');
    format!("{trimmed}-{repo_slug}-{family}")
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
pub const FAMILIES: &[&str] = &["code", "docs", "decisions", "turns", "governance", "misc"];

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
        assert_eq!(family_for_event(Kind::AgentCall, &[], None), "code");
    }
}

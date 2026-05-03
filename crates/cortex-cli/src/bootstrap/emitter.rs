//! Synthetic event emitter — turns walked files and commits into
//! envelope-compliant events on `cortex.events.bootstrap`.
//!
//! Mirrors the shapes in `docs/specs/09-bootstrap-cli.md` §Synthetic
//! event shape: `artifact.code`, `artifact.doc`, `turn.historical`,
//! `decision.imported`, `law.imported`, `memory.imported`. Every event
//! routes through `cortex_core::redact` before publication and carries
//! `content_hash = sha256(canonical_json(redacted_payload))` so
//! downstream writers (specs 06, 07, 08) can dedupe re-runs.

use chrono::{DateTime, Utc};
use cortex_core::canonical_json::canonicalize;
use cortex_core::redact::{redact, RedactReport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::git::CommitRecord;
use super::walker::{FileClass, WalkEntry};

/// Stream name spec 09 §Synthetic event shape declares as the sink for
/// every bootstrap event. Overridable at the CLI via `--stream`.
pub const BOOTSTRAP_STREAM: &str = "cortex.events.bootstrap";

/// Shape of a bootstrap event ready for publication. Mirrors spec 01
/// envelope plus the per-kind payload spec 09 declares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapEvent {
    /// ULID. Always client-generated in bootstrap mode.
    pub event_id: String,
    /// Event-creation timestamp (ms since epoch).
    pub ts: i64,
    /// Coarse event kind string (`artifact.code`, `turn.historical`, …).
    pub kind: String,
    /// Always `"bootstrap"` for events emitted by this crate.
    pub adapter: String,
    /// Stream the publisher targets. Defaults to [`BOOTSTRAP_STREAM`].
    pub stream: String,
    /// Owning bootstrap-run session id. Generated once per
    /// `run_repo` call and reused for every event the run emits, so
    /// the graph writer collapses them under a single Session node
    /// (`HAS_TURN(Session→Turn)`, `HAS_ARTIFACT(Session→Artifact)`)
    /// instead of one synthetic session per event.
    pub session_id: String,
    /// Provenance — repo, path, symbol, byte_range, git_ref.
    pub source: Value,
    /// Per-kind payload after redaction.
    pub redacted_payload: Value,
    /// `sha256(canonical_json(redacted_payload))`.
    pub content_hash: String,
    /// Redaction stats produced by [`redact`].
    #[serde(default)]
    pub redactions: u32,
}

/// Phase11e §5 — match a `BootstrapEvent.kind` (e.g.
/// `"decision.imported"` / `"artifact.code"` / `"turn.historical"`)
/// against a user-facing family token (e.g. `"decisions"` /
/// `"code"` / `"turns"`).
///
/// Returns `true` when the token matches the kind. The token is
/// case-insensitive and matches plurals / family suffixes per
/// spec 08 §Routing matrix.
pub fn kind_token_matches(token: &str, kind: &str) -> bool {
    let t = token.trim().to_ascii_lowercase();
    let k = kind.trim().to_ascii_lowercase();
    match t.as_str() {
        "decisions" | "decision" => k.starts_with("decision."),
        "turns" | "turn" => k.starts_with("turn."),
        "laws" | "law" | "governance" => k.starts_with("law."),
        "analyses" | "analysis" => k.starts_with("analysis."),
        "memory" | "memories" => k.starts_with("memory."),
        "knowledge" => k.starts_with("knowledge."),
        "learnings" | "learning" => k.starts_with("learning."),
        "code" => k == "artifact.code",
        "docs" | "doc" => k == "artifact.doc",
        "artifacts" | "artifact" => k.starts_with("artifact."),
        _ => false,
    }
}

/// Phase11e §5 — apply a CSV-style kind filter (already split into
/// tokens) to one event's kind label. An empty filter list passes
/// every kind through; a non-empty list keeps only events whose
/// kind matches at least one token via [`kind_token_matches`].
pub fn kind_passes_filter(filter_tokens: &[String], kind: &str) -> bool {
    if filter_tokens.is_empty() {
        return true;
    }
    filter_tokens.iter().any(|t| kind_token_matches(t, kind))
}

/// Build a single envelope from a set of inputs already prepared by
/// the caller. Centralises the redact + canonical-hash pipeline so
/// every kind reaches the wire identically.
fn finalise(
    kind: &str,
    session_id: &str,
    source: Value,
    mut payload: Value,
    stream: &str,
) -> BootstrapEvent {
    let report: RedactReport = redact(&mut payload);
    let hash = canonical_sha256(&payload);
    let now: DateTime<Utc> = Utc::now();
    BootstrapEvent {
        event_id: ulid::Ulid::new().to_string(),
        ts: now.timestamp_millis(),
        kind: kind.to_string(),
        adapter: "bootstrap".to_string(),
        stream: stream.to_string(),
        session_id: session_id.to_string(),
        source,
        redacted_payload: payload,
        content_hash: hash,
        redactions: u32::try_from(report.tokens.len()).unwrap_or(u32::MAX),
    }
}

/// Build the `artifact.code` event for one accepted code file.
pub fn emit_artifact_code(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    language: Option<&str>,
    stream: &str,
) -> Option<BootstrapEvent> {
    let WalkEntry::Accepted {
        rel_path,
        size_bytes,
        ..
    } = entry
    else {
        return None;
    };
    let source = build_source(repo_id, Some(rel_path), git_ref, None, Some(*size_bytes));
    let payload = json!({
        "text": body,
        "language": language,
    });
    Some(finalise(
        "artifact.code",
        session_id,
        source,
        payload,
        stream,
    ))
}

/// Build the `artifact.doc` event for one accepted documentation file.
pub fn emit_artifact_doc(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    stream: &str,
) -> Option<BootstrapEvent> {
    let WalkEntry::Accepted {
        rel_path,
        size_bytes,
        ..
    } = entry
    else {
        return None;
    };
    let source = build_source(repo_id, Some(rel_path), git_ref, None, Some(*size_bytes));
    let payload = json!({
        "text": body,
        "title": derive_doc_title(body, rel_path),
    });
    Some(finalise(
        "artifact.doc",
        session_id,
        source,
        payload,
        stream,
    ))
}

/// Build the `turn.historical` event for one git commit.
pub fn emit_turn_historical(
    repo_id: &str,
    session_id: &str,
    commit: &CommitRecord,
    stream: &str,
) -> BootstrapEvent {
    let source = json!({
        "repo": repo_id,
        "git_ref": commit.sha,
        "author": commit.author_email,
    });
    let mut message = commit.subject.clone();
    if !commit.body.is_empty() {
        message.push_str("\n\n");
        message.push_str(&commit.body);
    }
    let payload = json!({
        "role": "developer",
        "message": message,
        "evidence": {
            "files_changed": commit.files_changed,
            "diff_summary": commit.diff_summary(),
        },
    });
    let mut event = finalise("turn.historical", session_id, source, payload, stream);
    // Override timestamp to match the commit's author time. Newer
    // events keep `now`; historical turns must carry their authored
    // moment so the dashboard sorts them where they belong.
    event.ts = commit.author_ts.saturating_mul(1000);
    event
}

/// Build the `decision.imported` event for one ADR / OpenSpec body.
///
/// phase10i — the front-matter parser extracts `author`, `Date`
/// (→ RFC-3339 `occurred_at`), and `Related Tasks` / `Related
/// Analyses` (→ `source_analysis`) so the dashboard can sort
/// decisions by date, surface the author, and trace each ADR
/// back to the analysis that produced it.
pub fn emit_decision_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let parsed = parse_decision_markdown(body, rel_path);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    // Phase11p hotfix — `DecisionPayload::decision_id` is REQUIRED for
    // the spec-11 lane projection contract (phase11k §1.2). Without
    // it, the workers' `serde_json::from_value::<DecisionPayload>(...)`
    // parse fails and `apply_top_level_projection` silently skips
    // stamping `decision_id` / `decision_title` / `decision_status`
    // on the Meili document. Synthesise the id from the filename
    // stem (matches the existing law-id convention in
    // `emit_law_imported`); the upstream ADR registry uses the same
    // shape.
    let decision_id = filename_stem(rel_path);
    let mut payload = json!({
        "decision_id": decision_id,
        "title": parsed.title,
        "status": parsed.status,
        "supersedes": parsed.supersedes,
        "body": body,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(author) = parsed.author {
            map.insert("author".into(), Value::String(author));
        }
        if let Some(occurred_at) = parsed.occurred_at {
            map.insert("occurred_at".into(), Value::String(occurred_at));
        }
        if let Some(source_analysis) = parsed.source_analysis {
            map.insert("source_analysis".into(), Value::String(source_analysis));
        }
    }
    finalise("decision.imported", session_id, source, payload, stream)
}

/// Build the `law.imported` event for one law / rule file.
pub fn emit_law_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let parsed = parse_law(body, rel_path);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let payload = json!({
        "law_id": parsed.law_id,
        "title": parsed.title,
        "severity": parsed.severity,
        "detector": parsed.detector,
        "body": body,
    });
    finalise("law.imported", session_id, source, payload, stream)
}

/// Split a spec doc (`.rulebook/specs/**/*.md`) into one
/// `law.imported` event per top-level `## ` section, so each
/// rule lands in the graph + dashboard as an addressable law
/// instead of one giant catch-all envelope. Spec-09 §Rulebook
/// artifact indexing — closes the `laws_active = 0` gap captured
/// in the 2026-04-27 audit.
///
/// The split rule:
/// - Look for `## ` lines at column 0. Each one starts a new law.
/// - The first heading text becomes the law title.
/// - The body of the law is everything from the heading up to the
///   next `## ` (or EOF).
/// - Synthesise `law_id = LAW-{filename-stem}-{section-slug}`
///   uppercase, kebab-cased — operators can grep for the exact
///   prefix to find every law from a given spec doc.
/// - When the doc has zero `## ` sections, fall back to a single
///   law.imported via [`emit_law_imported`].
pub fn emit_spec_laws_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> Vec<BootstrapEvent> {
    let stem = filename_stem(rel_path);
    let stem_upper = stem.to_ascii_uppercase().replace('_', "-");

    // Split into sections by the first column-0 `## ` boundary.
    // Lines before the first `## ` are the doc's preamble — they
    // become a "preamble" law if they contain real content (more
    // than just a title + blank line). Otherwise they're dropped.
    // Anything before the first `## ` is doc preamble (H1 title +
    // intro paragraphs) — useful context but not itself a law, so
    // we drop it. The dashboard already has per-spec memory entries
    // capturing the full doc; this path only needs to materialise
    // the addressable laws.
    let mut sections: Vec<(String, String)> = Vec::new(); // (title, full body)
    let mut current_title: Option<String> = None;
    let mut current_buf = String::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // Flush previous section if any. Preamble (current_title
            // is None) is intentionally discarded.
            if let Some(t) = current_title.take() {
                sections.push((t, current_buf.clone()));
            }
            current_buf.clear();
            current_title = Some(rest.trim().to_string());
            current_buf.push_str(line);
            current_buf.push('\n');
            continue;
        }
        current_buf.push_str(line);
        current_buf.push('\n');
    }
    if let Some(t) = current_title.take() {
        sections.push((t, current_buf.clone()));
    }

    // No `## ` boundaries at all → fall back to single law per file.
    if sections.is_empty() {
        return vec![emit_law_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )];
    }

    sections
        .into_iter()
        .enumerate()
        .map(|(idx, (title, section_body))| {
            let slug = slug_from_title(&title);
            let law_id = format!("LAW-{stem_upper}-{:02}-{slug}", idx + 1);
            let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
            let payload = json!({
                "law_id": law_id,
                "title": title,
                "severity": "info",
                "detector": Value::Null,
                "body": section_body,
                "section_index": idx,
                "source_path": rel_path,
            });
            finalise("law.imported", session_id, source, payload, stream)
        })
        .collect()
}

/// Phase11k §3.2 — emit one `law.imported` envelope per `## `
/// heading whose first token matches the project's
/// `[cortex.laws].extract_pattern`. When the body has no matching
/// heading, falls back to a single-law envelope via
/// [`emit_law_imported`] so files that pre-date the extract path
/// still produce something.
///
/// The pattern check is deliberately narrow: only the regex shipped
/// in `cortex.toml` (`^LAW-[A-Z0-9-]+$`) is supported without
/// pulling in the `regex` crate. The implementation tests each
/// heading's first whitespace-delimited token for the prefix
/// (`LAW-`) plus the character class (`[A-Z0-9-]`). This is the
/// only pattern shipped in the spec; if a future config wants a
/// richer one the regex crate can be added then.
pub fn emit_extracted_laws_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    pattern: &str,
    stream: &str,
) -> Vec<BootstrapEvent> {
    let extractor = LawTokenExtractor::compile(pattern);
    let mut sections: Vec<(String, String, String)> = Vec::new(); // (law_id, title, body)
    let mut current: Option<(String, String, String)> = None;
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let trimmed = rest.trim();
            if let Some(token) = extractor.match_first_token(trimmed) {
                if let Some(prev) = current.take() {
                    sections.push(prev);
                }
                let mut buf = String::new();
                buf.push_str(line);
                buf.push('\n');
                current = Some((token.to_string(), trimmed.to_string(), buf));
                continue;
            }
        }
        if let Some((_, _, ref mut buf)) = current {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(prev) = current.take() {
        sections.push(prev);
    }

    if sections.is_empty() {
        // No heading matched the pattern — fall back so the file's
        // contents still reach the dashboard as a single law envelope.
        return vec![emit_law_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )];
    }

    sections
        .into_iter()
        .enumerate()
        .map(|(idx, (law_id, title, section_body))| {
            let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
            let payload = json!({
                "law_id": law_id,
                "title": title,
                "severity": "critical",
                "detector": Value::Null,
                "body": section_body,
                "section_index": idx,
                "source_path": rel_path,
            });
            finalise("law.imported", session_id, source, payload, stream)
        })
        .collect()
}

/// Phase11k §3.2 — minimal extractor for the shipped
/// `^LAW-[A-Z0-9-]+$` pattern. Avoids pulling the `regex` crate for
/// a single fixed shape. Future patterns would either add `regex`
/// or extend this matcher.
struct LawTokenExtractor {
    /// Pattern after stripping the `^` / `$` anchors so we can match
    /// against a whitespace-delimited token directly.
    inner: String,
}

impl LawTokenExtractor {
    fn compile(pattern: &str) -> Self {
        let trimmed = pattern.trim();
        let after_caret = trimmed.strip_prefix('^').unwrap_or(trimmed);
        let stripped = after_caret.strip_suffix('$').unwrap_or(after_caret);
        Self {
            inner: stripped.to_string(),
        }
    }

    /// Return the first whitespace-delimited token in `heading` when
    /// it satisfies the configured pattern, else `None`. The shipped
    /// pattern (`LAW-[A-Z0-9-]+`) is the only shape tested here:
    /// `LAW-` prefix followed by one-or-more `[A-Z0-9-]` chars.
    fn match_first_token<'a>(&self, heading: &'a str) -> Option<&'a str> {
        let token = heading.split_whitespace().next()?;
        // The shipped pattern is LAW- prefix + character class.
        // Reject tokens that don't start with LAW- regardless of the
        // configured pattern's prefix — broadening this would require
        // the regex crate.
        if !self.inner.starts_with("LAW-") {
            return None;
        }
        let suffix = token.strip_prefix("LAW-")?;
        if suffix.is_empty() {
            return None;
        }
        if !suffix
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
        {
            return None;
        }
        Some(token)
    }
}

/// Detect spec-doc shape from a repo-rooted path. The walker
/// classifies `.rulebook/specs/**/*.md` as `FileClass::Law`; the
/// emitter dispatch uses this predicate to decide whether to take
/// the `emit_spec_laws_imported` (multi-law fan-out) path or the
/// single-law `emit_law_imported` path used by `.claude/rules/*.md`.
fn is_spec_doc_path(rel_path: &str) -> bool {
    let normalised = rel_path.replace('\\', "/");
    normalised.contains("/.rulebook/specs/") || normalised.starts_with(".rulebook/specs/")
}

/// Slugify a section title for use inside a synthesised law id.
/// Keeps `[a-z0-9-]`, lowercases, collapses runs of separators
/// to a single `-`, trims to 32 chars so the produced law id
/// stays readable.
fn slug_from_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut prev_dash = false;
    for ch in title.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_string()
    } else if out.len() > 32 {
        let mut clipped = String::new();
        for (i, ch) in out.chars().enumerate() {
            if i >= 32 {
                break;
            }
            clipped.push(ch);
        }
        while clipped.ends_with('-') {
            clipped.pop();
        }
        clipped
    } else {
        out
    }
}

/// Build the `analysis.imported` event for one audit / deep-analysis
/// report. Title comes from the first H1; status from a `Status:`
/// line if present (defaults to `draft`). The `body` field carries
/// the full markdown so downstream embedders + fulltext can chunk
/// it without re-reading the source file.
pub fn emit_analysis_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let title = derive_doc_title(body, rel_path);
    let status = derive_status(body).unwrap_or_else(|| "draft".to_string());
    // phase10i — pull `Author:` + `Date:` from the analysis
    // README's front-matter so downstream surfaces can attribute
    // + sort by date.
    let author = derive_author(body);
    let occurred_at = derive_occurred_at(body);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let mut payload = json!({
        "title": title,
        "status": status,
        "body": body,
        "source_path": rel_path,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(author) = author {
            map.insert("author".into(), Value::String(author));
        }
        if let Some(occurred_at) = occurred_at {
            map.insert("occurred_at".into(), Value::String(occurred_at));
        }
    }
    finalise("analysis.imported", session_id, source, payload, stream)
}

/// phase10i — extract the `Author:` value from a front-matter
/// block. Mirrors [`derive_status`] in scanning rules but
/// returns the entire trimmed value rather than the first token.
fn derive_author(body: &str) -> Option<String> {
    for line in body.lines().take(40) {
        let unbolded = unbold_label(line.trim());
        let trimmed: &str = &unbolded;
        if let Some(rest) = strip_prefix_ci(trimmed, "author:") {
            let v = rest.trim();
            if !v.is_empty() && !v.eq_ignore_ascii_case("unknown") {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// phase10i — extract the `Date:` value, normalising via
/// [`front_matter_date_to_rfc3339`] so the wire shape carries an
/// RFC-3339 timestamp regardless of whether the source used a
/// bare date or a full ISO-8601 string.
fn derive_occurred_at(body: &str) -> Option<String> {
    for line in body.lines().take(40) {
        let unbolded = unbold_label(line.trim());
        let trimmed: &str = &unbolded;
        if let Some(rest) = strip_prefix_ci(trimmed, "date:") {
            return front_matter_date_to_rfc3339(rest.trim());
        }
    }
    None
}

/// Extract a `Status:` line value from the body, case-insensitive,
/// matching the same loose front-matter convention `parse_decision_markdown`
/// uses. Returns the first token only — analysis bodies often follow
/// the status with a sentence ("draft. Indexed as ...") that we do NOT
/// want surfacing as a status badge.
fn derive_status(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(|c: char| c == '*' || c == '>' || c == '#' || c.is_whitespace());
        if let Some(rest) = strip_prefix_ci(trimmed, "status:") {
            let v = rest
                .trim()
                .trim_matches(|c: char| c == '*' || c == '"' || c == '_');
            // Take only the first whitespace- or punctuation-delimited
            // token. `draft. Indexed as ...` → `draft`. `accepted` →
            // `accepted`. `not started` → `not` (acceptable; the user
            // can write `not_started` or `not-started` to keep both).
            // `split` leaves empty leading slices when the string
            // starts with a delimiter (e.g. `** draft` after the
            // earlier `**` strip leaves a space prefix), so skip
            // empties before taking the first real token.
            let token = v
                .split(|c: char| c.is_whitespace() || c == '.' || c == ',' || c == ';')
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// phase10e — build the `knowledge.imported` event for one
/// `.rulebook/knowledge/**/*.md` entry. Discriminates `pattern`
/// vs `anti-pattern` from the path: nested directories like
/// `.rulebook/knowledge/anti-patterns/...` win over the parent
/// glob, the rest defaults to `pattern`. The synthetic
/// `knowledge_id` is `<repo>:<rel_path-stem>` so the
/// graph-mapper's natural-key writes survive a re-walk
/// idempotently.
pub fn emit_knowledge_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let title = derive_doc_title(body, rel_path);
    let category = if rel_path.contains("/anti-pattern") || rel_path.contains("/anti_pattern") {
        "anti-pattern"
    } else {
        "pattern"
    };
    let knowledge_id = synth_id(repo_id, rel_path);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let payload = json!({
        "knowledge_id": knowledge_id,
        "title": title,
        "category": category,
        "body": body,
        "source_path": rel_path,
    });
    finalise("knowledge.imported", session_id, source, payload, stream)
}

/// phase10e — build the `learning.imported` event for one
/// `.rulebook/learnings/**/*.md` entry. The synthetic
/// `learning_id` mirrors [`emit_knowledge_imported`].
pub fn emit_learning_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let title = derive_doc_title(body, rel_path);
    let learning_id = synth_id(repo_id, rel_path);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let payload = json!({
        "learning_id": learning_id,
        "title": title,
        "body": body,
        "source_path": rel_path,
    });
    finalise("learning.imported", session_id, source, payload, stream)
}

/// phase10e — synthesise a stable id from `(repo, rel_path)`. The
/// path's filename stem is the human-readable anchor; prefixing
/// the repo keeps cross-repo collisions out of Nexus.
fn synth_id(repo_id: &str, rel_path: &str) -> String {
    let stem = std::path::Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path);
    format!("{repo_id}:{stem}")
}

/// Build the `memory.imported` event for one memory file.
///
/// Field shape mirrors `cortex_core::events::MemoryPayload` so the
/// graph mapper deserialises it cleanly. Earlier revisions emitted
/// `{"title", "body"}`, which the mapper's
/// `serde_json::from_value::<MemoryPayload>(...)` rejected because
/// `op` / `memory_type` / `name` are required — every Memory node
/// fell back to the `"Memory <ulid12>"` display label and lost
/// connection between the bootstrap source and the searchable name.
/// `op = "write"` + `memory_type = "reference"` are the right
/// constants for bootstrap imports: the file is being written into
/// the index for reference, not authored as user / feedback /
/// project memory.
pub fn emit_memory_imported(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    rel_path: &str,
    body: &str,
    stream: &str,
) -> BootstrapEvent {
    let title = derive_doc_title(body, rel_path);
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let payload = json!({
        "op": "write",
        "memory_type": "reference",
        "name": title,
        "memory_path": rel_path,
        "body": body,
    });
    finalise("memory.imported", session_id, source, payload, stream)
}

fn build_source(
    repo_id: &str,
    path: Option<&str>,
    git_ref: Option<&str>,
    symbol: Option<&str>,
    bytes: Option<u64>,
) -> Value {
    let mut s = json!({ "repo": repo_id });
    if let Some(p) = path {
        s["path"] = Value::String(p.to_string());
    }
    if let Some(g) = git_ref {
        s["git_ref"] = Value::String(g.to_string());
    }
    if let Some(sym) = symbol {
        s["symbol"] = Value::String(sym.to_string());
    }
    if let Some(b) = bytes {
        s["bytes"] = Value::from(b);
    }
    s
}

fn canonical_sha256(value: &Value) -> String {
    let bytes = match canonicalize(value) {
        Ok(b) => b,
        Err(_) => serde_json::to_vec(value).unwrap_or_default(),
    };
    let mut h = Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let mut out = String::from("sha256:");
    for b in digest.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[derive(Debug, Clone)]
struct DecisionParsed {
    title: String,
    status: String,
    supersedes: Option<String>,
    /// phase10i — author from the `Author:` / `**Author**:`
    /// front-matter line. `None` when the ADR does not list one.
    author: Option<String>,
    /// phase10i — RFC-3339 timestamp parsed from the `Date:` /
    /// `**Date**:` front-matter. `None` when missing or unparseable.
    occurred_at: Option<String>,
    /// phase10i — `docs/analysis/<slug>` path scraped from the
    /// `Related Tasks:` / `Related Analyses:` front-matter. The
    /// graph mapper uses this to attach the decision to the
    /// analysis that produced it.
    source_analysis: Option<String>,
}

fn parse_decision_markdown(body: &str, rel_path: &str) -> DecisionParsed {
    let mut title: Option<String> = None;
    let mut status: Option<String> = None;
    let mut supersedes: Option<String> = None;
    let mut author: Option<String> = None;
    let mut occurred_at: Option<String> = None;
    let mut source_analysis: Option<String> = None;
    for line in body.lines() {
        // phase10i — handle the bolded-label markdown convention
        // (`**Status**: accepted`) alongside the bare
        // (`Status: accepted`) form. `unbold_label` collapses
        // `**LABEL**: value` to `LABEL: value` so the
        // case-insensitive prefix scanner matches both shapes
        // uniformly.
        let unbolded = unbold_label(line.trim());
        let trimmed: &str = &unbolded;
        if title.is_none() {
            if let Some(rest) = trimmed.strip_prefix("# ") {
                title = Some(rest.trim().to_string());
                continue;
            }
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "status:") {
            if status.is_none() {
                status = Some(rest.trim().to_string());
            }
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "supersedes:") {
            if supersedes.is_none() {
                let v = rest.trim();
                if !v.is_empty() && !v.eq_ignore_ascii_case("none") {
                    supersedes = Some(v.to_string());
                }
            }
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "author:") {
            if author.is_none() {
                let v = rest.trim();
                if !v.is_empty() && !v.eq_ignore_ascii_case("unknown") {
                    author = Some(v.to_string());
                }
            }
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "date:") {
            if occurred_at.is_none() {
                occurred_at = front_matter_date_to_rfc3339(rest.trim());
            }
        }
        // `Related Tasks` / `Related Analyses` may list multiple
        // entries; capture the first `docs/analysis/...` mention
        // so the graph mapper can attach the decision to its
        // source.
        if source_analysis.is_none() {
            for label in ["related tasks:", "related analyses:", "source analysis:"] {
                if let Some(rest) = strip_prefix_ci(trimmed, label) {
                    if let Some(slug) = extract_analysis_slug(rest) {
                        source_analysis = Some(slug);
                    }
                }
            }
        }
    }
    DecisionParsed {
        title: title.unwrap_or_else(|| filename_stem(rel_path)),
        status: status.unwrap_or_else(|| "proposed".to_string()),
        supersedes,
        author,
        occurred_at,
        source_analysis,
    }
}

/// phase10i — collapse a leading `**LABEL**` wrapper into bare
/// `LABEL` so `**Status**: accepted` and `Status: accepted`
/// flow through the same `strip_prefix_ci` scanner. Returns the
/// input unchanged when no matching `**…**` opener-closer pair
/// sits before the first `:` (defensive: `bold *italic*: not a
/// label` rounds through verbatim).
fn unbold_label(line: &str) -> std::borrow::Cow<'_, str> {
    let Some(after_open) = line.strip_prefix("**") else {
        return std::borrow::Cow::Borrowed(line);
    };
    let close = match after_open.find("**") {
        Some(i) => i,
        None => return std::borrow::Cow::Borrowed(line),
    };
    // Reject lines whose `:` lands inside / before the opener-
    // closer pair (the `**` wrapper isn't around the label).
    if let Some(colon) = after_open.find(':') {
        if colon < close {
            return std::borrow::Cow::Borrowed(line);
        }
    }
    let label = &after_open[..close];
    let tail = &after_open[close + 2..];
    std::borrow::Cow::Owned(format!("{label}{tail}"))
}

/// phase10i — parse a front-matter `Date:` value into an RFC-3339
/// timestamp. Accepts `YYYY-MM-DD` (most common ADR shape) and
/// passes already-RFC-3339 inputs through unchanged. Returns
/// `None` for unparseable values so the caller treats the field
/// as absent rather than stamping garbage.
fn front_matter_date_to_rfc3339(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Already RFC-3339 / ISO-8601 with time + zone.
    if chrono::DateTime::parse_from_rfc3339(raw).is_ok() {
        return Some(raw.to_string());
    }
    // YYYY-MM-DD → midnight UTC of that day.
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(format!("{}T00:00:00Z", date.format("%Y-%m-%d")));
    }
    None
}

/// phase10i — pull the first `docs/analysis/<slug>` path from a
/// `Related Tasks` / `Related Analyses` front-matter line. The
/// scanner accepts comma-separated and whitespace-separated
/// lists alike. Returns `None` when nothing matches.
fn extract_analysis_slug(line: &str) -> Option<String> {
    for token in line.split(|c: char| c == ',' || c.is_whitespace()) {
        let token =
            token.trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c == '(' || c == ')');
        if token.starts_with("docs/analysis/") {
            return Some(token.to_string());
        }
    }
    None
}

/// Case-insensitive prefix strip that returns the remainder of the
/// original string (preserving case in the returned slice).
///
/// Byte-safe against multi-byte UTF-8: slicing `haystack[..prefix.len()]`
/// directly panics when the haystack starts with a multi-byte character
/// shorter than the prefix (e.g. `# 01 — Overview` slicing for a 7-byte
/// `"status:"` lands inside the em-dash). The `prefix` is ASCII by
/// construction (every callsite passes a literal like `"status:"`), so
/// we test the prefix length in *characters* against the haystack's
/// leading bytes, falling back to `is_char_boundary` to refuse slices
/// that would split a code point.
fn strip_prefix_ci<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    let n = prefix.len();
    if haystack.len() < n {
        return None;
    }
    if !haystack.is_char_boundary(n) {
        return None;
    }
    if haystack[..n].eq_ignore_ascii_case(prefix) {
        Some(&haystack[n..])
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct LawParsed {
    law_id: String,
    title: String,
    severity: String,
    detector: Option<String>,
}

fn parse_law(body: &str, rel_path: &str) -> LawParsed {
    // Light-weight key-value scanner — handles both YAML (`key: value`)
    // and Markdown front-matter without taking on a YAML dep just for
    // bootstrap. Anything richer falls back to filename-derived keys.
    let mut law_id: Option<String> = None;
    let mut title: Option<String> = None;
    let mut severity: Option<String> = None;
    let mut detector: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_prefix_ci(trimmed, "law_id:") {
            law_id.get_or_insert_with(|| rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = strip_prefix_ci(trimmed, "title:") {
            title.get_or_insert_with(|| rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = strip_prefix_ci(trimmed, "severity:") {
            severity.get_or_insert_with(|| rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = strip_prefix_ci(trimmed, "detector:") {
            detector.get_or_insert_with(|| rest.trim().trim_matches('"').to_string());
        }
    }
    let stem = filename_stem(rel_path);
    let upper_stem = stem.to_ascii_uppercase();
    LawParsed {
        law_id: law_id.unwrap_or(upper_stem.clone()),
        title: title.unwrap_or(upper_stem),
        severity: severity.unwrap_or_else(|| "info".to_string()),
        detector,
    }
}

fn derive_doc_title(body: &str, rel_path: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let t = rest.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    filename_stem(rel_path)
}

fn filename_stem(rel_path: &str) -> String {
    std::path::Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string()
}

/// Pure-function emitter dispatch for one walked file. Returns `None`
/// if the entry is `Dropped` or carries an empty body. Single-event
/// shape — for files that fan out into multiple events (currently
/// `.rulebook/specs/**/*.md` → one law per `## ` section), use
/// [`emit_for_file_multi`] which returns the full list. This entry
/// point keeps the historical contract for callers that don't know
/// about the fan-out yet.
pub fn emit_for_file(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    stream: &str,
) -> Option<BootstrapEvent> {
    emit_for_file_multi(repo_id, session_id, git_ref, entry, body, stream)
        .into_iter()
        .next()
}

/// Pure-function emitter dispatch returning every event the walked
/// file produces. Most file classes still produce a single event
/// (decision, memory, analysis, code/doc artifact); spec docs under
/// `.rulebook/specs/**/*.md` fan out into one `law.imported` per
/// top-level `## ` section so each rule lands in the dashboard as
/// an addressable law (closes spec-11 §`laws_active = 0` gap).
pub fn emit_for_file_multi(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    stream: &str,
) -> Vec<BootstrapEvent> {
    emit_for_file_multi_with_extract(
        repo_id, session_id, git_ref, entry, body, None, stream,
    )
}

/// Phase11k §3.2 — same as [`emit_for_file_multi`] but with an
/// optional `[cortex.laws].extract_pattern` plumbed through the Law
/// dispatch arm. When set AND the file is Law-classified AND not a
/// spec doc, the body is split per matching `## ` heading. When
/// unset, behaviour matches the legacy [`emit_for_file_multi`]
/// surface (single-law-per-file for `.claude/rules/*.md` etc.).
pub fn emit_for_file_multi_with_extract(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    laws_extract_pattern: Option<&str>,
    stream: &str,
) -> Vec<BootstrapEvent> {
    let WalkEntry::Accepted {
        rel_path, class, ..
    } = entry
    else {
        return Vec::new();
    };
    if body.trim().is_empty() {
        return Vec::new();
    }
    match class {
        FileClass::Code => emit_artifact_code(
            repo_id,
            session_id,
            git_ref,
            entry,
            body,
            language_for(rel_path).as_deref(),
            stream,
        )
        .into_iter()
        .collect(),
        FileClass::Doc => emit_artifact_doc(repo_id, session_id, git_ref, entry, body, stream)
            .into_iter()
            .collect(),
        FileClass::Decision => vec![emit_decision_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )],
        FileClass::Law => {
            // Spec docs fan out into per-section laws; AGENTS-style
            // files with an `extract_pattern` fan out per-LAW heading;
            // everything else (`.claude/rules/*.md`, `[cortex.laws]`
            // patterns without extract) stays single-law.
            if is_spec_doc_path(rel_path) {
                emit_spec_laws_imported(repo_id, session_id, git_ref, rel_path, body, stream)
            } else if let Some(pattern) = laws_extract_pattern.filter(|p| !p.is_empty()) {
                emit_extracted_laws_imported(
                    repo_id, session_id, git_ref, rel_path, body, pattern, stream,
                )
            } else {
                vec![emit_law_imported(
                    repo_id, session_id, git_ref, rel_path, body, stream,
                )]
            }
        }
        FileClass::Memory => vec![emit_memory_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )],
        FileClass::Analysis => vec![emit_analysis_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )],
        // phase10e — dedicated kinds; emitted per-file with the
        // rel_path-derived synthetic id so re-walks remain
        // idempotent.
        FileClass::Knowledge => vec![emit_knowledge_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )],
        FileClass::Learning => vec![emit_learning_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )],
        FileClass::Other => Vec::new(),
    }
}

fn language_for(rel_path: &str) -> Option<String> {
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "h" => "c",
        "rb" => "ruby",
        "ex" | "exs" => "elixir",
        "kt" => "kotlin",
        "swift" => "swift",
        "scala" => "scala",
        "cs" => "csharp",
        "php" => "php",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "sh" | "bash" => "bash",
        _ => return None,
    };
    Some(lang.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(rel: &str, class: FileClass, size: u64) -> WalkEntry {
        WalkEntry::Accepted {
            path: PathBuf::from(rel),
            rel_path: rel.to_string(),
            size_bytes: size,
            class,
        }
    }

    // Phase11e §5 — kind filter helpers.
    #[test]
    fn kind_token_matches_resolves_every_documented_token() {
        // Family tokens map to the canonical bootstrap kind labels.
        assert!(kind_token_matches("decisions", "decision.imported"));
        assert!(kind_token_matches("decision", "decision.imported"));
        assert!(kind_token_matches("turns", "turn.historical"));
        assert!(kind_token_matches("laws", "law.imported"));
        assert!(kind_token_matches("governance", "law.imported"));
        assert!(kind_token_matches("analyses", "analysis.imported"));
        assert!(kind_token_matches("memory", "memory.imported"));
        assert!(kind_token_matches("knowledge", "knowledge.imported"));
        assert!(kind_token_matches("learnings", "learning.imported"));
        assert!(kind_token_matches("code", "artifact.code"));
        assert!(kind_token_matches("docs", "artifact.doc"));
        assert!(kind_token_matches("artifacts", "artifact.code"));
        assert!(kind_token_matches("artifacts", "artifact.doc"));
        // Negative — unknown / mismatched tokens.
        assert!(!kind_token_matches("decisions", "turn.historical"));
        assert!(!kind_token_matches("code", "artifact.doc"));
        assert!(!kind_token_matches("nonsense", "decision.imported"));
    }

    #[test]
    fn kind_token_matches_is_case_insensitive() {
        assert!(kind_token_matches("DECISIONS", "DECISION.IMPORTED"));
        assert!(kind_token_matches("Turns", "Turn.Historical"));
    }

    #[test]
    fn kind_passes_filter_lets_every_kind_through_when_filter_empty() {
        let no_filter: Vec<String> = vec![];
        assert!(kind_passes_filter(&no_filter, "decision.imported"));
        assert!(kind_passes_filter(&no_filter, "anything.really"));
    }

    #[test]
    fn kind_passes_filter_keeps_only_listed_tokens_when_filter_set() {
        let filter = vec!["decisions".to_string(), "turns".to_string()];
        assert!(kind_passes_filter(&filter, "decision.imported"));
        assert!(kind_passes_filter(&filter, "turn.historical"));
        assert!(!kind_passes_filter(&filter, "artifact.code"));
        assert!(!kind_passes_filter(&filter, "memory.imported"));
    }

    #[test]
    fn artifact_code_carries_language_and_text() {
        let e = entry("src/lib.rs", FileClass::Code, 42);
        let evt = emit_artifact_code(
            "Vectorizer",
            "01TESTSESSION0000000000000",
            Some("abc123"),
            &e,
            "fn main() {}",
            Some("rust"),
            BOOTSTRAP_STREAM,
        )
        .expect("must emit");
        assert_eq!(evt.kind, "artifact.code");
        assert_eq!(evt.adapter, "bootstrap");
        assert_eq!(evt.source["repo"], "Vectorizer");
        assert_eq!(evt.source["path"], "src/lib.rs");
        assert_eq!(evt.source["git_ref"], "abc123");
        assert_eq!(evt.redacted_payload["text"], "fn main() {}");
        assert_eq!(evt.redacted_payload["language"], "rust");
        assert!(evt.content_hash.starts_with("sha256:"));
    }

    #[test]
    fn turn_historical_uses_commit_timestamp() {
        let commit = CommitRecord {
            sha: "abcdef".into(),
            author_ts: 1_710_000_000,
            author_email: "a@b.c".into(),
            subject: "fix: tighten parser".into(),
            body: "More detail".into(),
            files_changed: vec!["src/parser.rs".into()],
        };
        let evt = emit_turn_historical(
            "Vectorizer",
            "01TESTSESSION0000000000000",
            &commit,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.kind, "turn.historical");
        assert_eq!(evt.ts, 1_710_000_000_000);
        assert_eq!(evt.source["author"], "a@b.c");
        assert_eq!(evt.source["git_ref"], "abcdef");
        assert!(evt.redacted_payload["message"]
            .as_str()
            .unwrap()
            .contains("fix: tighten parser"));
        assert_eq!(
            evt.redacted_payload["evidence"]["files_changed"][0],
            "src/parser.rs"
        );
    }

    #[test]
    fn decision_imported_extracts_title_status_supersedes() {
        let body = "# Adopt Meilisearch\n\nStatus: accepted\nSupersedes: ADR-0001\n\nBody.";
        let evt = emit_decision_imported(
            "Vectorizer",
            "01TESTSESSION0000000000000",
            None,
            "docs/decisions/0042.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.kind, "decision.imported");
        assert_eq!(evt.redacted_payload["title"], "Adopt Meilisearch");
        assert_eq!(evt.redacted_payload["status"], "accepted");
        assert_eq!(evt.redacted_payload["supersedes"], "ADR-0001");
    }

    #[test]
    fn decision_imported_phase10i_metadata_fields_round_trip() {
        // phase10i §2.2 — `Date:` (YYYY-MM-DD) lands as RFC-3339
        // `occurred_at`; `Author:` round-trips verbatim;
        // `Related Tasks:` mention of `docs/analysis/<slug>` lands
        // as `source_analysis`.
        let body = "# Adopt RRF fusion\n\
                    \n\
                    **Status**: accepted\n\
                    **Date**: 2026-04-22\n\
                    **Author**: Andre Ferreira\n\
                    **Related Tasks**: docs/analysis/relevance, phase10a_query_lane_wiring\n\
                    \n\
                    Body of the ADR.\n";
        let evt = emit_decision_imported(
            "cortex",
            "01TESTSESSION0000000000000",
            None,
            ".rulebook/decisions/0042-adopt-rrf-fusion.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.redacted_payload["occurred_at"], "2026-04-22T00:00:00Z");
        assert_eq!(evt.redacted_payload["author"], "Andre Ferreira");
        assert_eq!(
            evt.redacted_payload["source_analysis"],
            "docs/analysis/relevance"
        );
        assert_eq!(evt.redacted_payload["status"], "accepted");
    }

    #[test]
    fn decision_imported_skips_unknown_author_and_invalid_date() {
        // phase10i — `Author: Unknown` + a malformed `Date:` must
        // NOT stamp garbage on the wire. The fields stay absent so
        // the dashboard's date sort + author column degrade
        // gracefully instead of showing nonsense.
        let body = "# Skeleton\n\
                    \n\
                    Status: proposed\n\
                    Date: tomorrow\n\
                    Author: Unknown\n";
        let evt = emit_decision_imported(
            "cortex",
            "01TESTSESSION0000000000000",
            None,
            ".rulebook/decisions/0099-skel.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert!(
            evt.redacted_payload.get("occurred_at").is_none(),
            "malformed Date must not leak onto the wire"
        );
        assert!(
            evt.redacted_payload.get("author").is_none(),
            "Author = `Unknown` must not stamp"
        );
    }

    #[test]
    fn analysis_imported_phase10i_stamps_author_and_occurred_at() {
        let body = "# Relevance audit\n\
                    \n\
                    Status: complete\n\
                    Date: 2026-04-29\n\
                    Author: Andre Ferreira\n\
                    \n\
                    Findings...\n";
        let evt = emit_analysis_imported(
            "cortex",
            "01TESTSESSION0000000000000",
            None,
            "docs/analysis/relevance/01-findings.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.redacted_payload["occurred_at"], "2026-04-29T00:00:00Z");
        assert_eq!(evt.redacted_payload["author"], "Andre Ferreira");
    }

    #[test]
    fn decision_falls_back_to_filename_when_no_title() {
        let evt = emit_decision_imported(
            "Vectorizer",
            "01TESTSESSION0000000000000",
            None,
            "docs/decisions/no-title.md",
            "",
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.redacted_payload["title"], "no-title");
        assert_eq!(evt.redacted_payload["status"], "proposed");
        assert_eq!(evt.redacted_payload["supersedes"], serde_json::Value::Null);
    }

    #[test]
    fn law_imported_extracts_yaml_keys() {
        let body = "law_id: LAW-007\ntitle: No skipping hooks\nseverity: critical\ndetector: hook:pre_commit_no_skip\n";
        let evt = emit_law_imported(
            "Rulebook",
            "01TESTSESSION0000000000000",
            None,
            "rulebook/laws/LAW-007.yaml",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.kind, "law.imported");
        assert_eq!(evt.redacted_payload["law_id"], "LAW-007");
        assert_eq!(evt.redacted_payload["severity"], "critical");
        assert_eq!(evt.redacted_payload["detector"], "hook:pre_commit_no_skip");
    }

    #[test]
    fn law_falls_back_to_filename_stem() {
        let evt = emit_law_imported(
            "Rulebook",
            "01TESTSESSION0000000000000",
            None,
            "rulebook/laws/LAW-042.yaml",
            "",
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.redacted_payload["law_id"], "LAW-042");
    }

    #[test]
    fn derive_status_does_not_panic_on_multibyte_lines() {
        // Regression: pre-fix `strip_prefix_ci` sliced bytes without
        // a char-boundary check, panicking on the em-dash in lines
        // like `# 01 — Overview`. The fix makes the helper boundary-
        // safe; this test pins the behaviour.
        assert_eq!(
            derive_status("# 01 — Overview\n\nNo status line here."),
            None
        );
        assert_eq!(
            derive_status("# Title — with em-dash\n\nStatus: draft"),
            Some("draft".to_string())
        );
    }

    #[test]
    fn derive_status_returns_first_token_only() {
        // Analysis bodies often pad the status with a sentence; the
        // parser must keep only the badge token so the GUI does not
        // render a paragraph as a label.
        assert_eq!(
            derive_status("> **Status:** draft. Indexed as first-class entities by phase4e."),
            Some("draft".to_string())
        );
        assert_eq!(
            derive_status("Status: accepted, see DEC-0042"),
            Some("accepted".to_string())
        );
        assert_eq!(
            derive_status("> Status: SUPERSEDED"),
            Some("superseded".to_string())
        );
    }

    #[test]
    fn analysis_imported_extracts_title_and_status() {
        let body = "# Cortex — System Analysis (2026-04-28)\n\n> Status: draft\n\nBody.";
        let evt = emit_analysis_imported(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            "docs/analysis/cortex/00-index.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.kind, "analysis.imported");
        assert_eq!(
            evt.redacted_payload["title"],
            "Cortex — System Analysis (2026-04-28)"
        );
        assert_eq!(evt.redacted_payload["status"], "draft");
        assert_eq!(
            evt.redacted_payload["source_path"],
            "docs/analysis/cortex/00-index.md"
        );
        assert!(evt.redacted_payload["body"]
            .as_str()
            .unwrap()
            .starts_with("# Cortex"));
    }

    #[test]
    fn analysis_imported_defaults_status_to_draft() {
        let evt = emit_analysis_imported(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            "docs/analysis/cortex/02-pipeline-state.md",
            "# Pipeline State\n\nNo status line.",
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.redacted_payload["status"], "draft");
    }

    #[test]
    fn analysis_imported_routed_via_emit_for_file() {
        let e = WalkEntry::Accepted {
            path: PathBuf::from("docs/analysis/cortex/01-overview.md"),
            rel_path: "docs/analysis/cortex/01-overview.md".to_string(),
            size_bytes: 4242,
            class: FileClass::Analysis,
        };
        let evt = emit_for_file(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            &e,
            "# Overview\n\n2-3 paragraph audit body.",
            BOOTSTRAP_STREAM,
        )
        .expect("must emit");
        assert_eq!(evt.kind, "analysis.imported");
    }

    #[test]
    fn memory_imported_uses_h1_title() {
        let body = "# CLAUDE memory\n\nNote body.";
        let evt = emit_memory_imported(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            "CLAUDE.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(evt.kind, "memory.imported");
        // The payload mirrors `cortex_core::events::MemoryPayload` —
        // the H1 title surfaces as `name` (not `title`); see the
        // doc-comment on `emit_memory_imported` for why the field
        // was renamed from the legacy `title` shape.
        assert_eq!(evt.redacted_payload["name"], "CLAUDE memory");
    }

    #[test]
    fn content_hash_is_deterministic_for_same_payload() {
        let body = "# Same\n\nSame body.";
        let a = emit_decision_imported(
            "R",
            "01TESTSESSION0000000000000",
            None,
            "a.md",
            body,
            BOOTSTRAP_STREAM,
        );
        let b = emit_decision_imported(
            "R",
            "01TESTSESSION0000000000000",
            None,
            "a.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(a.content_hash, b.content_hash);
    }

    #[test]
    fn empty_body_returns_none_for_artifact_paths() {
        let e = entry("src/lib.rs", FileClass::Code, 0);
        let evt = emit_for_file(
            "R",
            "01TESTSESSION0000000000000",
            None,
            &e,
            "   \n  ",
            BOOTSTRAP_STREAM,
        );
        assert!(evt.is_none());
    }

    #[test]
    fn spec_doc_splits_into_one_law_per_top_level_heading() {
        let body = "# Rulebook\n\n\
                    Preamble.\n\n\
                    ## Critical Rules\n\n\
                    Body of rule one.\n\n\
                    More text.\n\n\
                    ## Task Workflow\n\n\
                    Body of rule two.\n\n\
                    ## Quality Gates\n\n\
                    Body of rule three.\n";
        let events = emit_spec_laws_imported(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            ".rulebook/specs/RULEBOOK.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(events.len(), 3, "one law per `## ` section");
        assert_eq!(events[0].kind, "law.imported");
        // Synthesised law_id encodes filename stem + section index + slug.
        let id0 = events[0].redacted_payload["law_id"].as_str().unwrap();
        assert!(
            id0.starts_with("LAW-RULEBOOK-01-"),
            "id0={id0} should start with LAW-RULEBOOK-01-"
        );
        assert_eq!(events[0].redacted_payload["title"], "Critical Rules");
        assert_eq!(events[1].redacted_payload["title"], "Task Workflow");
        assert_eq!(events[2].redacted_payload["title"], "Quality Gates");
        // Each event's body carries only its own section, not the
        // whole doc — the dashboard's law card stays focused.
        let body1 = events[1].redacted_payload["body"].as_str().unwrap();
        assert!(body1.contains("Body of rule two"));
        assert!(!body1.contains("Body of rule three"));
    }

    #[test]
    fn spec_doc_with_no_headings_falls_back_to_single_law() {
        let body = "Just a paragraph with no ## headings.\n\nAnother paragraph.\n";
        let events = emit_spec_laws_imported(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            ".rulebook/specs/FLAT.md",
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "law.imported");
        // Falls through to `emit_law_imported` which derives the
        // law_id from the filename stem.
        let id0 = events[0].redacted_payload["law_id"].as_str().unwrap();
        assert_eq!(id0, "FLAT");
    }

    #[test]
    fn emit_for_file_multi_routes_spec_paths_to_split_laws() {
        // `.rulebook/specs/**/*.md` walks as `FileClass::Law` (per
        // walker.rs) and the dispatch fans out into multiple laws.
        let e = WalkEntry::Accepted {
            path: PathBuf::from(".rulebook/specs/GIT.md"),
            rel_path: ".rulebook/specs/GIT.md".to_string(),
            size_bytes: 200,
            class: FileClass::Law,
        };
        let body =
            "# Git\n\n## Allowed\n\nstatus / diff / log.\n\n## Forbidden\n\nrebase / reset.\n";
        let events = emit_for_file_multi(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            &e,
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].redacted_payload["title"], "Allowed");
        assert_eq!(events[1].redacted_payload["title"], "Forbidden");
    }

    #[test]
    fn emit_for_file_multi_keeps_single_law_for_non_spec_paths() {
        // `.claude/rules/*.md` is a single-law path — fan-out only
        // applies to `.rulebook/specs/**`.
        let e = WalkEntry::Accepted {
            path: PathBuf::from(".claude/rules/diagnostic-first.md"),
            rel_path: ".claude/rules/diagnostic-first.md".to_string(),
            size_bytes: 100,
            class: FileClass::Law,
        };
        let body = "# Check before test\n\n## Allowed\n\nfine.\n";
        let events = emit_for_file_multi(
            "Cortex",
            "01TESTSESSION0000000000000",
            None,
            &e,
            body,
            BOOTSTRAP_STREAM,
        );
        assert_eq!(
            events.len(),
            1,
            ".claude/rules/*.md keeps the single-law-per-file shape"
        );
        assert_eq!(events[0].kind, "law.imported");
    }

    #[test]
    fn slug_from_title_handles_punctuation_and_unicode() {
        assert_eq!(slug_from_title("Critical Rules"), "critical-rules");
        assert_eq!(
            slug_from_title("PROHIBITION 1: No Shortcuts"),
            "prohibition-1-no-shortcuts"
        );
        assert_eq!(slug_from_title("   "), "section");
        let long = "a".repeat(60);
        assert!(slug_from_title(&long).len() <= 32);
    }
}

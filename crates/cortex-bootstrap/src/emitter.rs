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

use crate::git::CommitRecord;
use crate::walker::{FileClass, WalkEntry};

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
    Some(finalise("artifact.code", session_id, source, payload, stream))
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
    Some(finalise("artifact.doc", session_id, source, payload, stream))
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
    let payload = json!({
        "title": parsed.title,
        "status": parsed.status,
        "supersedes": parsed.supersedes,
        "body": body,
    });
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
    let source = build_source(repo_id, Some(rel_path), git_ref, None, None);
    let payload = json!({
        "title": title,
        "status": status,
        "body": body,
        "source_path": rel_path,
    });
    finalise("analysis.imported", session_id, source, payload, stream)
}

/// Extract a `Status:` line value from the body, case-insensitive,
/// matching the same loose front-matter convention `parse_decision_markdown`
/// uses. Returns `None` when no recognisable status line is present.
fn derive_status(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim().trim_start_matches(|c: char| c == '*' || c == '>' || c.is_whitespace());
        if let Some(rest) = strip_prefix_ci(trimmed, "status:") {
            let v = rest.trim().trim_matches(|c: char| c == '*' || c == '"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
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
}

fn parse_decision_markdown(body: &str, rel_path: &str) -> DecisionParsed {
    let mut title: Option<String> = None;
    let mut status: Option<String> = None;
    let mut supersedes: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
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
    }
    DecisionParsed {
        title: title.unwrap_or_else(|| filename_stem(rel_path)),
        status: status.unwrap_or_else(|| "proposed".to_string()),
        supersedes,
    }
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
/// if the entry is `Dropped` or carries an empty body.
pub fn emit_for_file(
    repo_id: &str,
    session_id: &str,
    git_ref: Option<&str>,
    entry: &WalkEntry,
    body: &str,
    stream: &str,
) -> Option<BootstrapEvent> {
    let WalkEntry::Accepted { rel_path, class, .. } = entry else {
        return None;
    };
    if body.trim().is_empty() {
        return None;
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
        ),
        FileClass::Doc => emit_artifact_doc(repo_id, session_id, git_ref, entry, body, stream),
        FileClass::Decision => Some(emit_decision_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )),
        FileClass::Law => Some(emit_law_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )),
        FileClass::Memory => Some(emit_memory_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )),
        FileClass::Analysis => Some(emit_analysis_imported(
            repo_id, session_id, git_ref, rel_path, body, stream,
        )),
        FileClass::Other => None,
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
        let evt =
            emit_turn_historical("Vectorizer", "01TESTSESSION0000000000000", &commit, BOOTSTRAP_STREAM);
        assert_eq!(evt.kind, "turn.historical");
        assert_eq!(evt.ts, 1_710_000_000_000);
        assert_eq!(evt.source["author"], "a@b.c");
        assert_eq!(evt.source["git_ref"], "abcdef");
        assert!(evt
            .redacted_payload["message"]
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
        assert_eq!(
            evt.redacted_payload["detector"],
            "hook:pre_commit_no_skip"
        );
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
        assert!(evt.redacted_payload["body"].as_str().unwrap().starts_with("# Cortex"));
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
        assert_eq!(evt.redacted_payload["title"], "CLAUDE memory");
    }

    #[test]
    fn content_hash_is_deterministic_for_same_payload() {
        let body = "# Same\n\nSame body.";
        let a = emit_decision_imported("R", "01TESTSESSION0000000000000", None, "a.md", body, BOOTSTRAP_STREAM);
        let b = emit_decision_imported("R", "01TESTSESSION0000000000000", None, "a.md", body, BOOTSTRAP_STREAM);
        assert_eq!(a.content_hash, b.content_hash);
    }

    #[test]
    fn empty_body_returns_none_for_artifact_paths() {
        let e = entry("src/lib.rs", FileClass::Code, 0);
        let evt = emit_for_file("R", "01TESTSESSION0000000000000", None, &e, "   \n  ", BOOTSTRAP_STREAM);
        assert!(evt.is_none());
    }
}

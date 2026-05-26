//! Phase14f §3 — implicit-feedback citation detector.
//!
//! The pipeline emits a bundle that lists a set of file paths
//! (under §Snippets / §Graph neighbours). When the model's reply
//! cites a path from that set, we treat it as a positive signal —
//! the bundle was useful enough to drive the reply. When NO bundle
//! path appears in the reply, the bundle either failed to surface
//! the right context OR the model ignored what was surfaced.
//!
//! [`detect_citation`] computes the Jaccard overlap between the
//! bundle's file set and the file-shaped tokens extracted from
//! the reply's first ~100 tokens. The score lands in `[0.0, 1.0]`
//! and persists alongside the explicit feedback row in
//! `pre_thinking_feedback.implicit_score`.
//!
//! The "first 100 tokens" rule prevents long replies from
//! diluting the signal: if the model cites a file inside an early
//! sentence, that's a strong signal; if it surfaces 30 paragraphs
//! later, the bundle may have been weakly relevant at best.

use std::collections::HashSet;

/// Per-row Jaccard overlap score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JaccardScore {
    /// Score in `[0.0, 1.0]`.
    pub score: f64,
    /// Number of bundle files cited in the reply window.
    pub matched: usize,
    /// Total distinct file paths the bundle listed.
    pub bundle_size: usize,
}

impl JaccardScore {
    /// Convenience accessor — the score is `0.0` when the bundle
    /// was empty OR when no overlap was detected.
    pub fn value(&self) -> f64 {
        self.score
    }
}

/// Pull file-shaped tokens out of `prefix`. A token is treated as
/// a file path when it contains a `/` or matches one of the
/// canonical extensions (rs, ts, tsx, js, py, go, java, json,
/// yaml, toml, md, sql). Returns a deduplicated set so repeated
/// citations of the same path don't inflate the overlap.
pub fn extract_file_tokens(prefix: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    // Tokenise on whitespace + a small set of markdown framing
    // characters so `[crate/foo.rs](...)` and `` `crate/foo.rs` ``
    // both surface the path cleanly.
    for raw in prefix.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '(' | ')' | '[' | ']' | '`' | ',' | ';' | '"' | '\'' | '<' | '>' | '|'
            )
    }) {
        let t = raw.trim_matches(|c: char| matches!(c, '.' | ':' | '#'));
        if t.is_empty() {
            continue;
        }
        if looks_like_path(t) {
            out.insert(t.to_string());
        }
    }
    out
}

fn looks_like_path(s: &str) -> bool {
    if s.contains('/') {
        return true;
    }
    // Single-segment files like `Cargo.toml` count when the
    // extension is on a known allow-list.
    let lower = s.to_ascii_lowercase();
    const EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".cs", ".cpp", ".c", ".hpp",
        ".h", ".json", ".yaml", ".yml", ".toml", ".md", ".sql", ".sh", ".ps1",
    ];
    EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Trim `reply` to the first `max_tokens` whitespace-separated
/// tokens. Cheap proxy for "first ~100 tokens" so the caller does
/// not pull a full BPE tokenizer in.
pub fn first_tokens(reply: &str, max_tokens: usize) -> String {
    if max_tokens == 0 || reply.is_empty() {
        return String::new();
    }
    let mut count = 0usize;
    let mut end = reply.len();
    let mut in_token = false;
    for (i, c) in reply.char_indices() {
        if c.is_whitespace() {
            if in_token {
                count += 1;
                if count >= max_tokens {
                    end = i;
                    break;
                }
                in_token = false;
            }
        } else {
            in_token = true;
        }
    }
    reply[..end].to_string()
}

/// Default token cap when the caller does not specify. Matches
/// the proposal's "first 100 tokens" heuristic.
pub const DEFAULT_REPLY_TOKEN_WINDOW: usize = 100;

/// Compute the Jaccard-overlap citation score. `bundle_files` is
/// the set of file paths the bundle surfaced; `reply` is the
/// model's first-tokens output. Window is bounded by
/// [`DEFAULT_REPLY_TOKEN_WINDOW`].
pub fn detect_citation(reply: &str, bundle_files: &[String]) -> JaccardScore {
    detect_citation_with_window(reply, bundle_files, DEFAULT_REPLY_TOKEN_WINDOW)
}

/// `detect_citation` variant with an explicit token window.
pub fn detect_citation_with_window(
    reply: &str,
    bundle_files: &[String],
    window: usize,
) -> JaccardScore {
    if bundle_files.is_empty() {
        return JaccardScore {
            score: 0.0,
            matched: 0,
            bundle_size: 0,
        };
    }
    let bundle_set: HashSet<&str> = bundle_files.iter().map(|s| s.as_str()).collect();
    let bundle_size = bundle_set.len();
    let prefix = first_tokens(reply, window);
    let reply_tokens = extract_file_tokens(&prefix);
    if reply_tokens.is_empty() {
        return JaccardScore {
            score: 0.0,
            matched: 0,
            bundle_size,
        };
    }
    let intersection = reply_tokens
        .iter()
        .filter(|t| bundle_set.contains(t.as_str()))
        .count();
    let union = bundle_set
        .iter()
        .map(|s| s.to_string())
        .chain(reply_tokens.iter().cloned())
        .collect::<HashSet<_>>()
        .len();
    let score = if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    };
    JaccardScore {
        score,
        matched: intersection,
        bundle_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn extract_file_tokens_finds_path_like_strings() {
        let p = "Edit the file crate/foo.rs and then run `cargo test`. See Cargo.toml.";
        let t = extract_file_tokens(p);
        assert!(t.contains("crate/foo.rs"));
        assert!(t.contains("Cargo.toml"));
        assert!(!t.contains("Edit"));
    }

    #[test]
    fn detect_citation_perfect_overlap_scores_one() {
        let bundle = s(&["src/a.rs", "src/b.rs"]);
        let reply = "Patched src/a.rs and src/b.rs to fix the regression.";
        let r = detect_citation(reply, &bundle);
        assert_eq!(r.matched, 2);
        assert_eq!(r.bundle_size, 2);
        assert!((r.value() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn detect_citation_no_overlap_scores_zero() {
        let bundle = s(&["src/a.rs"]);
        let reply = "I cleaned up doc/intro.md and shipped.";
        let r = detect_citation(reply, &bundle);
        assert_eq!(r.matched, 0);
        assert_eq!(r.value(), 0.0);
    }

    #[test]
    fn detect_citation_partial_overlap_scores_jaccard() {
        let bundle = s(&["a.rs", "b.rs"]);
        let reply = "Edited a.rs and also c.rs"; // hits: {a.rs}; reply: {a.rs, c.rs}; union: {a.rs, b.rs, c.rs}
        let r = detect_citation(reply, &bundle);
        assert_eq!(r.matched, 1);
        assert!((r.value() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn detect_citation_empty_bundle_returns_zero() {
        let r = detect_citation("any reply src/a.rs", &[]);
        assert_eq!(r.value(), 0.0);
        assert_eq!(r.bundle_size, 0);
    }

    #[test]
    fn detect_citation_respects_token_window() {
        let bundle = s(&["src/a.rs"]);
        // Push the citation past the first 4 tokens.
        let reply = "one two three four src/a.rs five";
        let narrow = detect_citation_with_window(reply, &bundle, 4);
        assert_eq!(narrow.matched, 0);
        let wide = detect_citation_with_window(reply, &bundle, 100);
        assert_eq!(wide.matched, 1);
    }

    #[test]
    fn first_tokens_clips_at_count() {
        assert_eq!(first_tokens("one two three four", 2), "one two");
        assert_eq!(first_tokens("", 10), "");
        assert_eq!(first_tokens("solo", 10), "solo");
        assert_eq!(first_tokens("one two three", 0), "");
    }

    #[test]
    fn it_pins_known_fixture_turn_score() {
        // Fixture: a Cortex-style turn that cites two bundle files
        // out of three expected. The score is fixed so a future
        // change in extractor heuristics surfaces as a delta.
        let bundle = s(&["src/lib.rs", "src/budget.rs", "docs/specs/12.md"]);
        let reply = "Tweaked src/lib.rs to expose the new module and updated docs/specs/12.md. The clipper in src/budget.rs stays untouched.";
        let r = detect_citation(reply, &bundle);
        // 3 of 3 in the reply window → matched = 3, union = 3
        assert_eq!(r.matched, 3);
        assert_eq!(r.bundle_size, 3);
        assert!((r.value() - 1.0).abs() < 1e-9);
    }
}

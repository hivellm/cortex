//! Query rewriter — phase6f, closes F-004.
//!
//! The user prompt arrives at `/v1/query` as free-form English.
//! Forwarding it verbatim to every lane lets question-framing words
//! (`why`, `how`, `should we`) crowd out the load-bearing technical
//! tokens. This module owns the pre-pass that strips the framing
//! (or asks Sonnet to do it) before the orchestrator fans out.
//!
//! Three strategies:
//!
//! - [`PassthroughRewriter`] — kill-switch. Reproduces today's
//!   behaviour for A/B'ing under the phase6e harness.
//! - [`NounPhraseRewriter`] — deterministic, no LLM. Strips question
//!   words + a curated stop-list and re-emits the surviving tokens.
//!   Same string goes to both lanes — its job is removing noise,
//!   not specialising per-lane.
//! - [`SonnetRewriter`] — opt-in. One Sonnet call per cache miss,
//!   produces distinct vector / keyword queries. Falls back to
//!   `NounPhraseRewriter` on timeout / upstream error so a flaky
//!   upstream never fails the user-facing request.
//!
//! The orchestrator threads the resulting [`RewrittenQuery`] into
//! the per-lane request builders and round-trips it to the audit
//! envelope (`query_rewrite_strategy`, `vector_query`, `keyword_query`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::types::Intent;

/// Output of a [`QueryRewriter`] — what the orchestrator threads
/// into each lane request, plus the original prompt for audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenQuery {
    /// Optimised for embedding (load-bearing terms first, framing
    /// stripped).
    pub vector_query: String,
    /// Optimised for Meili — BM25-friendly tokens. The deterministic
    /// rewriter emits the same string as `vector_query`; the Sonnet
    /// rewriter may differentiate.
    pub keyword_query: String,
    /// Round-tripped for audit so operators can see what was typed
    /// before any rewriting fired.
    pub original: String,
    /// Discriminator stamped on the audit envelope — one of
    /// `"passthrough"`, `"noun_phrase"`, `"sonnet"`.
    pub strategy: &'static str,
}

impl RewrittenQuery {
    /// Build a passthrough record — both lanes carry the original
    /// prompt verbatim. Strategy `"passthrough"`.
    pub fn passthrough(prompt: &str) -> Self {
        Self {
            vector_query: prompt.to_string(),
            keyword_query: prompt.to_string(),
            original: prompt.to_string(),
            strategy: "passthrough",
        }
    }
}

/// Failure modes raised by a [`QueryRewriter`].
#[derive(Debug, Error)]
pub enum RewriteError {
    /// Upstream rewriter (e.g. Sonnet) exceeded its budget.
    #[error("rewriter timed out")]
    Timeout,
    /// Upstream rewriter returned an error.
    #[error("rewriter upstream: {0}")]
    Upstream(String),
    /// Strategy is wired but disabled at runtime (kill-switch flipped
    /// after boot).
    #[error("rewriter disabled")]
    Disabled,
}

/// Pluggable rewriter trait.
#[async_trait]
pub trait QueryRewriter: Send + Sync {
    /// Rewrite `prompt` into per-lane queries. Implementations MUST
    /// always return *some* [`RewrittenQuery`] for valid input —
    /// callers route a `Result::Err` straight to the user-facing
    /// failure path and never see a degraded mode otherwise.
    async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError>;
}

// ============================================================================
// PassthroughRewriter
// ============================================================================

/// Kill-switch rewriter — copies the prompt to both lanes
/// unchanged. Default strategy when `CORTEX_QUERY_REWRITER` is
/// unset, used by integration tests, and the explicit choice when
/// operators want to A/B against today's behaviour.
#[derive(Debug, Default, Clone, Copy)]
pub struct PassthroughRewriter;

#[async_trait]
impl QueryRewriter for PassthroughRewriter {
    async fn rewrite(&self, prompt: &str, _intent: Intent) -> Result<RewrittenQuery, RewriteError> {
        Ok(RewrittenQuery::passthrough(prompt))
    }
}

// ============================================================================
// NounPhraseRewriter
// ============================================================================

/// Default rewriter — deterministic, no network. Strips leading
/// question words + a curated stop-list, keeps technical tokens
/// (camelCase / snake_case / kebab-case / dotted paths / file
/// extensions). Same string goes to both lanes.
#[derive(Debug, Default, Clone, Copy)]
pub struct NounPhraseRewriter;

const QUESTION_LEADERS: &[&str] = &[
    "why", "how", "what", "when", "who", "where", "is", "are", "does", "do", "should", "can",
    "could", "would",
];

const STOP_WORDS: &[&str] = &[
    // Generic English noise.
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "at", "for", "with", "from", "this",
    "that", "these", "those", "it", "its", "as", "be", "by", "we", "you", "i", "me", "my", "our",
    "us", "they", "them", "their", "if", "then", "than", "so", "but", "not", "no", "yes", "ok",
    "okay", "just", "really", "very", "much", "more", "less", "any", "some", "all", "each",
    "every", "such", "into", "out", "over", "under", "about", "around", "near", "between",
    "without", "within", "via", "while", "after", "before", "still", "even",
    // Operator filler that shows up a lot in audit prompts.
    "please", "thanks", "thx", "btw", "kind", "of", "sort",
    // Auxiliaries the leading-word strip misses when they appear
    // mid-sentence.
    "was", "were", "been", "being", "am", "have", "has", "had", "having", "let", "go", "goes",
    "going", "make", "makes", "making", "did", "doing", "done",
];

impl NounPhraseRewriter {
    /// Build a fresh rewriter (no state).
    pub fn new() -> Self {
        Self
    }

    /// Pure-function variant — exposed for unit tests + the Sonnet
    /// rewriter's fallback path so neither has to construct a
    /// throwaway `NounPhraseRewriter` to invoke the algorithm.
    pub fn rewrite_str(prompt: &str) -> String {
        // 1. Lowercase + tokenise on whitespace + a small punctuation
        //    set. We deliberately keep `_`, `-`, `.`, `/` as part of a
        //    token so paths and identifiers (`crates/foo/bar.rs`,
        //    `meili_loader`, `cortex-api`) survive intact.
        let lower = prompt.to_lowercase();
        let raw_tokens: Vec<&str> = lower
            .split(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        ',' | ';'
                            | ':'
                            | '?'
                            | '!'
                            | '"'
                            | '\''
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                    )
            })
            .filter(|t| !t.is_empty())
            .collect();

        // 2. Strip leading question word(s). Operators sometimes
        //    chain ("how why does X..."); peel until the first
        //    non-question token.
        let mut idx = 0usize;
        while idx < raw_tokens.len() && QUESTION_LEADERS.contains(&raw_tokens[idx]) {
            idx += 1;
        }
        let body = &raw_tokens[idx..];

        // 3. Drop stop-words + lone punctuation residues; keep
        //    technical-looking tokens.
        let mut kept: Vec<String> = Vec::with_capacity(body.len());
        for tok in body {
            if STOP_WORDS.contains(tok) {
                continue;
            }
            if !looks_like_content_token(tok) {
                continue;
            }
            // Preserve original casing for identifier-style tokens
            // by re-extracting the slice from the original prompt.
            // We keep lower-case because the keyword + vector lanes
            // are case-insensitive — if a future lane needs case,
            // the audit `original` field still carries the verbatim
            // input.
            kept.push((*tok).to_string());
        }

        if kept.is_empty() {
            // The user typed only framing words — fall back to the
            // original prompt rather than send an empty string. An
            // empty query crashes Meili's tokenizer and returns
            // zero hits from Vectorizer.
            return prompt.trim().to_string();
        }

        kept.join(" ")
    }
}

fn looks_like_content_token(tok: &str) -> bool {
    // Reject pure-numeric (years, counts), pure-punctuation, and
    // single-letter noise. Keep:
    //  - identifiers (start with alpha, may contain `_`/`-`/`.`/`/`)
    //  - file extensions (start with `.`)
    //  - tokens that already contain `_`/`-`/`.`/`/` even if the
    //    leading char is non-alpha (rare but covers `2x_buf`).
    if tok.is_empty() {
        return false;
    }
    let first = tok.chars().next().unwrap();
    let has_compound_marker =
        tok.contains('_') || tok.contains('-') || tok.contains('.') || tok.contains('/');
    if first.is_ascii_digit() && !has_compound_marker {
        return false;
    }
    // Drop single-character residues unless they're a path-like
    // marker.
    if tok.chars().count() == 1 && !has_compound_marker {
        return false;
    }
    true
}

#[async_trait]
impl QueryRewriter for NounPhraseRewriter {
    async fn rewrite(&self, prompt: &str, _intent: Intent) -> Result<RewrittenQuery, RewriteError> {
        let stripped = Self::rewrite_str(prompt);
        Ok(RewrittenQuery {
            vector_query: stripped.clone(),
            keyword_query: stripped,
            original: prompt.to_string(),
            strategy: "noun_phrase",
        })
    }
}

// ============================================================================
// SonnetRewriter
// ============================================================================

/// Configuration for the Sonnet-backed rewriter. The Cortex stack
/// always invokes Claude through the Claude Code CLI (same pattern
/// as `cortex-classifier` and `crate::analyzer::Analyzer::invoke_cli`)
/// — never via the Anthropic HTTP API directly — so deployments
/// only need the binary on `PATH`, not an API key.
#[derive(Debug, Clone)]
pub struct SonnetRewriterConfig {
    /// Path to the `claude` binary. Default `claude` (resolved
    /// against `PATH`); operators can override with
    /// `CLAUDE_CODE_BIN=/usr/local/bin/claude` for non-standard
    /// install locations.
    pub claude_bin: String,
    /// Model id passed to `claude -p ... --model <model>`.
    /// Defaults to `claude-sonnet-4-6` per AGENTS.md.
    pub model: String,
    /// Per-call wall-clock budget. Spec calls for `1.5s` for the
    /// rewriter; operators bump via `CORTEX_REWRITER_TIMEOUT_MS`
    /// when their CLI install has a cold-start penalty (the
    /// classifier-worker raised this same knob from 30s → 90s
    /// after observing 100% timeout rates on cold starts).
    pub timeout: Duration,
    /// Cache TTL for `(prompt + intent)` rewrites. 24 hours per
    /// spec — operator phrasings repeat across sessions.
    pub cache_ttl: Duration,
    /// Bound the cache so a misbehaving caller can't OOM us.
    pub cache_capacity: usize,
}

impl Default for SonnetRewriterConfig {
    fn default() -> Self {
        Self {
            claude_bin: std::env::var("CLAUDE_CODE_BIN")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude".to_string()),
            model: std::env::var("CORTEX_REWRITER_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
            timeout: parse_timeout_ms_env("CORTEX_REWRITER_TIMEOUT_MS", 1_500),
            cache_ttl: Duration::from_secs(24 * 60 * 60),
            cache_capacity: 4096,
        }
    }
}

fn parse_timeout_ms_env(key: &str, default_ms: u64) -> Duration {
    let ms = std::env::var(key)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n >= 100)
        .unwrap_or(default_ms);
    Duration::from_millis(ms)
}

#[derive(Clone)]
struct CacheEntry {
    inserted: Instant,
    rewritten: RewrittenQuery,
}

/// Sonnet-backed rewriter. One CLI call per cache miss; falls back
/// to [`NounPhraseRewriter`] on any failure so the user-facing
/// request is never gated on the CLI's availability.
pub struct SonnetRewriter {
    cfg: SonnetRewriterConfig,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl SonnetRewriter {
    /// Build a new rewriter with the given config.
    pub fn new(cfg: SonnetRewriterConfig) -> Self {
        Self {
            cfg,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Convenience constructor reading env defaults.
    pub fn from_env() -> Self {
        Self::new(SonnetRewriterConfig::default())
    }

    fn cache_key(prompt: &str, intent: Intent) -> String {
        let mut h = Sha256::new();
        h.update(prompt.as_bytes());
        h.update([0x1f]);
        h.update(intent.label().as_bytes());
        let digest = h.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest.iter() {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{b:02x}");
        }
        out
    }

    fn cache_get(&self, key: &str) -> Option<RewrittenQuery> {
        let g = self.cache.lock().ok()?;
        let entry = g.get(key)?;
        if entry.inserted.elapsed() <= self.cfg.cache_ttl {
            Some(entry.rewritten.clone())
        } else {
            None
        }
    }

    fn cache_put(&self, key: String, rewritten: RewrittenQuery) {
        if let Ok(mut g) = self.cache.lock() {
            // Trivial bound: drop the oldest entries when over
            // capacity. Production-grade LRU is overkill for a
            // 4k-entry cap that exists purely to bound memory.
            if g.len() >= self.cfg.cache_capacity {
                let cutoff = self
                    .cfg
                    .cache_capacity
                    .saturating_sub(self.cfg.cache_capacity / 4);
                let mut entries: Vec<(String, Instant)> =
                    g.iter().map(|(k, v)| (k.clone(), v.inserted)).collect();
                entries.sort_by_key(|(_, t)| *t);
                for (k, _) in entries.iter().take(g.len().saturating_sub(cutoff)) {
                    g.remove(k);
                }
            }
            g.insert(
                key,
                CacheEntry {
                    inserted: Instant::now(),
                    rewritten,
                },
            );
        }
    }

    /// Build the prompt the CLI sees on stdin. Combines the spec-09
    /// system instructions with the operator's prompt + the routed
    /// intent. The Claude Code CLI does not have a separate
    /// `system` channel for the `-p -` invocation, so we inline the
    /// instructions ahead of the user content.
    fn render_cli_prompt(prompt: &str, intent: Intent) -> String {
        format!(
            "{system}\n\n---\nintent: {intent_label}\nprompt: {prompt}\n",
            system = SONNET_SYSTEM_PROMPT,
            intent_label = intent.label(),
        )
    }

    /// Spawn `claude -p - --model <model> --output-format json`
    /// and feed the rendered prompt over stdin. Mirrors
    /// [`crate::analyzer::Analyzer::invoke_cli`] (same envelope
    /// shape: `{"result": "<inner-json>", ...}`). Public so
    /// integration tests can swap the binary path via
    /// `SonnetRewriterConfig::claude_bin` without going through
    /// process spawning at all.
    pub async fn invoke_cli(
        &self,
        prompt: &str,
        intent: Intent,
    ) -> Result<RewrittenQuery, RewriteError> {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let rendered = Self::render_cli_prompt(prompt, intent);
        let mut cmd = Command::new(&self.cfg.claude_bin);
        cmd.args([
            "-p",
            "-",
            "--model",
            &self.cfg.model,
            "--output-format",
            "json",
        ]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| RewriteError::Upstream(format!("spawn {}: {e}", self.cfg.claude_bin)))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(rendered.as_bytes())
                .await
                .map_err(|e| RewriteError::Upstream(format!("write stdin: {e}")))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| RewriteError::Upstream(format!("shutdown stdin: {e}")))?;
        }
        let output = tokio::time::timeout(self.cfg.timeout, child.wait_with_output())
            .await
            .map_err(|_| RewriteError::Timeout)?
            .map_err(|e| RewriteError::Upstream(format!("wait: {e}")))?;

        if !output.status.success() {
            return Err(RewriteError::Upstream(format!(
                "claude exit {}: {}",
                output.status.code().unwrap_or_default(),
                clip(&String::from_utf8_lossy(&output.stderr), 240)
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Self::parse_cli_envelope(&stdout, prompt)
    }

    /// Parse the Claude Code CLI's `--output-format json` envelope
    /// (`{"result": "<text>", "session_id": "...", ...}`) and the
    /// inner JSON the model returned. Split out so unit tests can
    /// drive the parser without spawning a process.
    fn parse_cli_envelope(stdout: &str, original: &str) -> Result<RewrittenQuery, RewriteError> {
        let outer: serde_json::Value = serde_json::from_str(stdout).map_err(|e| {
            RewriteError::Upstream(format!("cli outer json: {e} — raw: {}", clip(stdout, 240)))
        })?;
        let inner_raw = outer
            .get("result")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RewriteError::Upstream(format!(
                    "cli output missing `result`: {}",
                    clip(stdout, 240)
                ))
            })?;
        let cleaned = strip_code_fence(inner_raw);
        let inner: serde_json::Value = serde_json::from_str(&cleaned).map_err(|e| {
            RewriteError::Upstream(format!(
                "cli inner json: {e} — body: {}",
                clip(&cleaned, 240)
            ))
        })?;
        let vec_q = inner
            .get("vector_query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RewriteError::Upstream("vector_query missing/empty".into()))?
            .to_string();
        let kw_q = inner
            .get("keyword_query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RewriteError::Upstream("keyword_query missing/empty".into()))?
            .to_string();
        Ok(RewrittenQuery {
            vector_query: vec_q,
            keyword_query: kw_q,
            original: original.to_string(),
            strategy: "sonnet",
        })
    }
}

const SONNET_SYSTEM_PROMPT: &str = r#"You are a query-rewriter for a hybrid retrieval system (vector + keyword + graph).

Given the operator's free-form prompt and the routed intent, return a JSON object:

  {
    "vector_query": "string optimised for cosine-similarity embedding search",
    "keyword_query": "string optimised for BM25 keyword search",
    "rationale": "one sentence explaining the rewrite"
  }

Rules:
- Drop framing words ("why is", "how do I", "should we"). Keep load-bearing technical
  tokens (identifiers, paths, file extensions, repo names).
- The vector_query may be a short natural-language phrase that captures intent.
- The keyword_query should be space-separated tokens (no English connectives).
- Both fields MUST be non-empty.
- Return ONLY the JSON object — no markdown, no commentary."#;

#[async_trait]
impl QueryRewriter for SonnetRewriter {
    async fn rewrite(&self, prompt: &str, intent: Intent) -> Result<RewrittenQuery, RewriteError> {
        // Cache hit — return immediately.
        let key = Self::cache_key(prompt, intent);
        if let Some(hit) = self.cache_get(&key) {
            return Ok(hit);
        }

        // Cache miss — invoke the Claude Code CLI. Any failure
        // (timeout, missing binary, non-zero exit, malformed JSON)
        // falls back to the deterministic noun-phrase strategy so
        // the user request never blocks on the CLI's availability.
        match self.invoke_cli(prompt, intent).await {
            Ok(rewritten) => {
                self.cache_put(key, rewritten.clone());
                Ok(rewritten)
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    intent = intent.label(),
                    "sonnet rewriter failed; falling back to noun_phrase"
                );
                let stripped = NounPhraseRewriter::rewrite_str(prompt);
                Ok(RewrittenQuery {
                    vector_query: stripped.clone(),
                    keyword_query: stripped,
                    original: prompt.to_string(),
                    // Phase6f §3.4 — surfaced to the audit envelope
                    // so an operator can tell "Sonnet ran" apart
                    // from "Sonnet timed out and we fell back".
                    strategy: "sonnet_fallback_noun_phrase",
                })
            }
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(max.min(s.len()));
    for c in s.chars().take(max) {
        out.push(c);
    }
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let stripped = stripped.trim_end_matches("```");
    stripped.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- RewrittenQuery / passthrough ----

    #[test]
    fn passthrough_copies_prompt_into_both_lanes() {
        let r = RewrittenQuery::passthrough("Hello world");
        assert_eq!(r.vector_query, "Hello world");
        assert_eq!(r.keyword_query, "Hello world");
        assert_eq!(r.original, "Hello world");
        assert_eq!(r.strategy, "passthrough");
    }

    #[tokio::test]
    async fn passthrough_rewriter_returns_passthrough_record() {
        let r = PassthroughRewriter
            .rewrite("anything", Intent::FreeSearch)
            .await
            .unwrap();
        assert_eq!(r.strategy, "passthrough");
        assert_eq!(r.vector_query, "anything");
    }

    // ---- NounPhraseRewriter ----

    #[test]
    fn noun_phrase_strips_leading_question_words() {
        let out = NounPhraseRewriter::rewrite_str(
            "why is the meili fan-out broken should we just rewrite the worker",
        );
        // Question leaders ("why", "is") + stop-words gone; tech
        // tokens (`meili`, `fan-out`, `worker`) preserved.
        assert!(out.contains("meili"));
        assert!(out.contains("fan-out"));
        assert!(out.contains("worker"));
        assert!(!out.contains("why "));
        assert!(!out.starts_with("is "));
    }

    #[test]
    fn noun_phrase_preserves_technical_tokens() {
        let out = NounPhraseRewriter::rewrite_str(
            "where is PreThinkingTool defined in crates/cortex-api/src/strategies.rs",
        );
        assert!(out.contains("prethinkingtool"));
        assert!(out.contains("crates/cortex-api/src/strategies.rs"));
        // Only the leading "where is" should be stripped, not
        // "in" mid-sentence; but `in` is a stop word so it's gone.
        assert!(!out.split_whitespace().any(|t| t == "in"));
    }

    #[test]
    fn noun_phrase_keeps_snake_case_and_kebab_case() {
        let out = NounPhraseRewriter::rewrite_str("how does meili_loader interact with cortex-api");
        assert!(out.contains("meili_loader"));
        assert!(out.contains("cortex-api"));
    }

    #[test]
    fn noun_phrase_drops_pure_punctuation_and_single_letters() {
        let out = NounPhraseRewriter::rewrite_str("? a x meili !");
        // `a` is in the stop list, single-letter `x` is dropped.
        assert!(out.contains("meili"));
        assert!(!out.split_whitespace().any(|t| t == "a"));
        assert!(!out.split_whitespace().any(|t| t == "x"));
    }

    #[test]
    fn noun_phrase_falls_back_to_original_when_only_framing() {
        // Pure framing words → none of them survive the strip.
        // Falling back to the trimmed original keeps Meili happy.
        let out = NounPhraseRewriter::rewrite_str("why is it");
        assert_eq!(out, "why is it");
    }

    #[test]
    fn noun_phrase_handles_empty_input_without_panic() {
        let out = NounPhraseRewriter::rewrite_str("");
        assert_eq!(out, "");
        let out = NounPhraseRewriter::rewrite_str("   ");
        assert_eq!(out, "");
    }

    #[test]
    fn noun_phrase_chained_question_leaders_are_all_stripped() {
        let out = NounPhraseRewriter::rewrite_str("how why does meili route events");
        // All three of "how", "why", "does" are leaders; the strip
        // continues until the first non-leader.
        assert!(out.starts_with("meili"));
    }

    #[tokio::test]
    async fn noun_phrase_rewriter_round_trips_original() {
        let r = NounPhraseRewriter
            .rewrite("why is meili broken", Intent::Explain)
            .await
            .unwrap();
        assert_eq!(r.original, "why is meili broken");
        assert_eq!(r.vector_query, r.keyword_query);
        assert_eq!(r.strategy, "noun_phrase");
        assert!(r.vector_query.contains("meili"));
    }

    // ---- looks_like_content_token ----

    #[test]
    fn looks_like_content_token_admits_identifier_and_path() {
        assert!(looks_like_content_token("meili"));
        assert!(looks_like_content_token("cortex-api"));
        assert!(looks_like_content_token("crates/foo/bar.rs"));
        assert!(looks_like_content_token(".rs"));
    }

    #[test]
    fn looks_like_content_token_rejects_pure_numeric_and_short_noise() {
        assert!(!looks_like_content_token("2026"));
        assert!(!looks_like_content_token("1"));
        assert!(!looks_like_content_token(""));
        assert!(!looks_like_content_token("a"));
        // Compound number is fine — `2x_buf` shows up in profile dumps.
        assert!(looks_like_content_token("2x_buf"));
    }

    // ---- Sonnet helpers ----

    #[test]
    fn cache_key_changes_with_intent_and_prompt() {
        let a = SonnetRewriter::cache_key("hello", Intent::FreeSearch);
        let b = SonnetRewriter::cache_key("hello", Intent::Explain);
        let c = SonnetRewriter::cache_key("world", Intent::FreeSearch);
        assert_ne!(a, b);
        assert_ne!(a, c);
        // Stable across calls.
        assert_eq!(a, SonnetRewriter::cache_key("hello", Intent::FreeSearch));
    }

    #[test]
    fn strip_code_fence_handles_json_wrapper() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("```\n{}\n```"), "{}");
        assert_eq!(strip_code_fence("plain"), "plain");
    }

    #[test]
    fn clip_truncates_with_ellipsis() {
        let s = "abcdefghij";
        assert_eq!(clip(s, 100), "abcdefghij");
        assert_eq!(clip(s, 4), "abcd…");
    }

    #[tokio::test]
    async fn sonnet_falls_back_when_cli_binary_missing() {
        // Point at a binary that definitely doesn't exist on PATH —
        // spawn fails, invoke_cli returns Upstream, the public
        // rewrite() catches it and falls back to noun-phrase.
        // Strategy stamp must reflect the fallback so the audit
        // envelope distinguishes "Sonnet ran" from "Sonnet bailed".
        let cfg = SonnetRewriterConfig {
            claude_bin: "definitely-not-a-real-binary-cortex-test".into(),
            timeout: Duration::from_secs(2),
            ..SonnetRewriterConfig::default()
        };
        let r = SonnetRewriter::new(cfg)
            .rewrite("why is meili broken", Intent::Explain)
            .await
            .unwrap();
        assert_eq!(r.strategy, "sonnet_fallback_noun_phrase");
        assert!(r.vector_query.contains("meili"));
    }

    #[test]
    fn parse_cli_envelope_extracts_both_fields() {
        // Simulates the canonical Claude Code CLI envelope shape:
        // {"result":"<inner-json>","session_id":"...","model":"..."}
        // where <inner-json> is the model's actual response.
        let stdout = serde_json::json!({
            "result": "{\"vector_query\":\"meili fan-out worker offset\",\"keyword_query\":\"meili fan-out worker offset\",\"rationale\":\"strip framing\"}",
            "session_id": "abc",
            "model": "claude-sonnet-4-6"
        })
        .to_string();
        let r = SonnetRewriter::parse_cli_envelope(&stdout, "original prompt").unwrap();
        assert_eq!(r.strategy, "sonnet");
        assert_eq!(r.vector_query, "meili fan-out worker offset");
        assert_eq!(r.keyword_query, "meili fan-out worker offset");
        assert_eq!(r.original, "original prompt");
    }

    #[test]
    fn parse_cli_envelope_strips_markdown_fences() {
        // Claude occasionally wraps its JSON in ```json fences
        // despite explicit instructions; the parser must peel them.
        let stdout = serde_json::json!({
            "result": "```json\n{\"vector_query\":\"v\",\"keyword_query\":\"k\"}\n```"
        })
        .to_string();
        let r = SonnetRewriter::parse_cli_envelope(&stdout, "p").unwrap();
        assert_eq!(r.vector_query, "v");
        assert_eq!(r.keyword_query, "k");
    }

    #[test]
    fn parse_cli_envelope_errors_on_missing_result_field() {
        let stdout = serde_json::json!({ "session_id": "x" }).to_string();
        let err = SonnetRewriter::parse_cli_envelope(&stdout, "p").unwrap_err();
        assert!(matches!(err, RewriteError::Upstream(_)));
        assert!(err.to_string().contains("missing `result`"));
    }

    #[test]
    fn parse_cli_envelope_errors_on_invalid_outer_json() {
        let err = SonnetRewriter::parse_cli_envelope("not-json", "p").unwrap_err();
        assert!(matches!(err, RewriteError::Upstream(_)));
        assert!(err.to_string().contains("cli outer json"));
    }

    #[test]
    fn parse_cli_envelope_errors_on_invalid_inner_json() {
        let stdout = serde_json::json!({ "result": "not-json" }).to_string();
        let err = SonnetRewriter::parse_cli_envelope(&stdout, "p").unwrap_err();
        assert!(matches!(err, RewriteError::Upstream(_)));
        assert!(err.to_string().contains("cli inner json"));
    }

    #[test]
    fn parse_cli_envelope_errors_when_vector_query_missing() {
        let stdout = serde_json::json!({
            "result": "{\"keyword_query\":\"k\"}"
        })
        .to_string();
        let err = SonnetRewriter::parse_cli_envelope(&stdout, "p").unwrap_err();
        assert!(err.to_string().contains("vector_query"));
    }

    #[test]
    fn parse_cli_envelope_errors_when_keyword_query_empty() {
        let stdout = serde_json::json!({
            "result": "{\"vector_query\":\"v\",\"keyword_query\":\"\"}"
        })
        .to_string();
        let err = SonnetRewriter::parse_cli_envelope(&stdout, "p").unwrap_err();
        assert!(err.to_string().contains("keyword_query"));
    }

    #[test]
    fn render_cli_prompt_inlines_intent_and_system_block() {
        let body = SonnetRewriter::render_cli_prompt("why is meili broken", Intent::Explain);
        // The rendered prompt must include the intent label so the
        // model can specialise its rewrite, AND the system block
        // (because the CLI's `-p -` invocation has no separate
        // system channel — we inline the instructions ahead of the
        // user content).
        assert!(body.contains("intent: explain"));
        assert!(body.contains("prompt: why is meili broken"));
        assert!(body.contains("vector_query"));
        assert!(body.contains("keyword_query"));
    }

    #[test]
    fn config_defaults_pull_from_env_with_fallbacks() {
        // Defaults must NOT depend on ANTHROPIC_API_KEY being set —
        // the rewriter is CLI-only and operators should not need
        // an API key for the Sonnet path. The default binary name
        // is `claude` (resolved against PATH).
        let cfg = SonnetRewriterConfig::default();
        assert!(!cfg.claude_bin.is_empty());
        assert_eq!(cfg.cache_capacity, 4096);
        assert_eq!(cfg.cache_ttl, Duration::from_secs(24 * 60 * 60));
        // Timeout falls back to 1.5s when the env var is unset; it
        // honours `CORTEX_REWRITER_TIMEOUT_MS` when ≥ 100ms.
        assert!(cfg.timeout >= Duration::from_millis(100));
    }
}

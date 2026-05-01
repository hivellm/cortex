//! On-demand session analysis backed by Sonnet via the local
//! `claude` CLI. The classifier worker handles per-event labeling
//! (topics + summary stamped on each envelope); this module sits
//! one layer above and produces a *cross-event* analysis: take
//! every turn + tool_call from one session, hand the whole thing
//! to Sonnet, get back a structured summary + key actions +
//! cross-references.
//!
//! The 2026-04-28 user signal: "use sonnet to start analyzing our
//! data and generating summaries and meaningful interconnections."
//! Per-event Haiku-grade classification was producing tags with no
//! lift; what was missing was the wider lens.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::lanes::{LaneHit, MemoryKeywordLane};

/// Analyzer-side config. Reads `CORTEX_ANALYZER_*` env vars at
/// construction with sensible defaults so a cold-stack dev doesn't
/// need to set anything to get a basic Sonnet summary.
#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    /// Path to the `claude` CLI. Defaults to `"claude"` on PATH.
    pub claude_bin: String,
    /// Model identifier passed via `--model` (or `model` in API body).
    pub model: String,
    /// Per-call timeout. Sonnet at 4-8k tokens of conversation
    /// usually answers in under 30s; we cap at 120s to be safe.
    pub timeout: Duration,
    /// Cap on the number of LaneHits we feed into one prompt. The
    /// CLI has a hard prompt-size ceiling and Sonnet's context cost
    /// scales linearly; clipping at 200 events keeps the typical
    /// session under ~50k tokens.
    pub max_events: usize,
    /// Anthropic API key — when set, the analyzer skips the CLI and
    /// posts directly to `https://api.anthropic.com/v1/messages`.
    /// Lets the daemon work on hosts that don't ship the `claude`
    /// binary (most servers, CI runners, the user's case where the
    /// CLI lives inside Cursor/VS Code rather than on PATH).
    pub api_key: Option<String>,
    /// Anthropic API base URL — overridable so tests can point at a
    /// wiremock fixture. Defaults to the public endpoint.
    pub api_base: String,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            claude_bin: std::env::var("CORTEX_ANALYZER_BIN")
                .or_else(|_| std::env::var("CLAUDE_CODE_BIN"))
                .unwrap_or_else(|_| "claude".to_string()),
            model: std::env::var("CORTEX_ANALYZER_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-6".to_string()),
            timeout: Duration::from_secs(120),
            max_events: 200,
            api_key: std::env::var("CORTEX_ANALYZER_API_KEY")
                .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
                .ok()
                .filter(|s| !s.is_empty()),
            api_base: std::env::var("CORTEX_ANALYZER_API_BASE")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_string()),
        }
    }
}

/// Structured summary returned to the dashboard. The CLI is
/// instructed to produce JSON in this exact shape; defensive
/// fallbacks coerce loose model output into the strict types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// 2-3 paragraph summary of what the session was about.
    pub summary: String,
    /// Bullet list of concrete actions / decisions / changes made
    /// during the session.
    #[serde(default)]
    pub key_actions: Vec<String>,
    /// Cross-references the model identified — file paths,
    /// decision ids (e.g. `DEC-0042`), session ids, repo names.
    /// These are the "interconexões" the user asked for; the GUI
    /// can render them as clickable links.
    #[serde(default)]
    pub references: Vec<String>,
    /// Free-text topics — same vocabulary as the per-event
    /// classifier but at the session granularity.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Repos this session touched (echoed from the input so the GUI
    /// can render the per-project tag without a second fetch).
    #[serde(default)]
    pub repos: Vec<String>,
}

/// Cache key — `(session_id, last_event_ts_ms)`. When the session
/// gains a new turn / tool_call the ts moves forward and the
/// cached summary becomes stale; the next request rebuilds it. A
/// pure session-id key would have served stale answers forever.
type CacheKey = (String, i64);

/// In-memory analyzer with a single cache. The cache is the only
/// reason this isn't a free function — a per-request invocation
/// would burn a Sonnet call on every dashboard refresh.
pub struct Analyzer {
    cfg: AnalyzerConfig,
    cache: Mutex<HashMap<CacheKey, SessionSummary>>,
}

impl Analyzer {
    /// Build a new analyzer with the given config.
    pub fn new(cfg: AnalyzerConfig) -> Self {
        Self {
            cfg,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Convenience — `Analyzer::new(AnalyzerConfig::default())`.
    pub fn from_env() -> Self {
        Self::new(AnalyzerConfig::default())
    }

    /// Generate (or return cached) summary for one session.
    ///
    /// Pulls every turn / tool_call envelope from the keyword lane
    /// for the requested session, builds a Sonnet prompt, and
    /// invokes the local `claude` CLI. Returns the structured
    /// summary on success, or a `String` describing why we
    /// couldn't (CLI not on PATH, timeout, parse error, etc.).
    /// The dashboard handler maps that to a 503 + `debug.errors`
    /// payload so the GUI shows a graceful "summary unavailable"
    /// message rather than a hard failure.
    pub async fn summarize_session(
        &self,
        lane: &MemoryKeywordLane,
        session_id: &str,
    ) -> Result<SessionSummary, String> {
        let hits = collect_session_hits(lane, session_id);
        if hits.is_empty() {
            return Err(format!("no envelopes captured for session {session_id}"));
        }
        let last_ts = hits.iter().map(|h| h.ts).max().unwrap_or(0);

        // Cache hit short-circuits the CLI call.
        if let Ok(cache) = self.cache.lock() {
            if let Some(cached) = cache.get(&(session_id.to_string(), last_ts)) {
                return Ok(cached.clone());
            }
        }

        let prompt = build_prompt(session_id, &hits, self.cfg.max_events);
        // Prefer the direct API when a key is set — cleaner shape,
        // no CLI dependency, structured error reporting. Fall back
        // to the CLI when the user explicitly opted in via env but
        // didn't supply a key (rare today, but the embedder-worker
        // and classifier-worker both run that way).
        let summary = if self.cfg.api_key.is_some() {
            self.invoke_api(&prompt).await?
        } else {
            self.invoke_cli(&prompt).await?
        };

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert((session_id.to_string(), last_ts), summary.clone());
        }
        Ok(summary)
    }

    /// Direct Anthropic Messages API call. Used when
    /// `ANTHROPIC_API_KEY` is set; bypasses the local CLI entirely
    /// so the daemon works on hosts where the binary isn't
    /// installed.
    async fn invoke_api(&self, prompt: &str) -> Result<SessionSummary, String> {
        let api_key = self
            .cfg
            .api_key
            .as_ref()
            .ok_or_else(|| "api_key unset — won't reach here".to_string())?;
        let url = format!("{}/v1/messages", self.cfg.api_base.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.cfg.model,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": prompt }],
        });
        let client = reqwest::Client::builder()
            .timeout(self.cfg.timeout)
            .build()
            .map_err(|e| format!("reqwest builder: {e}"))?;
        let resp = client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("post {url}: {e}"))?;
        let status = resp.status();
        let raw = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("anthropic api {status}: {}", clip(&raw, 240)));
        }
        // The Messages API response shape:
        //   { "content": [{"type":"text","text":"..."}, ...], ... }
        // We concatenate every `text` content block in case the
        // model emits its JSON across multiple blocks (rare with
        // the current models but harmless to handle).
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| format!("api outer json: {e} — raw: {}", clip(&raw, 240)))?;
        let mut buf = String::new();
        if let Some(arr) = parsed.get("content").and_then(|v| v.as_array()) {
            for block in arr {
                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        buf.push_str(t);
                    }
                }
            }
        }
        if buf.is_empty() {
            return Err(format!(
                "api response missing text content: {}",
                clip(&raw, 240)
            ));
        }
        let cleaned = strip_code_fence(&buf);
        let summary: SessionSummary = serde_json::from_str(&cleaned)
            .map_err(|e| format!("api inner json: {e} — body: {}", clip(&cleaned, 240)))?;
        Ok(summary)
    }

    /// Spawn `claude -p - --model <model> --output-format json`
    /// and feed the prompt over stdin. The CLI's JSON output wraps
    /// the model's response in `{ "result": "<text>", ... }`; we
    /// parse the inner text as JSON conforming to `SessionSummary`.
    async fn invoke_cli(&self, prompt: &str) -> Result<SessionSummary, String> {
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
            .map_err(|e| format!("spawn {}: {e}", self.cfg.claude_bin))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| format!("write stdin: {e}"))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| format!("shutdown stdin: {e}"))?;
        }
        let output = tokio::time::timeout(self.cfg.timeout, child.wait_with_output())
            .await
            .map_err(|_| "claude cli: timeout".to_string())?
            .map_err(|e| format!("wait_with_output: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "claude exit {}: {}",
                output.status.code().unwrap_or_default(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Claude Code CLI's JSON envelope: `{ "result": "<inner>",
        // "session_id": "...", "model": "...", ... }`. The model's
        // actual response is the `result` string; we then parse
        // *that* as JSON.
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let outer: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("cli outer json: {e} — raw: {}", clip(&stdout, 240)))?;
        let inner = outer
            .get("result")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("cli output missing `result`: {}", clip(&stdout, 240)))?;
        // Strip any markdown code fences the model occasionally
        // wraps around JSON despite explicit instructions.
        let cleaned = strip_code_fence(inner);
        let summary: SessionSummary = serde_json::from_str(&cleaned)
            .map_err(|e| format!("cli inner json: {e} — body: {}", clip(&cleaned, 240)))?;
        Ok(summary)
    }
}

/// Pull every turn / tool_call / agent_call hit for one session
/// from the keyword lane, ordered oldest → newest. The lane has
/// hits seeded by both archive_loader and meili_loader; both
/// stamp `extras.session_id` so this filter catches all of them.
fn collect_session_hits(lane: &MemoryKeywordLane, session_id: &str) -> Vec<LaneHit> {
    let g = match lane.hits.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<LaneHit> = Vec::new();
    for hits in g.values() {
        for h in hits {
            let sid = h
                .extras
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if sid == session_id {
                out.push(h.clone());
            }
        }
    }
    out.sort_by(|a, b| a.ts.cmp(&b.ts));
    out
}

/// Render the prompt fed to Sonnet. Front-loads the structured
/// instructions, then dumps the session's events as a numbered
/// list. The model is told to emit a single JSON object — no
/// preamble, no fences — though we strip both defensively.
fn build_prompt(session_id: &str, hits: &[LaneHit], max_events: usize) -> String {
    let mut out = String::new();
    out.push_str(
        "You are analyzing one session of captured Claude Code activity. Read the events \
         (oldest → newest) and produce a structured summary. Identify the goal of the session, \
         the concrete actions taken, the decisions / changes made, and any references to \
         other sessions, decisions (e.g. DEC-0042), files, repos, or external systems.\n\n\
         Respond with a SINGLE JSON object — no preamble, no markdown, no code fences — \
         conforming exactly to this schema:\n\n\
         {\n  \"summary\": \"2-3 paragraphs in plain prose\",\n  \
         \"key_actions\": [\"short action 1\", \"short action 2\", ...],\n  \
         \"references\": [\"DEC-0042\", \"path/to/file.rs\", \"session-id-or-other\", ...],\n  \
         \"topics\": [\"topic1\", \"topic2\", ...],\n  \
         \"repos\": [\"Cortex\", \"Nexus\", ...]\n}\n\n",
    );
    out.push_str(&format!("session_id: {session_id}\n"));

    let mut repos: std::collections::BTreeSet<String> = Default::default();
    for h in hits {
        if let Some(r) = h.repo.as_deref() {
            repos.insert(r.to_string());
        }
    }
    out.push_str(&format!(
        "repos_touched: {}\n\n",
        repos.iter().cloned().collect::<Vec<_>>().join(", ")
    ));
    out.push_str("--- events ---\n");

    let len = hits.len().min(max_events);
    for (i, h) in hits.iter().take(len).enumerate() {
        let kind = h.symbol.as_deref().unwrap_or("event");
        let path = h.path.as_deref().unwrap_or("");
        let text = clip(&h.text, 1200);
        out.push_str(&format!("[{i:03}] kind={kind} path={path}\n"));
        out.push_str(&text);
        out.push_str("\n---\n");
    }
    if hits.len() > max_events {
        out.push_str(&format!(
            "(truncated: {} additional events not shown)\n",
            hits.len() - max_events
        ));
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Trim ```json … ``` fences when the model wraps its output.
fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let stripped = stripped.strip_suffix("```").unwrap_or(stripped);
    stripped.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn hit(ts: i64, sid: &str, repo: Option<&str>, text: &str) -> LaneHit {
        let mut extras = std::collections::BTreeMap::new();
        extras.insert("session_id".to_string(), Value::String(sid.to_string()));
        LaneHit {
            doc_id: format!("test|{ts}"),
            text: text.to_string(),
            repo: repo.map(String::from),
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
        }
    }

    #[test]
    fn collect_session_hits_filters_by_session_id_and_orders_oldest_first() {
        let lane = MemoryKeywordLane::new();
        lane.seed(
            "cortex-cortex-code",
            vec![
                hit(200, "alpha", Some("Cortex"), "second"),
                hit(100, "alpha", Some("Cortex"), "first"),
                hit(300, "beta", Some("Cortex"), "other-session"),
            ],
        );
        let got = collect_session_hits(&lane, "alpha");
        let texts: Vec<_> = got.iter().map(|h| h.text.clone()).collect();
        assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn build_prompt_truncates_at_max_events() {
        let hits: Vec<LaneHit> = (0..210)
            .map(|i| hit(i as i64, "alpha", Some("Cortex"), &format!("evt-{i}")))
            .collect();
        let prompt = build_prompt("alpha", &hits, 200);
        assert!(prompt.contains("evt-199"), "200th event must be present");
        assert!(
            !prompt.contains("evt-200"),
            "events past max_events must be dropped",
        );
        assert!(
            prompt.contains("(truncated: 10 additional events not shown)"),
            "tail must announce the truncation"
        );
    }

    #[test]
    fn strip_code_fence_handles_json_fences() {
        assert_eq!(
            strip_code_fence("```json\n{\"summary\":\"x\"}\n```"),
            "{\"summary\":\"x\"}"
        );
        assert_eq!(
            strip_code_fence("```\n{\"summary\":\"x\"}\n```"),
            "{\"summary\":\"x\"}"
        );
        assert_eq!(
            strip_code_fence("{\"summary\":\"x\"}"),
            "{\"summary\":\"x\"}"
        );
    }

    #[test]
    fn session_summary_round_trips() {
        let s = SessionSummary {
            summary: "ran some tests".into(),
            key_actions: vec!["fixed a bug".into()],
            references: vec!["DEC-0042".into(), "src/lib.rs".into()],
            topics: vec!["testing".into()],
            repos: vec!["Cortex".into()],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.summary, s.summary);
        assert_eq!(back.references, s.references);
    }
}

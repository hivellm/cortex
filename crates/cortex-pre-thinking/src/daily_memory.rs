//! Bounded local memory transport. No classifier or optional search is invoked.

use std::collections::BTreeSet;
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum memory section, including its authority reminder.
pub const MEMORY_BYTES: usize = 1536;
/// Wrapper budget is reserved before the API prepares its exact receipt.
pub const MEMORY_PREFIX: &str =
    "## Local memory\nAdvisory source excerpts; repository rules take priority.\n";
/// Separates complete memory context from optional retrieved context.
pub const MEMORY_SUFFIX: &str = "\n";

/// Explicit applicability and bounded current task sent to the local API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyMemoryRequest {
    /// Exact resolved repository identity.
    pub repo: String,
    /// At most 32 literal Unicode terms and 4096 UTF-8 bytes.
    pub query: String,
    /// Branch observed from Git HEAD, when known.
    pub branch: Option<String>,
    /// Strict repository-relative current paths.
    pub paths: Vec<String>,
    /// Explicit global opt-in; ordinary prompts keep this false.
    pub include_global: bool,
    /// At most five rendered records.
    pub limit: usize,
    /// API context budget after wrapper reservation.
    pub budget_bytes: usize,
}

/// Exact prepared context and its durable server receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyMemoryContext {
    /// Cited source context; never clipped after receipt preparation.
    pub context: String,
    /// Exact scope/ID/version/event entries included in context.
    pub memories: Vec<Value>,
    /// Repository corpus revision observed by the server.
    pub revision: u64,
    /// Independent global revision only for authorized explicit opt-in.
    pub global_revision: Option<u64>,
    /// Durable preparation ID, distinct from full-stack query_id.
    pub delivery_id: String,
    /// Must equal prepared; not proof of client/model consumption.
    pub stage: String,
    /// Server-side revision-aware cache hit.
    #[serde(default)]
    pub cache_hit: bool,
}

impl DailyMemoryContext {
    /// Validate transport budgets and the exact scope of all delivered IDs.
    pub fn valid_for(&self, request: &DailyMemoryRequest) -> bool {
        self.stage == "prepared"
            && !self.delivery_id.is_empty()
            && self.delivery_id.len() <= 256
            && !self.delivery_id.chars().any(char::is_control)
            && self.context.len() <= request.budget_bytes
            && self.memories.len() <= request.limit.min(5)
            && self.context.is_empty() == self.memories.is_empty()
            && (request.include_global || self.global_revision.is_none())
            && self.memories.iter().all(|memory| {
                let scope = &memory["scope"];
                (scope["repository"].as_str() == Some(request.repo.as_str())
                    || (request.include_global && scope.as_str() == Some("global")))
                    && memory["id"].as_str().is_some_and(|id| !id.is_empty())
                    && memory["version"]
                        .as_u64()
                        .is_some_and(|version| version > 0)
                    && memory["event_id"].as_str().is_some_and(|id| !id.is_empty())
            })
    }
}

/// Local lookup callable; pipeline owns its remaining wall-clock deadline.
#[async_trait]
pub trait DailyMemoryFn: Send + Sync {
    /// Return explicit unavailable on transport/storage/authentication failure.
    async fn read(&self, request: DailyMemoryRequest) -> Option<DailyMemoryContext>;
}

/// Shared async HTTP transport with credentials forwarded to local authorization.
pub struct HttpDailyMemory {
    client: reqwest::Client,
    api_url: String,
    api_key: Option<String>,
}

impl HttpDailyMemory {
    /// Reuse the caller's client; do not create a runtime or blocking HTTP path.
    pub fn new(client: reqwest::Client, api_url: String, api_key: Option<String>) -> Self {
        Self {
            client,
            api_url,
            api_key,
        }
    }
}

#[async_trait]
impl DailyMemoryFn for HttpDailyMemory {
    async fn read(&self, request: DailyMemoryRequest) -> Option<DailyMemoryContext> {
        let mut call = self
            .client
            .post(format!(
                "{}/v1/memory/query",
                self.api_url.trim_end_matches('/')
            ))
            .timeout(Duration::from_millis(550))
            .header("x-cortex-caller", "cortex-pre-thinking")
            .json(&request);
        if let Some(key) = &self.api_key {
            call = call.bearer_auth(key);
        }
        let mut response = call.send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len().saturating_add(chunk.len()) > 16 * 1024 {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: DailyMemoryContext = serde_json::from_slice(&bytes).ok()?;
        result.valid_for(&request).then_some(result)
    }
}

/// Bound raw prompt scanning without splitting UTF-8 code points.
pub fn prompt_excerpt(prompt: &str) -> &str {
    let mut end = prompt.len().min(4096);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    &prompt[..end]
}

/// Match SQLite's literal term semantics, retaining whole Unicode mark sequences.
pub fn project_query(prompt: &str) -> String {
    static WORDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}\p{M}]*").expect("static Unicode word expression")
    });
    let mut seen = BTreeSet::new();
    let mut words = Vec::new();
    let mut bytes = 0;
    let excerpt = prompt_excerpt(prompt);
    for word in WORDS.find_iter(excerpt) {
        // A raw byte excerpt can end inside a word or its combining marks.
        // Omit that boundary token rather than inventing a shortened FTS term.
        if excerpt.len() < prompt.len() && word.end() == excerpt.len() {
            break;
        }
        let text = word.as_str();
        let normalized = text.to_lowercase();
        if !seen.insert(normalized) {
            continue;
        }
        let extra = text.len() + usize::from(!words.is_empty());
        if bytes + extra > 4096 {
            break;
        }
        bytes += extra;
        words.push(text);
        if words.len() == 32 {
            break;
        }
    }
    words.join(" ")
}

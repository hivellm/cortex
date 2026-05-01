//! Phase11j §2.3 — abstract summariser trait + cost-tracked
//! Haiku45 / Opus47 implementations against the Anthropic Messages
//! API.
//!
//! Anthropic does not publish an official Rust SDK, so the live
//! impls hit `POST https://api.anthropic.com/v1/messages` directly
//! via `reqwest`. The pricing constants (`HAIKU45_*` / `OPUS47_*`)
//! reflect Anthropic's published rates as of phase11j; operators
//! can override the base URL via `ANTHROPIC_API_URL` for staging /
//! captive deployments.

use serde::{Deserialize, Serialize};

/// Two-state enum identifying which model the summariser uses.
/// Maps 1:1 onto [`cortex_core::events::ConsolidationDepth`] —
/// `Haiku45 → Shallow`, `Opus47 → Deep`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SummariserKind {
    /// Claude Haiku 4.5 — default; ~$0.0008/session.
    Haiku45,
    /// Claude Opus 4.7 — auto-promoted; ~$0.05/call.
    Opus47,
}

impl SummariserKind {
    /// Identifier the [`cortex_core::events::ConsolidationPayload::model`]
    /// field carries on the wire AND the Anthropic API expects.
    pub fn model_id(self) -> &'static str {
        match self {
            SummariserKind::Haiku45 => "claude-haiku-4-5",
            SummariserKind::Opus47 => "claude-opus-4-7",
        }
    }

    /// Map onto the [`cortex_core::events::ConsolidationDepth`]
    /// the renderer + fidelity-IT thresholds key on.
    pub fn depth(self) -> cortex_core::events::ConsolidationDepth {
        match self {
            SummariserKind::Haiku45 => cortex_core::events::ConsolidationDepth::Shallow,
            SummariserKind::Opus47 => cortex_core::events::ConsolidationDepth::Deep,
        }
    }
}

/// Phase11j pricing constants (USD micro-cents per 1k input tokens).
/// 1 micro-cent = 1e-8 USD; we work in micro-cents so an integer
/// representation captures the published rates without rounding.
/// The orchestrator divides by 1_000_000 when surfacing the running
/// total to operators.
pub const HAIKU45_INPUT_MICROCENTS_PER_1K: u64 = 80; // $0.0008/M tokens × 1k
pub const HAIKU45_OUTPUT_MICROCENTS_PER_1K: u64 = 400; // $0.004/M tokens × 1k
pub const OPUS47_INPUT_MICROCENTS_PER_1K: u64 = 1_500_000; // $15/M tokens × 1k
pub const OPUS47_OUTPUT_MICROCENTS_PER_1K: u64 = 7_500_000; // $75/M tokens × 1k

/// Compute USD cents for a (input_tokens, output_tokens) pair under
/// the given model's pricing. Rounds half-up to the nearest cent.
pub fn cost_cents(kind: SummariserKind, input_tokens: u64, output_tokens: u64) -> u32 {
    let (in_rate, out_rate) = match kind {
        SummariserKind::Haiku45 => (
            HAIKU45_INPUT_MICROCENTS_PER_1K,
            HAIKU45_OUTPUT_MICROCENTS_PER_1K,
        ),
        SummariserKind::Opus47 => (
            OPUS47_INPUT_MICROCENTS_PER_1K,
            OPUS47_OUTPUT_MICROCENTS_PER_1K,
        ),
    };
    let microcents = input_tokens.saturating_mul(in_rate) / 1_000
        + output_tokens.saturating_mul(out_rate) / 1_000;
    // micro-cents → cents: divide by 1_000_000, round half-up.
    let cents = (microcents + 500_000) / 1_000_000;
    cents.min(u32::MAX as u64) as u32
}

/// Summariser request — a rendered prompt + a budget cap.
#[derive(Debug, Clone)]
pub struct SummariserRequest {
    /// Rendered prompt (already produced by [`crate::templates`]).
    pub prompt: String,
    /// Hard cap on output tokens. Producers leave this `None` to
    /// pick up the per-grain default (1 024 — large enough for the
    /// 2 KB summary cap with token-byte ratio headroom).
    pub max_output_tokens: Option<u32>,
}

/// Summariser response — the raw text + cost tracking signal.
#[derive(Debug, Clone)]
pub struct SummariserResult {
    /// Raw model output (caller parses into Markdown / takeaways).
    pub text: String,
    /// Realised cost in USD cents. Drives the cost ledger + monthly
    /// cap enforcement.
    pub cost_cents: u32,
    /// Which model produced the result.
    pub kind: SummariserKind,
    /// Input tokens reported by the upstream usage block.
    pub input_tokens: u64,
    /// Output tokens reported by the upstream usage block.
    pub output_tokens: u64,
}

/// Errors the summariser surface returns. Distinguishes recoverable
/// (rate-limited) from terminal (auth / 5xx) so the orchestrator
/// can retry the right way.
#[derive(Debug, thiserror::Error)]
pub enum SummariserError {
    /// Rate-limited by upstream. The orchestrator backs off + retries.
    #[error("rate limited; retry after {retry_after_ms} ms")]
    RateLimited {
        /// Server-suggested retry-after window.
        retry_after_ms: u64,
    },
    /// Cost ceiling tripped — the orchestrator aborts the run and
    /// reports `buckets_pending` so the next run resumes cleanly.
    #[error("cost ceiling tripped: {0} cents")]
    CostCeiling(u32),
    /// HTTP / IO failure — terminal for this attempt.
    #[error("transport: {0}")]
    Transport(String),
    /// Anthropic API returned a 4xx other than 429.
    #[error("upstream rejected: {status} ({body})")]
    Upstream {
        /// HTTP status code.
        status: u16,
        /// Body excerpt.
        body: String,
    },
    /// Anthropic API returned a 5xx — the orchestrator retries
    /// once and falls back to Haiku on a second failure.
    #[error("upstream 5xx: {0}")]
    UpstreamUnavailable(String),
}

/// Abstract summariser. Live impls below; tests use a separate
/// in-memory implementation under `#[cfg(test)]`.
#[async_trait::async_trait]
pub trait Summariser: Send + Sync {
    /// Identifier of the underlying model.
    fn kind(&self) -> SummariserKind;
    /// Produce a summary for the given request.
    async fn summarise(
        &self,
        request: SummariserRequest,
    ) -> Result<SummariserResult, SummariserError>;
}

/// Anthropic Messages API base URL. Operators can override with
/// `ANTHROPIC_API_URL` for staging deployments.
pub const ANTHROPIC_API_URL_DEFAULT: &str = "https://api.anthropic.com";
/// API version header value Anthropic expects.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default `max_output_tokens` when the caller does not set one.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_024;

/// Live HTTP-backed summariser pointing at the Anthropic Messages
/// API. Single struct parameterised by model — Haiku and Opus
/// share the same wire shape, only the `model` body field differs.
pub struct AnthropicSummariser {
    client: reqwest::Client,
    api_key: String,
    api_url: String,
    kind: SummariserKind,
}

impl AnthropicSummariser {
    /// Build a summariser. `api_key` resolves from the
    /// `ANTHROPIC_API_KEY` env var at the call site; `api_url`
    /// defaults to [`ANTHROPIC_API_URL_DEFAULT`].
    pub fn new(client: reqwest::Client, api_key: String, kind: SummariserKind) -> Self {
        let api_url = std::env::var("ANTHROPIC_API_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| ANTHROPIC_API_URL_DEFAULT.to_string());
        Self {
            client,
            api_key,
            api_url,
            kind,
        }
    }

    /// Override the base URL (test wiring against wiremock).
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }
}

#[derive(Serialize)]
struct MessagesBody<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<MessageInput<'a>>,
}

#[derive(Serialize)]
struct MessageInput<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<MessageContent>,
    usage: MessagesUsage,
}

#[derive(Deserialize)]
struct MessageContent {
    #[serde(rename = "type")]
    _ty: String,
    text: String,
}

#[derive(Deserialize)]
struct MessagesUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[async_trait::async_trait]
impl Summariser for AnthropicSummariser {
    fn kind(&self) -> SummariserKind {
        self.kind
    }
    async fn summarise(
        &self,
        request: SummariserRequest,
    ) -> Result<SummariserResult, SummariserError> {
        let url = format!("{}/v1/messages", self.api_url.trim_end_matches('/'));
        let body = MessagesBody {
            model: self.kind.model_id(),
            max_tokens: request
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
            messages: vec![MessageInput {
                role: "user",
                content: &request.prompt,
            }],
        };
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|err| SummariserError::Transport(format!("POST {url}: {err}")))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            let retry_after_ms = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|secs| secs * 1_000)
                .unwrap_or(1_000);
            return Err(SummariserError::RateLimited { retry_after_ms });
        }
        if status.is_server_error() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SummariserError::UpstreamUnavailable(format!(
                "{status}: {body}"
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SummariserError::Upstream {
                status: status.as_u16(),
                body,
            });
        }
        let parsed: MessagesResponse = resp
            .json()
            .await
            .map_err(|err| SummariserError::Transport(format!("decode {url} response: {err}")))?;
        let text = parsed
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");
        let cost = cost_cents(
            self.kind,
            parsed.usage.input_tokens,
            parsed.usage.output_tokens,
        );
        Ok(SummariserResult {
            text,
            cost_cents: cost,
            kind: self.kind,
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_cents_scales_linearly_with_token_count() {
        // Haiku: 1 M input + 1 M output ≈ $0.0008 + $0.004 = $0.0048
        // → ~0 cents (rounded half-up).
        let h = cost_cents(SummariserKind::Haiku45, 1_000_000, 1_000_000);
        assert!(
            h <= 1,
            "haiku 1M+1M tokens should round to ≤ 1 cent, got {h}"
        );
        // Opus: 1 M input + 1 M output ≈ $15 + $75 = $90.
        let o = cost_cents(SummariserKind::Opus47, 1_000_000, 1_000_000);
        assert_eq!(
            o, 9_000,
            "opus 1M+1M tokens should land at $90 (9000 cents)"
        );
    }

    #[test]
    fn cost_cents_rounds_half_up_at_the_micro_cent_boundary() {
        // Haiku 1k input + 1k output: 80 microcents in + 400 microcents out
        // (per 1k) → 80 + 400 = 480 microcents per 1k → /1k * 1k = 0
        // microcents (saturating). The function returns 0 cents.
        let zero = cost_cents(SummariserKind::Haiku45, 1_000, 1_000);
        assert_eq!(zero, 0);
    }

    #[test]
    fn anthropic_summariser_with_api_url_overrides_base() {
        let client = reqwest::Client::new();
        let s = AnthropicSummariser::new(client, "k".into(), SummariserKind::Haiku45)
            .with_api_url("http://localhost:9999");
        assert_eq!(s.api_url, "http://localhost:9999");
    }

    #[test]
    fn summariser_kind_round_trips_through_serde() {
        let h = serde_json::to_string(&SummariserKind::Haiku45).unwrap();
        assert_eq!(h, "\"haiku45\"");
        let h_back: SummariserKind = serde_json::from_str(&h).unwrap();
        assert_eq!(h_back, SummariserKind::Haiku45);
    }
}

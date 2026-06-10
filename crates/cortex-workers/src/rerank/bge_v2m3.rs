//! TEI-format cross-encoder reranker using BGE-reranker-v2-m3 (phase17 §2.2).
//!
//! Calls the HuggingFace Text Embeddings Inference `/rerank` endpoint
//! (POST `{endpoint}/rerank`) and maps the response scores back to the
//! input [`Candidate`] order.
//!
//! ## Fail-open contract (§2.5)
//!
//! [`BgeRerankerV2M3::score`] never panics. On timeout or any HTTP/decode
//! error it returns [`RerankerError`] so the orchestrator can fall back to
//! the pre-rerank fusion order and emit an audit event.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{Candidate, Reranker, RerankerError};

// ── TEI request / response shapes ────────────────────────────────────────────

#[derive(Serialize)]
struct RerankRequest<'a> {
    query: &'a str,
    texts: Vec<&'a str>,
    /// Ask TEI to return scores only (not the full text back).
    #[serde(skip_serializing_if = "Option::is_none")]
    return_text: Option<bool>,
}

#[derive(Deserialize)]
struct RerankItem {
    /// Zero-based index into the original `texts` slice.
    index: usize,
    score: f32,
}

// ── Implementation ────────────────────────────────────────────────────────────

/// BGE-reranker-v2-m3 client against a local / remote TEI service.
///
/// Constructed via [`BgeRerankerV2M3::new`]. Cheaply cloneable (inner
/// `reqwest::Client` uses an `Arc` pool internally).
#[derive(Clone)]
pub struct BgeRerankerV2M3 {
    client: reqwest::Client,
    /// Full URL including path, e.g. `http://localhost:8080/rerank`.
    rerank_url: String,
    timeout: Duration,
}

impl BgeRerankerV2M3 {
    /// Create a new client.
    ///
    /// # Arguments
    /// * `endpoint` — base URL of the TEI service (e.g. `http://localhost:8080`).
    ///   The `/rerank` path is appended automatically.
    /// * `timeout_ms` — per-request HTTP timeout in milliseconds.
    pub fn new(endpoint: &str, timeout_ms: u64) -> Self {
        let endpoint = endpoint.trim_end_matches('/');
        Self {
            client: reqwest::Client::new(),
            rerank_url: format!("{endpoint}/rerank"),
            timeout: Duration::from_millis(timeout_ms),
        }
    }
}

#[async_trait]
impl Reranker for BgeRerankerV2M3 {
    async fn score(
        &self,
        query: &str,
        candidates: &[Candidate],
    ) -> Result<Vec<f32>, RerankerError> {
        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let texts: Vec<&str> = candidates.iter().map(|c| c.text.as_str()).collect();
        let body = RerankRequest {
            query,
            texts,
            return_text: Some(false),
        };

        let resp = self
            .client
            .post(&self.rerank_url)
            .timeout(self.timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RerankerError::Timeout {
                        ms: self.timeout.as_millis() as u64,
                    }
                } else {
                    RerankerError::Http(e.to_string())
                }
            })?;

        if !resp.status().is_success() {
            return Err(RerankerError::Http(format!(
                "status {}",
                resp.status().as_u16()
            )));
        }

        let items: Vec<RerankItem> = resp
            .json()
            .await
            .map_err(|e| RerankerError::Decode(e.to_string()))?;

        // TEI returns items in descending score order with the original index.
        // Reconstruct a score vec aligned with the input candidates slice.
        let mut scores = vec![0.0f32; candidates.len()];
        for item in items {
            if item.index < scores.len() {
                scores[item.index] = item.score;
            }
        }

        Ok(scores)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_appends_rerank_path() {
        let r = BgeRerankerV2M3::new("http://localhost:8080", 500);
        assert_eq!(r.rerank_url, "http://localhost:8080/rerank");
    }

    #[test]
    fn new_strips_trailing_slash() {
        let r = BgeRerankerV2M3::new("http://localhost:8080/", 500);
        assert_eq!(r.rerank_url, "http://localhost:8080/rerank");
    }
}

//! One bounded provider attempt; durable scheduling and backoff belong to the queue.
use super::{
    Category, ClassificationAdvice, FailureKind, PreparedClassification, ProviderFailure,
    TokenUsage, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
use crate::{LocalError, Result};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER},
    Client, Url,
};
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const MAX_RETRY_AFTER_SECONDS: u64 = 300;

pub struct Provider {
    client: Client,
    endpoint: Url,
    authorization: Option<HeaderValue>,
    secret: Option<String>,
    timeout: Duration,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provider")
            .field("configured", &self.is_configured())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Provider {
    pub fn new(api_key: Option<String>, timeout: Duration) -> Result<Self> {
        Self::build(
            api_key,
            timeout,
            Url::parse(ENDPOINT).map_err(|_| invalid("invalid fixed provider endpoint"))?,
            true,
        )
    }

    /// Explicit test transport. DNS names and non-loopback destinations are forbidden.
    /// Production configuration must use `new`, whose TLS endpoint is fixed.
    pub fn for_loopback(
        api_key: Option<String>,
        timeout: Duration,
        endpoint: &str,
    ) -> Result<Self> {
        let endpoint =
            Url::parse(endpoint).map_err(|_| invalid("invalid loopback test endpoint"))?;
        let literal = endpoint.host_str().and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        });
        if endpoint.scheme() != "http"
            || !literal.is_some_and(|address| address.is_loopback())
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.port().is_some_and(|port| port != 0)
            || endpoint.path() != "/v1/systemone"
        {
            return Err(invalid(
                "provider test endpoint must be an explicit literal loopback HTTP address",
            ));
        }
        Self::build(api_key, timeout, endpoint, false)
    }

    fn build(
        api_key: Option<String>,
        timeout: Duration,
        endpoint: Url,
        tls_only: bool,
    ) -> Result<Self> {
        if timeout < Duration::from_secs(1) || timeout > Duration::from_secs(30) {
            return Err(invalid("provider timeout must be between 1 and 30 seconds"));
        }
        let secret = api_key.filter(|value| !value.is_empty());
        let authorization = secret
            .as_ref()
            .map(|value| {
                if value.len() > 4096
                    || value.trim() != value
                    || value.chars().any(char::is_control)
                {
                    return Err(invalid("invalid provider credential format"));
                }
                let mut header = HeaderValue::from_str(&format!("Bearer {value}"))
                    .map_err(|_| invalid("invalid provider credential format"))?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(tls_only)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .build()
            .map_err(|_| invalid("could not initialize classification transport"))?;
        Ok(Self {
            client,
            endpoint,
            authorization,
            secret,
            timeout,
        })
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.authorization.is_some()
    }

    pub async fn classify(
        &self,
        input: &PreparedClassification,
    ) -> std::result::Result<ClassificationAdvice, ProviderFailure> {
        let authorization = self
            .authorization
            .as_ref()
            .ok_or_else(|| failure(FailureKind::Unavailable, true))?;
        let body = crate::files::canonical(&input.request_body())
            .map_err(|_| failure(FailureKind::Rejected, false))?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(failure(FailureKind::Oversized, false));
        }
        input
            .validate()
            .map_err(|_| failure(FailureKind::Rejected, false))?;
        if self.secret.as_ref().is_some_and(|secret| {
            body.windows(secret.len())
                .any(|window| window == secret.as_bytes())
                || contains_secret(&input.state, secret)
        }) {
            return Err(failure(FailureKind::Rejected, false));
        }
        match tokio::time::timeout(self.timeout, self.execute(input, body, authorization)).await {
            Ok(result) => result,
            Err(_) => Err(failure(FailureKind::Timeout, true)),
        }
    }

    async fn execute(
        &self,
        input: &PreparedClassification,
        body: Vec<u8>,
        authorization: &HeaderValue,
    ) -> std::result::Result<ClassificationAdvice, ProviderFailure> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, authorization.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_failure)?;
        let status = response.status();
        if !status.is_success() {
            let (kind, retryable) = match status.as_u16() {
                401 | 403 => (FailureKind::Authentication, false),
                408 => (FailureKind::Timeout, true),
                429 => (FailureKind::RateLimited, true),
                300..=399 => (FailureKind::Redirect, false),
                500..=599 => (FailureKind::Server, true),
                _ => (FailureKind::Rejected, false),
            };
            let mut error = failure(kind, retryable);
            if retryable {
                error.retry_after_secs = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|value| value.clamp(1, MAX_RETRY_AFTER_SECONDS));
            }
            return Err(error);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(failure(FailureKind::Oversized, false));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_failure)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(failure(FailureKind::Oversized, false));
            }
            bytes.extend_from_slice(&chunk);
        }
        decode(&bytes, &input.requested_model)
    }
}

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(message.into())
}
fn contains_secret(value: &serde_json::Value, secret: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value.contains(secret),
        serde_json::Value::Array(values) => {
            values.iter().any(|value| contains_secret(value, secret))
        }
        serde_json::Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_secret(value, secret)),
        _ => false,
    }
}
fn failure(kind: FailureKind, retryable: bool) -> ProviderFailure {
    ProviderFailure {
        kind,
        retryable,
        retry_after_secs: None,
        usage: None,
    }
}
fn transport_failure(error: reqwest::Error) -> ProviderFailure {
    failure(
        if error.is_timeout() {
            FailureKind::Timeout
        } else {
            FailureKind::Transport
        },
        true,
    )
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    usage: WireUsage,
    answers: WireAnswers,
}
#[derive(Deserialize)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
}
#[derive(Deserialize)]
struct WireAnswers {
    category: WireChoice,
    durable: WireNoul,
}
#[derive(Deserialize)]
struct WireChoice {
    #[serde(rename = "type")]
    kind: String,
    choice: Category,
    probabilities: BTreeMap<Category, f64>,
    confidence: f64,
}
#[derive(Deserialize)]
struct WireNoul {
    #[serde(rename = "type")]
    kind: String,
    noul: f64,
}

fn decode(bytes: &[u8], model: &str) -> std::result::Result<ClassificationAdvice, ProviderFailure> {
    let raw: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| failure(FailureKind::InvalidResponse, false))?;
    let usage = raw
        .get("usage")
        .and_then(|value| serde_json::from_value::<WireUsage>(value.clone()).ok())
        .filter(|usage| {
            usage.input_tokens <= i64::MAX as u64 && usage.output_tokens <= i64::MAX as u64
        })
        .map(|usage| TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        });
    let invalid = || ProviderFailure {
        usage: usage.clone(),
        ..failure(FailureKind::InvalidResponse, false)
    };
    let wire: WireResponse = serde_json::from_value(raw).map_err(|_| invalid())?;
    if wire.answers.category.kind != "choice"
        || wire.answers.durable.kind != "noul"
        || wire.usage.input_tokens > i64::MAX as u64
        || wire.usage.output_tokens > i64::MAX as u64
    {
        return Err(invalid());
    }
    let advice = ClassificationAdvice {
        category: wire.answers.category.choice,
        probabilities: wire.answers.category.probabilities,
        confidence: wire.answers.category.confidence,
        durable_probability: wire.answers.durable.noul,
        model: wire.model,
        usage: TokenUsage {
            input_tokens: wire.usage.input_tokens,
            output_tokens: wire.usage.output_tokens,
        },
    };
    advice.validate(model).map_err(|_| invalid())?;
    Ok(advice)
}

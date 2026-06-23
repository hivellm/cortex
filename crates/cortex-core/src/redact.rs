//! Static-pattern redactor.
//!
//! Walks a [`serde_json::Value`] and replaces substrings that match known
//! secret patterns with `[REDACTED:<class>]`, emitting one opaque token per
//! hit as a `secret:<class>:<locator>` string.
//!
//! Patterns are versioned and exposed as [`PATTERN_CATALOG_V1`] so adapters
//! (spec 10 / 17) can mirror the exact same set on the client side.
//!
//! ## Ordering invariant (phase21 §3.5 — ADR-034)
//!
//! Redaction runs in the **adapter layer** before the event is published to
//! Synap, which means it executes *before* any classification stamping in the
//! classifier worker. The two mechanisms compose sequentially:
//!
//! 1. **Redaction** removes the raw secret from the payload (irreversible).
//! 2. **Classification** gates visibility of the already-redacted fact per
//!    principal.
//!
//! The `[REDACTED:<class>]` tokens that redaction leaves in the payload are
//! *intentionally detectable* by the classifier's sensitivity rules (see
//! `cortex_workers::classifier::statics::SENSITIVITY_RULES` — the
//! `\[REDACTED:` pattern maps to `internal / customer_pii`). This means a
//! fact that originally contained raw PII is still classified at ≥ `internal`
//! after redaction, preventing silent downgrade. Neither step substitutes for
//! the other.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

/// A single classification rule used by the redactor.
#[derive(Debug, Clone)]
pub struct Pattern {
    /// Short identifier used in the `secret:<class>:...` token.
    pub class: &'static str,
    /// Compiled regex.
    pub regex: Regex,
}

/// Pattern catalog v1 — additive-only.
pub static PATTERN_CATALOG_V1: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        Pattern {
            class: "aws_access_key_id",
            regex: Regex::new(r"\b(?:AKIA|ASIA|AIDA|AGPA|AROA|AIPA)[0-9A-Z]{16}\b").unwrap(),
        },
        Pattern {
            class: "aws_secret_access_key",
            regex: Regex::new(r"\baws_secret_access_key\s*=\s*([A-Za-z0-9/+=]{40})\b").unwrap(),
        },
        Pattern {
            class: "github_token",
            regex: Regex::new(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b").unwrap(),
        },
        Pattern {
            class: "slack_token",
            regex: Regex::new(r"\bxox[abpsr]-[A-Za-z0-9-]{10,}\b").unwrap(),
        },
        Pattern {
            class: "openai_api_key",
            regex: Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").unwrap(),
        },
        Pattern {
            class: "anthropic_api_key",
            regex: Regex::new(r"\bsk-ant-[A-Za-z0-9_-]{32,}\b").unwrap(),
        },
        Pattern {
            class: "google_api_key",
            regex: Regex::new(r"\bAIza[0-9A-Za-z_-]{35}\b").unwrap(),
        },
        Pattern {
            class: "stripe_live_key",
            regex: Regex::new(r"\bsk_live_[0-9A-Za-z]{20,}\b").unwrap(),
        },
        Pattern {
            class: "bearer_token",
            regex: Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9\-_\.=]+").unwrap(),
        },
        Pattern {
            class: "private_key_pem",
            regex: Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----[\s\S]+?-----END (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----").unwrap(),
        },
        Pattern {
            class: "jwt",
            regex: Regex::new(r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\b").unwrap(),
        },
        Pattern {
            class: "generic_env_secret",
            regex: Regex::new(r"(?im)^\s*(?:export\s+)?(?:[A-Z0-9_]+(?:KEY|TOKEN|SECRET|PASSWORD|PASS|PWD)[A-Z0-9_]*)\s*=\s*\S+").unwrap(),
        },
    ]
});

/// Result of a redaction pass.
#[derive(Debug, Clone, Default)]
pub struct RedactReport {
    /// Redaction tokens in `secret:<class>:<locator>` form.
    pub tokens: Vec<String>,
}

/// Redact sensitive substrings in `value`, replacing matches with
/// `[REDACTED:<class>]`. Returns the list of opaque locator tokens that can
/// be stored on the event envelope.
pub fn redact(value: &mut Value) -> RedactReport {
    let mut report = RedactReport::default();
    redact_inner(value, "", &PATTERN_CATALOG_V1, &mut report);
    report
}

fn redact_inner(value: &mut Value, path: &str, patterns: &[Pattern], report: &mut RedactReport) {
    match value {
        Value::String(s) => {
            let mut any_changed = false;
            for p in patterns {
                let mut out = String::with_capacity(s.len());
                let mut last = 0;
                let mut count_in_field = 0u32;
                for m in p.regex.find_iter(s) {
                    out.push_str(&s[last..m.start()]);
                    out.push_str("[REDACTED:");
                    out.push_str(p.class);
                    out.push(']');
                    last = m.end();
                    count_in_field += 1;
                    report.tokens.push(format!(
                        "secret:{}:{}:offset={}:length={}",
                        p.class,
                        path,
                        m.start(),
                        m.len()
                    ));
                }
                if count_in_field > 0 {
                    out.push_str(&s[last..]);
                    *s = out;
                    any_changed = true;
                }
                if any_changed {
                    // Re-scan with the next pattern against the now-redacted string.
                    continue;
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                let p = if path.is_empty() {
                    format!("[{i}]")
                } else {
                    format!("{path}[{i}]")
                };
                redact_inner(item, &p, patterns, report);
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                let p = if path.is_empty() {
                    k.to_string()
                } else {
                    format!("{path}.{k}")
                };
                redact_inner(v, &p, patterns, report);
            }
        }
        _ => {}
    }
}

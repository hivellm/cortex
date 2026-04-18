//! Static-pattern redactor.
//!
//! Walks a [`serde_json::Value`] and replaces substrings that match known
//! secret patterns with `[REDACTED:<class>]`, emitting one opaque token per
//! hit as a `secret:<class>:<locator>` string.
//!
//! Patterns are versioned and exposed as [`PATTERN_CATALOG_V1`] so adapters
//! (spec 10 / 17) can mirror the exact same set on the client side.

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_aws_access_key_in_string() {
        let mut v = json!({ "input": { "command": "export AWS_KEY=AKIAIOSFODNN7EXAMPLE" } });
        let report = redact(&mut v);
        assert!(v["input"]["command"].as_str().unwrap().contains("[REDACTED:"));
        assert!(!report.tokens.is_empty());
    }

    #[test]
    fn redacts_github_token_inline() {
        let mut v = json!("curl -H 'Authorization: token ghp_abcdefghijklmnopqrstuvwxyz0123456789'");
        let report = redact(&mut v);
        assert!(v.as_str().unwrap().contains("[REDACTED:github_token]"));
        assert_eq!(report.tokens.len(), 1);
        assert!(report.tokens[0].starts_with("secret:github_token:"));
    }

    #[test]
    fn redacts_anthropic_key() {
        let mut v = json!("ANTHROPIC_API_KEY=sk-ant-0123456789abcdefghijklmnopqrstuvwxyz");
        let report = redact(&mut v);
        assert!(!report.tokens.is_empty());
        assert!(v.as_str().unwrap().contains("[REDACTED"));
    }

    #[test]
    fn redacts_private_key_block() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAAS==\n-----END PRIVATE KEY-----";
        let mut v = json!(pem);
        let report = redact(&mut v);
        assert!(!report.tokens.is_empty());
        assert!(v.as_str().unwrap().contains("[REDACTED:private_key_pem]"));
    }

    #[test]
    fn preserves_non_secret_content() {
        let mut v = json!({ "user_message": "please refactor the hnsw search function" });
        let report = redact(&mut v);
        assert!(report.tokens.is_empty());
        assert_eq!(
            v["user_message"].as_str().unwrap(),
            "please refactor the hnsw search function"
        );
    }

    #[test]
    fn walks_nested_arrays_and_objects() {
        let mut v = json!({
            "a": ["ok", "sk-abcdefghijklmnopqrstuvwx"],
            "b": { "c": "xoxb-12345678-abcdef0123456789" }
        });
        let report = redact(&mut v);
        assert_eq!(report.tokens.len(), 2);
        let paths: Vec<_> = report.tokens.iter().map(|t| t.as_str()).collect();
        assert!(paths.iter().any(|t| t.contains("a[1]")));
        assert!(paths.iter().any(|t| t.contains("b.c")));
    }

    #[test]
    fn locator_records_offset_and_length() {
        let mut v = json!("prefix AKIAIOSFODNN7EXAMPLE suffix");
        let report = redact(&mut v);
        assert_eq!(report.tokens.len(), 1);
        let tok = &report.tokens[0];
        assert!(tok.contains("offset=7"));
        assert!(tok.contains("length=20"));
    }

    #[test]
    fn idempotent_on_already_redacted_value() {
        let mut v = json!("[REDACTED:github_token] hello");
        let report = redact(&mut v);
        assert!(report.tokens.is_empty());
    }
}

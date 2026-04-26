//! Final read-time redaction pass. Spec 11 §Security / privacy:
//! response bodies pass through `cortex_core::redact` again so a
//! single bad write (storage writers are authoritative but fallible)
//! never leaks forever.
//!
//! The pass walks every caller-visible string field — `text`,
//! `summary`, decision titles, violation messages — in place. Every
//! redaction observed contributes to the response's
//! `debug.errors.redacted` summary.

use serde_json::Value;

use crate::types::QueryResponse;

/// Apply the redactor to every string field on the response. Returns
/// the number of redactions performed.
pub fn redact_response(response: &mut QueryResponse) -> u64 {
    let mut count: u64 = 0;
    // Snippets — `text`, `path`, `symbol`, `why`.
    for s in response.results.snippets.iter_mut() {
        count += redact_string(&mut s.text);
        if let Some(ref mut p) = s.path {
            count += redact_string(p);
        }
        if let Some(ref mut sym) = s.symbol {
            count += redact_string(sym);
        }
        if let Some(ref mut why) = s.why {
            count += redact_string(why);
        }
    }
    // Decisions — title.
    for d in response.results.decisions.iter_mut() {
        count += redact_string(&mut d.title);
    }
    // Violations — message.
    for v in response.results.violations.iter_mut() {
        count += redact_string(&mut v.message);
    }
    // Similar turns — summary.
    for t in response.results.similar_turns.iter_mut() {
        count += redact_string(&mut t.summary);
    }
    // Active laws — title.
    for l in response.laws_active.iter_mut() {
        count += redact_string(&mut l.title);
    }
    if count > 0 {
        response
            .debug
            .errors
            .insert("redacted".to_string(), count.to_string());
    }
    count
}

fn redact_string(s: &mut String) -> u64 {
    let mut value = Value::String(std::mem::take(s));
    let report = cortex_core::redact::redact(&mut value);
    *s = match value {
        Value::String(out) => out,
        other => other.to_string(),
    };
    report.tokens.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{QueryResponse, ResultsBag, Snippet};

    #[test]
    fn aws_key_in_snippet_text_is_redacted() {
        let mut resp = QueryResponse {
            intent: "free_search".into(),
            query_id: "q1".into(),
            scope_resolved: Default::default(),
            results: ResultsBag {
                snippets: vec![Snippet {
                    rank: 1,
                    source: "vector".into(),
                    collection: None,
                    repo: None,
                    path: None,
                    symbol: None,
                    content_hash: None,
                    text: "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000".to_string(),
                    score: 0.0,
                    why: None,
                }],
                ..Default::default()
            },
            laws_active: Vec::new(),
            budget: Default::default(),
            debug: Default::default(),
        };
        let n = redact_response(&mut resp);
        assert!(n >= 1);
        assert!(!resp.results.snippets[0].text.contains("AKIAIOSFODNN7EXAMPLE0000"));
    }

    #[test]
    fn clean_response_records_no_redactions() {
        let mut resp = QueryResponse {
            intent: "free_search".into(),
            query_id: "q1".into(),
            scope_resolved: Default::default(),
            results: ResultsBag::default(),
            laws_active: Vec::new(),
            budget: Default::default(),
            debug: Default::default(),
        };
        assert_eq!(redact_response(&mut resp), 0);
        assert!(resp.debug.errors.is_empty());
    }
}

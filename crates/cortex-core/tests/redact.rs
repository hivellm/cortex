//! Integration tests for `cortex_core::redact`.

use cortex_core::redact;
use serde_json::json;

#[test]
fn redacts_aws_access_key_in_string() {
    let mut v = json!({ "input": { "command": "export AWS_KEY=AKIAIOSFODNN7EXAMPLE" } });
    let report = redact(&mut v);
    assert!(v["input"]["command"]
        .as_str()
        .unwrap()
        .contains("[REDACTED:"));
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
    let pem =
        "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAAS==\n-----END PRIVATE KEY-----";
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

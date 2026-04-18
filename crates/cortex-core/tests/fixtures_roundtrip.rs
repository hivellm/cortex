//! Loads every fixture under `tests/fixtures/` and exercises the public API.
//!
//! - `events/*.json` must pass validation, deserialize into an [`Envelope`],
//!   and round-trip canonical bytes stably.
//! - `invalid/*.json` must fail validation with at least one error.

use cortex_core::{canonicalize, content_hash, validate_event, Envelope, Kind};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir(sub: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(sub);
    p
}

fn read_json(path: &PathBuf) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

fn read_dir_sorted(sub: &str) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(fixtures_dir(sub))
        .expect("fixtures dir exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    entries.sort();
    entries
}

#[test]
fn all_valid_fixtures_validate() {
    let mut seen = 0usize;
    for path in read_dir_sorted("events") {
        let value = read_json(&path);
        let result = validate_event(&value);
        if let Err(errors) = &result {
            panic!(
                "fixture {} failed validation: {:?}",
                path.display(),
                errors
            );
        }
        seen += 1;
    }
    assert!(seen >= 10, "expected at least 10 valid fixtures, got {seen}");
}

#[test]
fn all_valid_fixtures_deserialize_into_envelope() {
    for path in read_dir_sorted("events") {
        let value = read_json(&path);
        let envelope: Envelope = serde_json::from_value(value.clone())
            .unwrap_or_else(|e| panic!("{} does not deserialize: {e}", path.display()));
        // Round-trip: struct → JSON → struct produces the same bytes after canonicalization.
        let re_encoded = serde_json::to_value(&envelope).expect("re-encode");
        let a = canonicalize(&value).expect("canonicalize original");
        let b = canonicalize(&re_encoded).expect("canonicalize re-encoded");
        assert_eq!(
            a,
            b,
            "round-trip for {} lost data:\n  original: {}\n  re-encoded: {}",
            path.display(),
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&b)
        );
    }
}

#[test]
fn invalid_fixtures_fail_validation() {
    let mut seen = 0usize;
    for path in read_dir_sorted("invalid") {
        let value = read_json(&path);
        match validate_event(&value) {
            Ok(()) => panic!("invalid fixture {} unexpectedly validated", path.display()),
            Err(errors) => assert!(!errors.is_empty(), "no errors reported for {}", path.display()),
        }
        seen += 1;
    }
    assert!(seen >= 3, "expected at least 3 invalid fixtures, got {seen}");
}

#[test]
fn every_kind_has_a_fixture() {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    for path in read_dir_sorted("events") {
        let value = read_json(&path);
        let kind = value.get("kind").and_then(|k| k.as_str()).unwrap().to_string();
        seen.insert(kind);
    }
    for kind in [
        Kind::Turn,
        Kind::ToolCall,
        Kind::AgentCall,
        Kind::Memory,
        Kind::Decision,
        Kind::Analysis,
        Kind::LawViolation,
        Kind::Artifact,
    ] {
        assert!(
            seen.contains(kind.schema_stem()),
            "missing fixture for kind `{}`",
            kind.schema_stem()
        );
    }
}

#[test]
fn content_hash_is_stable_across_platforms() {
    // The hash of canonical-JSON for a known payload must not depend on host byte order,
    // line endings, or locale. Known vector reproduces across win32 / darwin / linux.
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "input": { "command": "echo hi" },
        "outcome": "success"
    });
    let hash = content_hash(&payload).expect("hash");
    assert_eq!(
        hash.as_str(),
        "sha256:eebb1198940d67249bc6c9b0b43ba3c34deeb718be11c7eb3320d2e059364016"
    );
}

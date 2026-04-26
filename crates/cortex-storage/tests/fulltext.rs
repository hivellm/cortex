//! Integration tests for `cortex_storage::fulltext`.

use cortex_storage::fulltext::INDEXES;

#[test]
fn every_index_parses() {
    for idx in INDEXES {
        let v: serde_json::Value = serde_json::from_str(idx.settings_json)
            .unwrap_or_else(|e| panic!("settings for {} do not parse: {e}", idx.name));
        assert!(v.is_object(), "settings must be a JSON object for {}", idx.name);
    }
}

#[test]
fn settings_have_searchable_attributes() {
    for idx in INDEXES {
        let v: serde_json::Value = serde_json::from_str(idx.settings_json).unwrap();
        assert!(
            v.get("searchableAttributes").is_some(),
            "{} missing searchableAttributes",
            idx.name
        );
    }
}

#[test]
fn index_primary_keys_are_non_empty() {
    for idx in INDEXES {
        assert!(!idx.primary_key.is_empty());
    }
}

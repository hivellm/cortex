//! ADR-016 env-name → typed-field mapping.
//!
//! Each entry is `(env_name, json_pointer)`. The JSON pointer is
//! the path serde would walk inside [`crate::Config`] (e.g.
//! `"/embedder/vectorizer_url"`). [`crate::load::env_overlay`]
//! walks this table, reads each env var, and stitches a
//! `serde_json::Value` tree with the right nesting before merging
//! it on top of the TOML / default values.
//!
//! Why a hand-rolled table instead of `#[serde(alias)]`:
//! `serde(alias)` resolves names at the SAME nesting level as the
//! field — it cannot map a flat env name like
//! `CORTEX_EMBEDDER_VECTORIZER_URL` onto a nested path
//! `embedder.vectorizer_url`. The table here keeps the mapping
//! explicit + greppable + testable; round-trip tests pin every
//! entry so a rename either updates BOTH columns or fails CI.

/// `(env_name, json_pointer)` for every operator-facing
/// `CORTEX_*` knob the workspace reads. Sorted by env name so
/// `binary_search_by_key` is valid (the round-trip test pins
/// the sort).
pub const KNOWN_ENV_NAMES: &[(&str, &str)] = &[
    ("CORTEX_API_BIND", "/dashboard/api_bind"),
    ("CORTEX_API_SYNAP_URL", "/dashboard/synap_url"),
    (
        "CORTEX_ARCHIVE_REFRESH_SECS",
        "/dashboard/archive_refresh_secs",
    ),
    ("CORTEX_ARCHIVE_ROOT", "/ingestion/archive_root"),
    ("CORTEX_ARCHIVE_ZSTD", "/ingestion/archive_zstd_level"),
    (
        "CORTEX_EMBEDDER_CHUNKER_CONCURRENCY",
        "/embedder/chunker_concurrency",
    ),
    (
        "CORTEX_EMBEDDER_COLLECTION_PREFIX",
        "/embedder/collection_prefix",
    ),
    ("CORTEX_EMBEDDER_DIM", "/embedder/vector_dim"),
    ("CORTEX_EMBEDDER_MAX_RETRY", "/embedder/max_retry"),
    ("CORTEX_EMBEDDER_SYNAP_URL", "/embedder/synap_url"),
    ("CORTEX_EMBEDDER_UPSERT_BATCH", "/embedder/upsert_batch"),
    (
        "CORTEX_EMBEDDER_VECTORIZER_PASSWORD",
        "/embedder/vectorizer_password",
    ),
    ("CORTEX_EMBEDDER_VECTORIZER_URL", "/embedder/vectorizer_url"),
    (
        "CORTEX_EMBEDDER_VECTORIZER_USER",
        "/embedder/vectorizer_user",
    ),
    ("CORTEX_EMBEDDER_WORKERS", "/embedder/workers"),
    ("CORTEX_FULLTEXT_AWAIT_TASK", "/meili/await_task"),
    ("CORTEX_FULLTEXT_BATCH", "/meili/upsert_batch"),
    ("CORTEX_FULLTEXT_FLUSH_MS", "/meili/flush_ms"),
    ("CORTEX_FULLTEXT_INDEX_PREFIX", "/meili/index_prefix"),
    ("CORTEX_FULLTEXT_MAX_BODY_BYTES", "/meili/max_body_bytes"),
    ("CORTEX_FULLTEXT_MAX_RETRY", "/meili/max_retry"),
    ("CORTEX_FULLTEXT_MEILI_API_KEY", "/meili/meili_api_key"),
    ("CORTEX_FULLTEXT_MEILI_URL", "/meili/meili_url"),
    ("CORTEX_FULLTEXT_SYNAP_GROUP", "/meili/synap_group"),
    ("CORTEX_FULLTEXT_SYNAP_URL", "/meili/synap_url"),
    ("CORTEX_FULLTEXT_WORKERS", "/meili/workers"),
    ("CORTEX_GRAPH_FLUSH_MS", "/nexus/flush_ms"),
    ("CORTEX_GRAPH_MAX_RETRY", "/nexus/max_retry"),
    ("CORTEX_GRAPH_NEXUS_PASSWORD", "/nexus/nexus_password"),
    ("CORTEX_GRAPH_NEXUS_URL", "/nexus/nexus_url"),
    ("CORTEX_GRAPH_NEXUS_USER", "/nexus/nexus_user"),
    (
        "CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS",
        "/nexus/out_of_order_buffer_secs",
    ),
    ("CORTEX_GRAPH_PATCH_BATCH", "/nexus/patch_batch"),
    ("CORTEX_GRAPH_SYNAP_GROUP", "/nexus/synap_group"),
    ("CORTEX_GRAPH_SYNAP_URL", "/nexus/synap_url"),
    ("CORTEX_GRAPH_TRANSPORT", "/nexus/transport"),
    ("CORTEX_GRAPH_WORKERS", "/nexus/workers"),
    ("CORTEX_INGESTION_BIND", "/ingestion/bind"),
    ("CORTEX_MEILI_REFRESH_SECS", "/dashboard/meili_refresh_secs"),
    ("CORTEX_METADATA_DB", "/ingestion/metadata_db"),
    ("CORTEX_NEXUS_URL", "/nexus/nexus_url"),
    ("CORTEX_PRE_THINKING_KB", "/pre_thinking/bundle_kb"),
    ("CORTEX_PRE_THINKING_TIMEOUT_MS", "/pre_thinking/timeout_ms"),
    ("CORTEX_RETENTION_BATCH_SIZE", "/retention/batch_size"),
    (
        "CORTEX_RETENTION_FP32_TO_PQ_DAYS",
        "/retention/fp32_to_pq_days",
    ),
    ("CORTEX_RETENTION_NOW", "/retention/now_override"),
    (
        "CORTEX_RETENTION_PQ_TO_BINARY_DAYS",
        "/retention/pq_to_binary_days",
    ),
    ("CORTEX_SYNAP_URL", "/ingestion/synap_url"),
];

/// Reverse lookup: given an env name, return the JSON pointer
/// that points at the matching typed field, or `None` when the
/// name is not in [`KNOWN_ENV_NAMES`].
pub fn env_name_for(env_name: &str) -> Option<&'static str> {
    KNOWN_ENV_NAMES
        .binary_search_by_key(&env_name, |(n, _)| n)
        .ok()
        .map(|i| KNOWN_ENV_NAMES[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_env_names_table_stays_sorted_for_binary_search() {
        let mut sorted: Vec<&'static str> = KNOWN_ENV_NAMES.iter().map(|(n, _)| *n).collect();
        let original = sorted.clone();
        sorted.sort();
        assert_eq!(
            sorted, original,
            "KNOWN_ENV_NAMES must stay ASCII-sorted — env_name_for relies on binary_search"
        );
    }

    #[test]
    fn env_name_for_resolves_a_handful_of_known_knobs() {
        assert_eq!(
            env_name_for("CORTEX_EMBEDDER_VECTORIZER_URL"),
            Some("/embedder/vectorizer_url")
        );
        assert_eq!(
            env_name_for("CORTEX_FULLTEXT_MEILI_URL"),
            Some("/meili/meili_url")
        );
        assert_eq!(
            env_name_for("CORTEX_RETENTION_BATCH_SIZE"),
            Some("/retention/batch_size")
        );
    }

    #[test]
    fn env_name_for_returns_none_for_unknown_name() {
        assert!(env_name_for("CORTEX_NOT_A_REAL_KNOB").is_none());
        assert!(env_name_for("FOOBAR").is_none());
    }

    #[test]
    fn no_duplicate_env_names() {
        let mut names: Vec<&'static str> = KNOWN_ENV_NAMES.iter().map(|(n, _)| *n).collect();
        let before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            before,
            "KNOWN_ENV_NAMES contains a duplicate env name"
        );
    }
}

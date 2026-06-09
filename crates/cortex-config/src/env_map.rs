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
    ("CORTEX_ADAPTER_ADMIN_PORT", "/adapter/adapter_admin_port"),
    ("CORTEX_ADAPTER_DISABLE", "/adapter/adapter_disable"),
    ("CORTEX_ADAPTER_HTTP_BIND", "/adapter/http_bind"),
    ("CORTEX_ADAPTER_PIPE", "/adapter/adapter_pipe"),
    ("CORTEX_ADAPTER_SOCK", "/adapter/adapter_sock"),
    ("CORTEX_ANALYZER_API_BASE", "/analyzer/api_base"),
    ("CORTEX_ANALYZER_API_KEY", "/analyzer/api_key"),
    ("CORTEX_ANALYZER_BIN", "/analyzer/bin"),
    ("CORTEX_ANALYZER_MODEL", "/analyzer/model"),
    ("CORTEX_API_BIND", "/dashboard/api_bind"),
    ("CORTEX_API_KEYS_DB", "/dashboard/api_keys_db"),
    ("CORTEX_API_SYNAP_URL", "/dashboard/synap_url"),
    ("CORTEX_API_TOKEN", "/dashboard/api_token"),
    ("CORTEX_API_URL", "/dashboard/api_url"),
    (
        "CORTEX_ARCHIVE_REFRESH_SECS",
        "/dashboard/archive_refresh_secs",
    ),
    ("CORTEX_ARCHIVE_ROOT", "/ingestion/archive_root"),
    (
        "CORTEX_ARCHIVE_WATCHER_URLS",
        "/ingestion/archive_watcher_urls",
    ),
    ("CORTEX_ARCHIVE_ZSTD", "/ingestion/archive_zstd_level"),
    ("CORTEX_AUTO_MEMORY_PROJECT", "/auto_memory/project"),
    ("CORTEX_CANARY_DEADLINE_SECS", "/canary/deadline_secs"),
    ("CORTEX_CANARY_ENABLED", "/canary/enabled"),
    ("CORTEX_CANARY_INTERVAL_SECS", "/canary/interval_secs"),
    ("CORTEX_CAS_DB", "/ingestion/cas_db"),
    ("CORTEX_CLASSIFIER_HEALTH_URL", "/classifier/health_url"),
    (
        "CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS",
        "/classifier/max_consume_errors",
    ),
    ("CORTEX_CLASSIFIER_MODE", "/classifier/mode"),
    ("CORTEX_CLASSIFIER_MODEL", "/classifier/model"),
    (
        "CORTEX_CLASSIFIER_PROMPT_VERSION",
        "/classifier/prompt_version",
    ),
    ("CORTEX_CLASSIFIER_STALENESS_MS", "/classifier/staleness_ms"),
    ("CORTEX_CLASSIFIER_SYNAP_URL", "/classifier/synap_url"),
    ("CORTEX_CLAUDE_ARCHIVE_BIND", "/claude_archive/bind"),
    ("CORTEX_CLAUDE_ARCHIVE_POLL_MS", "/claude_archive/poll_ms"),
    ("CORTEX_CLAUDE_ARCHIVE_ROOT", "/claude_archive/root"),
    (
        "CORTEX_CONSOLIDATIONS_FALLBACK_FILE",
        "/consolidator/fallback_file",
    ),
    (
        "CORTEX_CONSOLIDATIONS_FALLBACK_ROTATE_BYTES",
        "/consolidator/fallback_rotate_bytes",
    ),
    (
        "CORTEX_CONSOLIDATOR_CURSOR_FILE",
        "/consolidator/cursor_file",
    ),
    ("CORTEX_COVERAGE_SLUGS", "/dashboard/coverage_slugs"),
    (
        "CORTEX_COVERAGE_SLUGS_ONLY",
        "/dashboard/coverage_slugs_only",
    ),
    ("CORTEX_CROSS_PROJECT_ENABLED", "/cross_project/enabled"),
    ("CORTEX_CROSS_PROJECT_MAX_HOPS", "/cross_project/max_hops"),
    ("CORTEX_DASHBOARD_MEMORY_TAIL", "/dashboard/memory_tail"),
    ("CORTEX_DASHBOARD_WATCH", "/dashboard/watch"),
    ("CORTEX_DOCTOR_BENCH", "/doctor/bench"),
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
    ("CORTEX_EMBEDDER_VECTORIZER_JWT", "/embedder/vectorizer_jwt"),
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
    ("CORTEX_FULLTEXT_MEILI_KEY", "/meili/meili_api_key"),
    ("CORTEX_FULLTEXT_MEILI_URL", "/meili/meili_url"),
    ("CORTEX_FULLTEXT_REPLAY_MISSING", "/meili/replay_missing"),
    ("CORTEX_FULLTEXT_SYNAP_GROUP", "/meili/synap_group"),
    ("CORTEX_FULLTEXT_SYNAP_URL", "/meili/synap_url"),
    ("CORTEX_FULLTEXT_WORKERS", "/meili/workers"),
    ("CORTEX_GRAPH_CONSUMER_ID", "/nexus/consumer_id"),
    ("CORTEX_GRAPH_CYPHER_DIR", "/nexus/cypher_dir"),
    ("CORTEX_GRAPH_CYPHER_ENABLED", "/nexus/cypher_enabled"),
    ("CORTEX_GRAPH_FLUSH_MS", "/nexus/flush_ms"),
    ("CORTEX_GRAPH_MAX_RETRY", "/nexus/max_retry"),
    ("CORTEX_GRAPH_METADATA_DB", "/nexus/metadata_db"),
    ("CORTEX_GRAPH_NEXUS_PASSWORD", "/nexus/nexus_password"),
    ("CORTEX_GRAPH_NEXUS_URL", "/nexus/nexus_url"),
    ("CORTEX_GRAPH_NEXUS_USER", "/nexus/nexus_user"),
    (
        "CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS",
        "/nexus/out_of_order_buffer_secs",
    ),
    ("CORTEX_GRAPH_PATCH_BATCH", "/nexus/patch_batch"),
    (
        "CORTEX_GRAPH_PROJECTION_ENABLED",
        "/nexus/projection_enabled",
    ),
    (
        "CORTEX_GRAPH_SWEEPER_INTERVAL_SECS",
        "/nexus/sweeper_interval_secs",
    ),
    ("CORTEX_GRAPH_SYNAP_GROUP", "/nexus/synap_group"),
    ("CORTEX_GRAPH_SYNAP_URL", "/nexus/synap_url"),
    ("CORTEX_GRAPH_TRANSPORT", "/nexus/transport"),
    ("CORTEX_GRAPH_WORKERS", "/nexus/workers"),
    ("CORTEX_HOME", "/ingestion/home"),
    ("CORTEX_HOOK_FORCE_FALLBACK", "/adapter/hook_force_fallback"),
    (
        "CORTEX_INDEX_LOW_SIGNAL_TOOL_CALLS",
        "/meili/index_low_signal_tool_calls",
    ),
    ("CORTEX_INGESTION_BIND", "/ingestion/bind"),
    ("CORTEX_INGESTION_URL", "/ingestion/ingestion_url"),
    ("CORTEX_MCP_TOOL_TIMEOUT_MS", "/mcp/default_timeout_ms"),
    ("CORTEX_MEILI_REFRESH_SECS", "/dashboard/meili_refresh_secs"),
    ("CORTEX_METADATA_DB", "/ingestion/metadata_db"),
    ("CORTEX_NEXUS_API_KEY", "/nexus/nexus_api_key"),
    ("CORTEX_NEXUS_URL", "/nexus/nexus_url"),
    ("CORTEX_PRE_THINKING_KB", "/pre_thinking/bundle_kb"),
    ("CORTEX_PRE_THINKING_TIMEOUT_MS", "/pre_thinking/timeout_ms"),
    ("CORTEX_PRUNER_STATUS_FILE", "/ingestion/pruner_status_file"),
    ("CORTEX_QUERY_REWRITER", "/dashboard/query_rewriter"),
    (
        "CORTEX_RELEVANCE_CONFIG",
        "/dashboard/relevance_config_path",
    ),
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
    ("CORTEX_REWRITER_MODEL", "/dashboard/rewriter_model"),
    (
        "CORTEX_REWRITER_TIMEOUT_MS",
        "/dashboard/rewriter_timeout_ms",
    ),
    ("CORTEX_RRF_ALPHA", "/dashboard/rrf_alpha"),
    ("CORTEX_RRF_K", "/dashboard/rrf_k"),
    ("CORTEX_RULEBOOK_ROOT", "/rulebook/root"),
    ("CORTEX_RULEBOOK_ROOTS", "/rulebook/roots"),
    ("CORTEX_SYNAP_URL", "/ingestion/synap_url"),
    // Phase18 §3.4 — temporal classifier knobs (sorted alphabetically
    // among `CORTEX_T*` keys; the binary-search assertion in
    // `tests::known_env_names_table_stays_sorted_for_binary_search`
    // pins the ordering).
    ("CORTEX_TEMPORAL_BOOST", "/temporal/temporal_boost"),
    ("CORTEX_TEMPORAL_DEMOTE_FACTOR", "/temporal/demote_factor"),
    ("CORTEX_TEMPORAL_ENABLED", "/temporal/enabled"),
    (
        "CORTEX_TEMPORAL_INCLUDE_HISTORY_DEFAULT",
        "/temporal/include_history_default",
    ),
    (
        "CORTEX_TEMPORAL_WINDOW_DAYS",
        "/temporal/temporal_window_days",
    ),
    ("CORTEX_VECTORIZER_API_KEY", "/embedder/vectorizer_api_key"),
    (
        "CORTEX_VECTORIZER_JWT_WARMUP_SECS",
        "/embedder/jwt_warmup_secs",
    ),
    (
        "CORTEX_VECTORIZER_PASSWORD",
        "/embedder/vectorizer_password",
    ),
    ("CORTEX_VECTORIZER_URL", "/embedder/vectorizer_url"),
    ("CORTEX_VECTORIZER_USER", "/embedder/vectorizer_user"),
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

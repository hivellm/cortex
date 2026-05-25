//! Phase13e §5.2 — per-section TOML round-trip tests.
//!
//! Builds a hermetic `cortex.toml` fixture covering every sub-struct
//! at least once, loads it via `Config::load_from` with an empty env
//! map, and asserts each typed field round-trips byte-for-byte.
//!
//! Pins the contract documented in `docs/specs/00-architecture.md
//! § Configuration` so a future field rename either updates this
//! fixture or fails CI.

use std::collections::HashMap;
use std::path::PathBuf;

use cortex_config::Config;

fn write_fixture(dir: &std::path::Path, toml: &str) -> PathBuf {
    let path = dir.join("cortex.toml");
    std::fs::write(&path, toml).expect("write cortex.toml fixture");
    path
}

#[test]
fn embedder_section_round_trips_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [embedder]
        workers = 8
        chunker_concurrency = 2
        upsert_batch = 128
        max_retry = 5
        vectorizer_url = "http://emb:9001"
        synap_url = "http://emb-synap:9002"
        vectorizer_user = "embuser"
        vectorizer_password = "embpw"
        vectorizer_api_key = "embkey"
        vectorizer_jwt = "embjwt"
        jwt_warmup_secs = 30
        collection_prefix = "embprefix"
        vector_dim = 512
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");
    assert_eq!(cfg.embedder.workers, 8);
    assert_eq!(cfg.embedder.chunker_concurrency, 2);
    assert_eq!(cfg.embedder.upsert_batch, 128);
    assert_eq!(cfg.embedder.max_retry, 5);
    assert_eq!(
        cfg.embedder.vectorizer_url.as_deref(),
        Some("http://emb:9001")
    );
    assert_eq!(cfg.embedder.synap_url, "http://emb-synap:9002");
    assert_eq!(cfg.embedder.vectorizer_user, "embuser");
    assert_eq!(cfg.embedder.vectorizer_password.as_deref(), Some("embpw"));
    assert_eq!(cfg.embedder.vectorizer_api_key.as_deref(), Some("embkey"));
    assert_eq!(cfg.embedder.vectorizer_jwt.as_deref(), Some("embjwt"));
    assert_eq!(cfg.embedder.jwt_warmup_secs, 30);
    assert_eq!(cfg.embedder.collection_prefix, "embprefix");
    assert_eq!(cfg.embedder.vector_dim, 512);
}

#[test]
fn meili_section_round_trips_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [meili]
        meili_url = "http://meili:7700"
        meili_api_key = "mkey"
        synap_url = "http://meili-synap:9003"
        synap_group = "grp-meili"
        index_prefix = "mp-"
        workers = 6
        upsert_batch = 2048
        flush_ms = 750
        max_retry = 4
        await_task = true
        max_body_bytes = 5242880
        replay_missing = true
        index_low_signal_tool_calls = true
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");
    assert_eq!(cfg.meili.meili_url.as_deref(), Some("http://meili:7700"));
    assert_eq!(cfg.meili.meili_api_key.as_deref(), Some("mkey"));
    assert_eq!(cfg.meili.synap_url, "http://meili-synap:9003");
    assert_eq!(cfg.meili.synap_group, "grp-meili");
    assert_eq!(cfg.meili.index_prefix, "mp-");
    assert_eq!(cfg.meili.workers, 6);
    assert_eq!(cfg.meili.upsert_batch, 2048);
    assert_eq!(cfg.meili.flush_ms, 750);
    assert_eq!(cfg.meili.max_retry, 4);
    assert!(cfg.meili.await_task);
    assert_eq!(cfg.meili.max_body_bytes, 5_242_880);
    assert!(cfg.meili.replay_missing);
    assert!(cfg.meili.index_low_signal_tool_calls);
}

#[test]
fn nexus_section_round_trips_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [nexus]
        nexus_url = "http://nexus:7474"
        transport = "rpc"
        synap_url = "http://nexus-synap:9004"
        synap_group = "grp-nexus"
        workers = 6
        patch_batch = 512
        flush_ms = 600
        max_retry = 5
        nexus_user = "nxuser"
        nexus_password = "nxpw"
        nexus_api_key = "nxkey"
        out_of_order_buffer_secs = 60
        metadata_db = "/tmp/nx-meta.sqlite"
        cypher_dir = "/tmp/nx-cypher"
        consumer_id = "nx-consumer-7"
        cypher_enabled = true
        sweeper_interval_secs = 90
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");
    assert_eq!(cfg.nexus.nexus_url.as_deref(), Some("http://nexus:7474"));
    assert_eq!(cfg.nexus.transport, "rpc");
    assert_eq!(cfg.nexus.synap_url, "http://nexus-synap:9004");
    assert_eq!(cfg.nexus.synap_group, "grp-nexus");
    assert_eq!(cfg.nexus.workers, 6);
    assert_eq!(cfg.nexus.patch_batch, 512);
    assert_eq!(cfg.nexus.flush_ms, 600);
    assert_eq!(cfg.nexus.max_retry, 5);
    assert_eq!(cfg.nexus.nexus_user.as_deref(), Some("nxuser"));
    assert_eq!(cfg.nexus.nexus_password.as_deref(), Some("nxpw"));
    assert_eq!(cfg.nexus.nexus_api_key.as_deref(), Some("nxkey"));
    assert_eq!(cfg.nexus.out_of_order_buffer_secs, 60);
    assert_eq!(
        cfg.nexus.metadata_db.as_deref(),
        Some("/tmp/nx-meta.sqlite")
    );
    assert_eq!(cfg.nexus.cypher_dir.as_deref(), Some("/tmp/nx-cypher"));
    assert_eq!(cfg.nexus.consumer_id.as_deref(), Some("nx-consumer-7"));
    assert!(cfg.nexus.cypher_enabled);
    assert_eq!(cfg.nexus.sweeper_interval_secs, Some(90));
}

#[test]
fn ingestion_section_round_trips_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [ingestion]
        bind = "0.0.0.0:17020"
        archive_root = "/srv/cortex/archive"
        synap_url = "http://ingest-synap:9005"
        archive_zstd_level = 5
        metadata_db = "/srv/cortex/metadata.sqlite"
        home = "/srv/cortex"
        ingestion_url = "http://ingest:17010"
        cas_db = "/srv/cortex/cas.sqlite"
        archive_watcher_urls = "http://w1:17030,http://w2:17030"
        pruner_status_file = "/srv/cortex/pruner-status.json"
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");
    assert_eq!(cfg.ingestion.bind, "0.0.0.0:17020");
    assert_eq!(
        cfg.ingestion.archive_root.as_deref(),
        Some("/srv/cortex/archive")
    );
    assert_eq!(
        cfg.ingestion.synap_url.as_deref(),
        Some("http://ingest-synap:9005")
    );
    assert_eq!(cfg.ingestion.archive_zstd_level, 5);
    assert_eq!(
        cfg.ingestion.metadata_db.as_deref(),
        Some("/srv/cortex/metadata.sqlite")
    );
    assert_eq!(cfg.ingestion.home.as_deref(), Some("/srv/cortex"));
    assert_eq!(
        cfg.ingestion.ingestion_url.as_deref(),
        Some("http://ingest:17010")
    );
    assert_eq!(
        cfg.ingestion.cas_db.as_deref(),
        Some("/srv/cortex/cas.sqlite")
    );
    assert_eq!(
        cfg.ingestion.archive_watcher_urls.as_deref(),
        Some("http://w1:17030,http://w2:17030")
    );
    assert_eq!(
        cfg.ingestion.pruner_status_file.as_deref(),
        Some("/srv/cortex/pruner-status.json")
    );
}

#[test]
fn dashboard_section_round_trips_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [dashboard]
        api_bind = "0.0.0.0:17000"
        synap_url = "http://dash-synap:9006"
        archive_refresh_secs = 60
        meili_refresh_secs = 90
        api_keys_db = "/srv/cortex/api_keys.sqlite"
        watch = false
        memory_tail = false
        coverage_slugs = "cortex,nexus"
        coverage_slugs_only = "cortex"
        rrf_alpha = 0.42
        rrf_k = 50
        query_rewriter = "sonnet"
        rewriter_model = "claude-opus-4-6"
        rewriter_timeout_ms = 2500
        relevance_config_path = "/srv/cortex/relevance.toml"
        api_token = "dashtoken"
        api_url = "http://dash:17000"
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");
    assert_eq!(cfg.dashboard.api_bind, "0.0.0.0:17000");
    assert_eq!(cfg.dashboard.archive_refresh_secs, 60);
    assert_eq!(cfg.dashboard.meili_refresh_secs, 90);
    assert!(!cfg.dashboard.watch);
    assert!(!cfg.dashboard.memory_tail);
    assert_eq!(
        cfg.dashboard.coverage_slugs.as_deref(),
        Some("cortex,nexus")
    );
    assert_eq!(cfg.dashboard.coverage_slugs_only.as_deref(), Some("cortex"));
    assert!((cfg.dashboard.rrf_alpha.unwrap() - 0.42).abs() < f32::EPSILON);
    assert_eq!(cfg.dashboard.rrf_k, Some(50));
    assert_eq!(cfg.dashboard.query_rewriter.as_deref(), Some("sonnet"));
    assert_eq!(
        cfg.dashboard.rewriter_model.as_deref(),
        Some("claude-opus-4-6")
    );
    assert_eq!(cfg.dashboard.rewriter_timeout_ms, Some(2500));
    assert_eq!(
        cfg.dashboard.relevance_config_path.as_deref(),
        Some("/srv/cortex/relevance.toml")
    );
    assert_eq!(cfg.dashboard.api_token.as_deref(), Some("dashtoken"));
    assert_eq!(cfg.dashboard.api_url.as_deref(), Some("http://dash:17000"));
}

#[test]
fn small_sections_round_trip_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let toml = r#"
        [retention]
        now_override = "2026-01-01T00:00:00Z"
        fp32_to_pq_days = 14
        pq_to_binary_days = 180
        batch_size = 1024

        [pre_thinking]
        bundle_kb = 32
        timeout_ms = 3000

        [rulebook]
        roots = "/a/.rulebook,/b/.rulebook"
        root = "/legacy/.rulebook"

        [canary]
        enabled = true
        interval_secs = 600
        deadline_secs = 30

        [doctor]
        bench = true

        [classifier]
        health_url = "http://classifier/health"
        staleness_ms = 30000
        max_consume_errors = 25
        synap_url = "http://classifier-synap:9007"
        mode = "claude"
        prompt_version = "v3"
        model = "claude-haiku-4-5"

        [consolidator]
        fallback_file = "/srv/cortex/consolidations.jsonl"
        fallback_rotate_bytes = 1048576
        cursor_file = "/srv/cortex/consolidator-cursor"

        [auto_memory]
        project = "e--HiveLLM-Cortex"

        [analyzer]
        bin = "claude-cli"
        model = "claude-sonnet-4-7"
        api_key = "analyzer-key"
        api_base = "https://anthropic.example.com"

        [claude_archive]
        bind = "0.0.0.0:17030"
        poll_ms = 250
        root = "/srv/claude/projects"

        [adapter]
        hook_force_fallback = true
        adapter_disable = true
        adapter_pipe = "\\\\.\\pipe\\custom"
        adapter_sock = "/srv/custom.sock"
        adapter_admin_port = 17091
    "#;
    let path = write_fixture(dir.path(), toml);
    let cfg = Config::load_from(&path, |_| None).expect("load");

    assert_eq!(
        cfg.retention.now_override.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(cfg.retention.fp32_to_pq_days, 14);
    assert_eq!(cfg.retention.pq_to_binary_days, 180);
    assert_eq!(cfg.retention.batch_size, 1024);

    assert_eq!(cfg.pre_thinking.bundle_kb, 32);
    assert_eq!(cfg.pre_thinking.timeout_ms, 3000);

    assert_eq!(
        cfg.rulebook.roots.as_deref(),
        Some("/a/.rulebook,/b/.rulebook")
    );
    assert_eq!(cfg.rulebook.root.as_deref(), Some("/legacy/.rulebook"));

    assert!(cfg.canary.enabled);
    assert_eq!(cfg.canary.interval_secs, 600);
    assert_eq!(cfg.canary.deadline_secs, 30);

    assert!(cfg.doctor.bench);

    assert_eq!(
        cfg.classifier.health_url.as_deref(),
        Some("http://classifier/health")
    );
    assert_eq!(cfg.classifier.staleness_ms, Some(30000));
    assert_eq!(cfg.classifier.max_consume_errors, Some(25));
    assert_eq!(
        cfg.classifier.synap_url.as_deref(),
        Some("http://classifier-synap:9007")
    );
    assert_eq!(cfg.classifier.mode.as_deref(), Some("claude"));
    assert_eq!(cfg.classifier.prompt_version.as_deref(), Some("v3"));
    assert_eq!(cfg.classifier.model.as_deref(), Some("claude-haiku-4-5"));

    assert_eq!(
        cfg.consolidator.fallback_file.as_deref(),
        Some("/srv/cortex/consolidations.jsonl")
    );
    assert_eq!(cfg.consolidator.fallback_rotate_bytes, Some(1_048_576));
    assert_eq!(
        cfg.consolidator.cursor_file.as_deref(),
        Some("/srv/cortex/consolidator-cursor")
    );

    assert_eq!(
        cfg.auto_memory.project.as_deref(),
        Some("e--HiveLLM-Cortex")
    );

    assert_eq!(cfg.analyzer.bin.as_deref(), Some("claude-cli"));
    assert_eq!(cfg.analyzer.model.as_deref(), Some("claude-sonnet-4-7"));
    assert_eq!(cfg.analyzer.api_key.as_deref(), Some("analyzer-key"));
    assert_eq!(
        cfg.analyzer.api_base.as_deref(),
        Some("https://anthropic.example.com")
    );

    assert_eq!(cfg.claude_archive.bind.as_deref(), Some("0.0.0.0:17030"));
    assert_eq!(cfg.claude_archive.poll_ms, Some(250));
    assert_eq!(
        cfg.claude_archive.root.as_deref(),
        Some("/srv/claude/projects")
    );

    assert!(cfg.adapter.hook_force_fallback);
    assert!(cfg.adapter.adapter_disable);
    assert_eq!(
        cfg.adapter.adapter_pipe.as_deref(),
        Some(r"\\.\pipe\custom")
    );
    assert_eq!(
        cfg.adapter.adapter_sock.as_deref(),
        Some("/srv/custom.sock")
    );
    assert_eq!(cfg.adapter.adapter_admin_port, Some(17091));
}

#[test]
fn env_overlay_round_trips_a_knob_from_each_section() {
    // One representative env knob per sub-struct — confirms the
    // env_overlay walks the full KNOWN_ENV_NAMES table.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.toml");
    let env: HashMap<&'static str, &'static str> = HashMap::from([
        ("CORTEX_RETENTION_BATCH_SIZE", "4096"),
        ("CORTEX_EMBEDDER_WORKERS", "12"),
        ("CORTEX_FULLTEXT_BATCH", "9999"),
        ("CORTEX_GRAPH_WORKERS", "9"),
        ("CORTEX_INGESTION_BIND", "0.0.0.0:17099"),
        ("CORTEX_API_TOKEN", "envtoken"),
        ("CORTEX_PRE_THINKING_KB", "64"),
        ("CORTEX_RULEBOOK_ROOTS", "/env/.rulebook"),
        ("CORTEX_CANARY_INTERVAL_SECS", "777"),
        ("CORTEX_DOCTOR_BENCH", "true"),
        ("CORTEX_CLASSIFIER_MODEL", "env-classifier-model"),
        ("CORTEX_CONSOLIDATOR_CURSOR_FILE", "/env/cursor"),
        ("CORTEX_AUTO_MEMORY_PROJECT", "env-project"),
        ("CORTEX_ANALYZER_MODEL", "env-analyzer-model"),
        ("CORTEX_CLAUDE_ARCHIVE_POLL_MS", "321"),
        ("CORTEX_ADAPTER_ADMIN_PORT", "17092"),
    ]);
    let env_lookup = move |k: &str| env.get(k).map(|v| v.to_string());
    let cfg = Config::load_from(&path, env_lookup).expect("load");

    assert_eq!(cfg.retention.batch_size, 4096);
    assert_eq!(cfg.embedder.workers, 12);
    assert_eq!(cfg.meili.upsert_batch, 9999);
    assert_eq!(cfg.nexus.workers, 9);
    assert_eq!(cfg.ingestion.bind, "0.0.0.0:17099");
    assert_eq!(cfg.dashboard.api_token.as_deref(), Some("envtoken"));
    assert_eq!(cfg.pre_thinking.bundle_kb, 64);
    assert_eq!(cfg.rulebook.roots.as_deref(), Some("/env/.rulebook"));
    assert_eq!(cfg.canary.interval_secs, 777);
    assert!(cfg.doctor.bench);
    assert_eq!(
        cfg.classifier.model.as_deref(),
        Some("env-classifier-model")
    );
    assert_eq!(cfg.consolidator.cursor_file.as_deref(), Some("/env/cursor"));
    assert_eq!(cfg.auto_memory.project.as_deref(), Some("env-project"));
    assert_eq!(cfg.analyzer.model.as_deref(), Some("env-analyzer-model"));
    assert_eq!(cfg.claude_archive.poll_ms, Some(321));
    assert_eq!(cfg.adapter.adapter_admin_port, Some(17092));
}

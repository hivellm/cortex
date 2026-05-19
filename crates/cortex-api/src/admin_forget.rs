//! Phase11t §1 — admin `/v1/admin/forget` endpoint.
//!
//! Wraps `cortex_workers::pruner::purge::forget` behind an HTTP
//! surface so the MCP `cortex_forget` tool (and operator-side
//! `curl`) can drive the irreversible cascade across Vectorizer +
//! Meili + Nexus + Parquet.
//!
//! Architecture: `cortex-mcp-server` is an MCP-protocol shim that
//! proxies to `cortex-api`; it has no direct handle on the backend
//! clients. The cascade lives here (where Vectorizer + Meili +
//! Nexus clients already exist for the query lanes), and the MCP
//! tool simply POSTs to this route.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_storage::archive::scan_envelope_by_event_id;
use cortex_workers::pruner::purge::{
    ArchivePurgeError, ArchivePurgeOps, NexusPurgeError, NexusPurgeOps, PurgeError, PurgeReport,
    REQUIRED_CONFIRMATION_TOKEN,
};
use nexus_sdk::NexusClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request body for `POST /v1/admin/forget`.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgetRequest {
    /// Globally unique event id the operator wants forgotten.
    pub event_id: String,
    /// Must match [`REQUIRED_CONFIRMATION_TOKEN`] verbatim. Any
    /// other value (or missing field) returns HTTP 400.
    pub confirmation_token: String,
    /// When `true`, the handler resolves the target collections +
    /// returns the projection without invoking the cascade.
    #[serde(default)]
    pub dry_run: bool,
}

/// Response shape for both real and dry-run invocations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForgetResponse {
    /// Echoed back so the caller can correlate.
    pub event_id: String,
    /// Vectorizer collections the cascade probed (or would probe
    /// on a dry run).
    pub vectorizer_collections: Vec<String>,
    /// `true` when the request carried `dry_run = true`.
    pub dry_run: bool,
    /// Vectors removed across every collection. `0` on dry-run.
    #[serde(default)]
    pub vectors_removed: u64,
    /// `true` when the Meili row was deleted. `false` on dry-run.
    #[serde(default)]
    pub meili_deleted: bool,
    /// `true` when the Nexus node was deleted. `false` on dry-run.
    #[serde(default)]
    pub nexus_deleted: bool,
    /// `true` when the Parquet partition was rewritten. `false` on
    /// dry-run.
    #[serde(default)]
    pub archive_rewritten: bool,
}

/// Phase11t §1.1 — resolve the Vectorizer collections the cascade
/// should probe for an event of the given `kind`. Unknown kinds
/// fall through to the consolidation triple (safe over-purge: an
/// extra `delete_vectors` call is idempotent / cheap, while a
/// missed collection would leave the row alive).
pub fn resolve_target_collections(kind: Option<Kind>) -> Vec<&'static str> {
    use cortex_storage::names::{
        COLLECTION_CODE_CHUNK_FP32, COLLECTION_CODE_CHUNK_PQ, COLLECTION_COLD_BINARY,
        COLLECTION_CONSOLIDATION_FP32, COLLECTION_CONSOLIDATION_PQ, COLLECTION_TOOL_CALL_FP32,
        COLLECTION_TOOL_CALL_PQ, COLLECTION_TURN_FP32, COLLECTION_TURN_PQ,
    };
    match kind {
        Some(Kind::Consolidation) | None => vec![
            COLLECTION_CONSOLIDATION_FP32,
            COLLECTION_CONSOLIDATION_PQ,
            COLLECTION_COLD_BINARY,
        ],
        Some(Kind::Turn) => vec![
            COLLECTION_TURN_FP32,
            COLLECTION_TURN_PQ,
            COLLECTION_COLD_BINARY,
        ],
        Some(Kind::ToolCall) => vec![
            COLLECTION_TOOL_CALL_FP32,
            COLLECTION_TOOL_CALL_PQ,
            COLLECTION_COLD_BINARY,
        ],
        Some(Kind::Artifact) => vec![
            COLLECTION_CODE_CHUNK_FP32,
            COLLECTION_CODE_CHUNK_PQ,
            COLLECTION_COLD_BINARY,
        ],
        // Other kinds (Decision, Analysis, Memory, Law, Knowledge,
        // Learning, etc.) only land in their per-kind hot
        // collection. The cold tier is shared so probe both.
        Some(_) => vec![COLLECTION_CONSOLIDATION_FP32, COLLECTION_COLD_BINARY],
    }
}

// ----------------------------------------------------------------------
// Phase11t §1.2 — live `NexusPurgeOps` adapter.
// ----------------------------------------------------------------------

/// Live adapter wrapping a `NexusClient` for the purge cascade.
pub struct LiveNexusPurger {
    client: Arc<NexusClient>,
}

impl LiveNexusPurger {
    /// Build a new purger from a shared client handle.
    pub fn new(client: Arc<NexusClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl NexusPurgeOps for LiveNexusPurger {
    async fn delete_node_by_event_id(&self, event_id: &str) -> Result<(), NexusPurgeError> {
        // Single Cypher pass: every Cortex-owned node carries
        // `event_id` as a property (set by the graph mapper). A
        // zero-row delete is the idempotent path — Nexus returns
        // success without raising an error.
        let mut params: std::collections::HashMap<String, nexus_sdk::Value> =
            std::collections::HashMap::new();
        params.insert(
            "id".to_string(),
            nexus_sdk::Value::String(event_id.to_string()),
        );
        self.client
            .execute_cypher("MATCH (n { event_id: $id }) DETACH DELETE n", Some(params))
            .await
            .map_err(|e| NexusPurgeError::Server(e.to_string()))?;
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Phase11t §1.3 — live `ArchivePurgeOps` adapter.
// ----------------------------------------------------------------------

/// Live adapter that walks the parquet hierarchy + rewrites every
/// partition that contains the matching `event_id` line.
pub struct LiveArchivePurger {
    archive_root: PathBuf,
}

impl LiveArchivePurger {
    /// Build a new purger from the configured archive root.
    pub fn new(archive_root: impl Into<PathBuf>) -> Self {
        Self {
            archive_root: archive_root.into(),
        }
    }
}

#[async_trait]
impl ArchivePurgeOps for LiveArchivePurger {
    async fn drop_event(&self, event_id: &str) -> Result<(), ArchivePurgeError> {
        // 1. Walk the archive once collecting the parquet files
        //    that contain the matching id.
        let target_paths = collect_target_files(&self.archive_root, event_id)
            .map_err(|e| ArchivePurgeError::Io(e.to_string()))?;
        if target_paths.is_empty() {
            // Idempotent — missing-id is not an error.
            return Ok(());
        }
        // 2. Rewrite each partition without the matching line via
        //    a `*.parquet.tmp` + atomic rename. A mid-write crash
        //    leaves the original intact.
        for path in target_paths {
            rewrite_partition(&path, event_id)
                .map_err(|e| ArchivePurgeError::Io(format!("rewrite {path:?}: {e}")))?;
        }
        Ok(())
    }
}

fn collect_target_files(
    archive_root: &std::path::Path,
    event_id: &str,
) -> std::io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk_dir_for_target(archive_root, event_id, &mut out)?;
    Ok(out)
}

fn walk_dir_for_target(
    dir: &std::path::Path,
    event_id: &str,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir_for_target(&path, event_id, out)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        if file_contains_event_id(&path, event_id)? {
            out.push(path);
        }
    }
    Ok(())
}

// Phase12c §2 — the partial-frame predicate now lives in
// `cortex_storage::archive_purge::is_live_partial_frame`. Re-imported
// here so the call-site shape stays the same and a single source of
// truth catches drift across every purger / walker that has to
// tolerate the live current-hour file.
use cortex_storage::is_live_partial_frame;

fn file_contains_event_id(path: &std::path::Path, event_id: &str) -> std::io::Result<bool> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)?;
    let decoder = match zstd::stream::read::Decoder::new(file) {
        Ok(d) => d,
        Err(_) => return Ok(false),
    };
    let reader = std::io::BufReader::new(decoder);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            // Live current-hour file with a half-flushed frame at
            // the tail — treat as end-of-stream and use whatever we
            // already inspected. Aborting here would skip the
            // file entirely and leave the target id stranded across
            // every subsequent purge run.
            Err(e) if is_live_partial_frame(&e) => return Ok(false),
            Err(e) => return Err(e),
        };
        if line.contains(event_id) {
            // String-level pre-filter so we don't decode every
            // line; a confirmed match parses next.
            if let Ok(env) = serde_json::from_str::<cortex_core::events::Envelope>(line.trim()) {
                if env.event_id == event_id {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn rewrite_partition(path: &std::path::Path, event_id: &str) -> std::io::Result<()> {
    use std::io::{BufRead, Write};
    let file = std::fs::File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let reader = std::io::BufReader::new(decoder);

    let tmp = path.with_extension("parquet.tmp");
    let out_file = std::fs::File::create(&tmp)?;
    let mut enc = zstd::stream::write::Encoder::new(out_file, 6)?;
    let mut hit_partial_frame = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            // Live current-hour file with a half-flushed frame —
            // abort the rewrite so the original stays intact. The
            // target event_id may be on either side of the broken
            // frame; rewriting through the partial-read tail would
            // truncate every line the loader has appended since we
            // started reading. Subsequent purge attempts retry
            // after the file rotates (next hour) and the frame is
            // sealed.
            Err(e) if is_live_partial_frame(&e) => {
                hit_partial_frame = true;
                break;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip lines matching the target event_id.
        if let Ok(env) = serde_json::from_str::<cortex_core::events::Envelope>(trimmed) {
            if env.event_id == event_id {
                continue;
            }
        }
        enc.write_all(line.as_bytes())?;
        enc.write_all(b"\n")?;
    }
    if hit_partial_frame {
        // Don't promote the partial rewrite over the live original.
        let _ = enc.finish();
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(
            path = %path.display(),
            "admin_forget: skipped rewrite — current-hour zstd frame still being written; \
             retry next sweep after the partition rotates"
        );
        return Ok(());
    }
    enc.finish()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ----------------------------------------------------------------------
// Phase11t §1.4 — handler entrypoint (called from http.rs).
// ----------------------------------------------------------------------

/// Errors the handler maps onto HTTP responses.
#[derive(Debug, thiserror::Error)]
pub enum ForgetError {
    /// The caller's confirmation token did not match the canonical
    /// constant — surfaces as HTTP 400.
    #[error("missing or wrong confirmation_token (expected {0:?})")]
    BadToken(&'static str),
    /// Cascade leg failed — surfaces as HTTP 500.
    #[error("cascade: {0}")]
    Cascade(#[from] PurgeError),
}

/// Handle one `/v1/admin/forget` invocation. The caller (`http.rs`)
/// is responsible for instantiating the live ops + threading them
/// in; this fn coordinates the cascade.
#[allow(clippy::too_many_arguments)]
pub async fn handle_forget<V, M>(
    req: &ForgetRequest,
    archive_root: &std::path::Path,
    vectorizer: &V,
    meili: &M,
    nexus: &dyn NexusPurgeOps,
    archive: &dyn ArchivePurgeOps,
    meili_index: &str,
) -> Result<ForgetResponse, ForgetError>
where
    V: cortex_workers::embedder::vectorizer_client::VectorizerClient,
    M: cortex_workers::pruner::meili_sink::MeiliPruneOps,
{
    if req.confirmation_token != REQUIRED_CONFIRMATION_TOKEN {
        return Err(ForgetError::BadToken(REQUIRED_CONFIRMATION_TOKEN));
    }

    // Resolve kind from the archive (if the envelope still exists)
    // so the target-collection list matches the event's family.
    let kind = scan_envelope_by_event_id(archive_root, &req.event_id)
        .ok()
        .flatten()
        .map(|env| env.kind);
    let target_collections = resolve_target_collections(kind);

    if req.dry_run {
        return Ok(ForgetResponse {
            event_id: req.event_id.clone(),
            vectorizer_collections: target_collections.iter().map(|s| s.to_string()).collect(),
            dry_run: true,
            vectors_removed: 0,
            meili_deleted: false,
            nexus_deleted: false,
            archive_rewritten: false,
        });
    }

    let report: PurgeReport = cortex_workers::pruner::purge::forget(
        &req.event_id,
        &target_collections,
        vectorizer,
        meili,
        meili_index,
        nexus,
        archive,
        REQUIRED_CONFIRMATION_TOKEN,
    )
    .await?;

    Ok(ForgetResponse {
        event_id: req.event_id.clone(),
        vectorizer_collections: target_collections.iter().map(|s| s.to_string()).collect(),
        dry_run: false,
        vectors_removed: report.vectors_removed,
        meili_deleted: report.meili_deleted,
        nexus_deleted: report.nexus_deleted,
        archive_rewritten: report.archive_rewritten,
    })
}

/// Convert a `ForgetError` to a `(status, json)` pair the handler
/// wraps into an axum response.
pub fn forget_error_response(err: &ForgetError) -> (u16, Value) {
    match err {
        ForgetError::BadToken(_) => (
            400,
            serde_json::json!({
                "reason": "missing_confirmation_token",
                "message": err.to_string(),
            }),
        ),
        ForgetError::Cascade(_) => (
            500,
            serde_json::json!({
                "reason": "purge_cascade_failed",
                "message": err.to_string(),
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    // ------------------- §1.1 resolve_target_collections -------------------

    #[test]
    fn resolve_consolidation_returns_three_consolidation_collections() {
        let got = resolve_target_collections(Some(Kind::Consolidation));
        assert_eq!(got.len(), 3);
        assert!(got.contains(&"cortex.consolidation.fp32"));
        assert!(got.contains(&"cortex.consolidation.pq"));
        assert!(got.contains(&"cortex.cold.binary"));
    }

    #[test]
    fn resolve_turn_returns_per_kind_pair_plus_cold() {
        let got = resolve_target_collections(Some(Kind::Turn));
        assert!(got.contains(&"cortex.turn.fp32"));
        assert!(got.contains(&"cortex.turn.pq"));
        assert!(got.contains(&"cortex.cold.binary"));
    }

    #[test]
    fn resolve_tool_call_returns_per_kind_pair_plus_cold() {
        let got = resolve_target_collections(Some(Kind::ToolCall));
        assert!(got.contains(&"cortex.tool_call.fp32"));
        assert!(got.contains(&"cortex.tool_call.pq"));
        assert!(got.contains(&"cortex.cold.binary"));
    }

    #[test]
    fn resolve_unknown_kind_falls_through_to_consolidation_triple() {
        // `None` (envelope not in archive) is the safe over-purge
        // path — every consolidation collection probed.
        let got = resolve_target_collections(None);
        assert_eq!(got.len(), 3);
        assert!(got.contains(&"cortex.consolidation.fp32"));
    }

    // ------------------- §1.2 / §1.3 mock-driven cascade -------------------

    #[derive(Default)]
    struct StubNexus {
        deleted: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl NexusPurgeOps for StubNexus {
        async fn delete_node_by_event_id(&self, event_id: &str) -> Result<(), NexusPurgeError> {
            self.deleted.lock().unwrap().push(event_id.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct StubArchive {
        dropped: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl ArchivePurgeOps for StubArchive {
        async fn drop_event(&self, event_id: &str) -> Result<(), ArchivePurgeError> {
            self.dropped.lock().unwrap().push(event_id.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct StubMeili {
        deletes: Mutex<Vec<(String, Vec<String>)>>,
    }
    #[async_trait]
    impl cortex_workers::pruner::meili_sink::MeiliPruneOps for StubMeili {
        async fn update_documents(
            &self,
            _index: &str,
            _docs: &[Value],
        ) -> Result<(), cortex_workers::pruner::meili_sink::MeiliPruneError> {
            Ok(())
        }
        async fn delete_documents(
            &self,
            index: &str,
            ids: &[String],
        ) -> Result<(), cortex_workers::pruner::meili_sink::MeiliPruneError> {
            self.deletes
                .lock()
                .unwrap()
                .push((index.to_string(), ids.to_vec()));
            Ok(())
        }
    }

    fn write_archive(root: &std::path::Path, envelopes: &[cortex_core::events::Envelope]) {
        use std::io::Write;
        let dir = root.join("events/year=2026/month=04/day=26/hour=19");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw-00000.parquet");
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
        for env in envelopes {
            let line = serde_json::to_string(env).unwrap();
            enc.write_all(line.as_bytes()).unwrap();
            enc.write_all(b"\n").unwrap();
        }
        enc.finish().unwrap();
    }

    fn turn_envelope(event_id: &str) -> cortex_core::events::Envelope {
        cortex_core::events::Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: "2026-04-26T19:00:00Z".to_string(),
            ingested_at: None,
            session_id: "S1".to_string(),
            stream: cortex_core::events::Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: cortex_core::events::Context {
                repo: Some("cortex".to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".to_string(),
                ide: None,
                extras: BTreeMap::new(),
            },
            payload: serde_json::to_value(cortex_core::events::Turn {
                user_message: "x".to_string(),
                assistant_message: None,
                tokens: None,
                tool_call_event_ids: vec![],
            })
            .unwrap(),
            redactions: vec![],
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            parent_event_id: None,
        }
    }

    #[tokio::test]
    async fn live_archive_purger_rewrites_partition_without_event_id() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(dir.path(), &[turn_envelope("KEEP"), turn_envelope("DROP")]);
        let purger = LiveArchivePurger::new(dir.path());
        purger.drop_event("DROP").await.unwrap();

        // Re-read the partition: only KEEP survives.
        let after = scan_envelope_by_event_id(dir.path(), "KEEP")
            .unwrap()
            .expect("KEEP must survive");
        assert_eq!(after.event_id, "KEEP");
        let dropped = scan_envelope_by_event_id(dir.path(), "DROP").unwrap();
        assert!(dropped.is_none(), "DROP must be gone");
    }

    #[tokio::test]
    async fn live_archive_purger_miss_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(dir.path(), &[turn_envelope("KEEP")]);
        let purger = LiveArchivePurger::new(dir.path());
        purger.drop_event("NEVER_THERE").await.unwrap();
        let still = scan_envelope_by_event_id(dir.path(), "KEEP")
            .unwrap()
            .expect("KEEP must survive");
        assert_eq!(still.event_id, "KEEP");
    }

    // ------------------- §1.4 handle_forget happy / dry / token -------------------

    #[tokio::test]
    async fn handle_forget_dry_run_returns_projection_without_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(dir.path(), &[turn_envelope("EVT")]);
        let nexus = StubNexus::default();
        let archive = StubArchive::default();
        let meili = StubMeili::default();
        let vec_client =
            cortex_workers::embedder::vectorizer_client::MemoryVectorizerClient::default();

        let req = ForgetRequest {
            event_id: "EVT".into(),
            confirmation_token: REQUIRED_CONFIRMATION_TOKEN.into(),
            dry_run: true,
        };
        let resp = handle_forget(
            &req,
            dir.path(),
            &vec_client,
            &meili,
            &nexus,
            &archive,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        .unwrap();
        assert!(resp.dry_run);
        assert!(!resp.meili_deleted);
        assert!(!resp.nexus_deleted);
        assert!(!resp.archive_rewritten);
        // Resolved as Turn since the envelope is in the archive.
        assert!(resp
            .vectorizer_collections
            .iter()
            .any(|c| c == "cortex.turn.fp32"));
        assert!(nexus.deleted.lock().unwrap().is_empty());
        assert!(archive.dropped.lock().unwrap().is_empty());
        assert!(meili.deletes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_forget_rejects_missing_confirmation_token() {
        let dir = tempfile::tempdir().unwrap();
        let nexus = StubNexus::default();
        let archive = StubArchive::default();
        let meili = StubMeili::default();
        let vec_client =
            cortex_workers::embedder::vectorizer_client::MemoryVectorizerClient::default();
        let req = ForgetRequest {
            event_id: "EVT".into(),
            confirmation_token: "wrong".into(),
            dry_run: false,
        };
        let err = handle_forget(
            &req,
            dir.path(),
            &vec_client,
            &meili,
            &nexus,
            &archive,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        .unwrap_err();
        match err {
            ForgetError::BadToken(_) => {}
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_forget_happy_path_runs_full_cascade() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(dir.path(), &[turn_envelope("EVT")]);
        let nexus = StubNexus::default();
        let archive = StubArchive::default();
        let meili = StubMeili::default();
        let vec_client =
            cortex_workers::embedder::vectorizer_client::MemoryVectorizerClient::default();
        let req = ForgetRequest {
            event_id: "EVT".into(),
            confirmation_token: REQUIRED_CONFIRMATION_TOKEN.into(),
            dry_run: false,
        };
        let resp = handle_forget(
            &req,
            dir.path(),
            &vec_client,
            &meili,
            &nexus,
            &archive,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        .unwrap();
        assert!(!resp.dry_run);
        assert!(resp.meili_deleted);
        assert!(resp.nexus_deleted);
        assert!(resp.archive_rewritten);
        assert_eq!(*nexus.deleted.lock().unwrap(), vec!["EVT".to_string()]);
        assert_eq!(*archive.dropped.lock().unwrap(), vec!["EVT".to_string()]);
        let meili_calls = meili.deletes.lock().unwrap();
        assert_eq!(meili_calls.len(), 1);
        assert_eq!(meili_calls[0].1, vec!["EVT".to_string()]);
    }
}

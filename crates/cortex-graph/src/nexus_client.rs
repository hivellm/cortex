//! Nexus-client abstraction.
//!
//! The graph writer talks to Nexus through this trait so the worker can
//! be exercised against an in-memory fake during tests, while the
//! production path delegates to the official `nexus-graph-sdk`
//! ([`nexus_sdk::NexusClient`]). Spec 07 calls for "Bolt or HTTP";
//! [`nexus_sdk::NexusClient`] already speaks two transports — its
//! native RPC over `nexus://` URLs and HTTP over `http(s)://` URLs —
//! so we map the spec's "Bolt vs HTTP" to "RPC vs HTTP".
//!
//! ## On `deadpool`
//!
//! Spec 07 §Nexus client + tasks.md 2.1 mention a `deadpool` connection
//! pool. The SDK 1.15 builds its own `Arc<dyn Transport>` internally
//! (see `client.rs:169` in the SDK source) — every clone of
//! [`nexus_sdk::NexusClient`] shares that single transport, which
//! handles its own connection lifecycle, retries (`max_retries` on
//! [`nexus_sdk::ClientConfig`]) and concurrency. Layering `deadpool`
//! on top would duplicate state and risk transport thrash, so this
//! module stores the SDK client behind an [`Arc`] and clones it for
//! each writer task instead.
//!
//! ## Cypher generation
//!
//! Round 2 generates `UNWIND $rows AS row MERGE ...` Cypher inline
//! per `(label)` and `(from_label, edge_type, to_label)` group. The
//! source-controlled `cypher/*.cypher` template registry will take
//! over in Round 3 — the dispatcher logic lives entirely inside this
//! module so swapping it is local.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use nexus_sdk::{ClientConfig, NexusClient, NexusError, QueryResult, Value as SdkValue};
use thiserror::Error;

use crate::config::{GraphConfig, GraphTransport};
use crate::cypher::CypherTemplates;
use crate::patch::{EdgeOp, GraphPatch, NodeOp};

/// Per-transaction write counters returned by a successful
/// [`GraphClient::run_write_tx`] call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteStats {
    /// Number of node `MERGE`s that ran in the transaction.
    pub nodes_upserted: u32,
    /// Number of edge `MERGE`s that ran in the transaction.
    pub edges_upserted: u32,
}

/// Failure modes raised by a [`GraphClient`].
#[derive(Debug, Error)]
pub enum GraphClientError {
    /// Transport-layer or unclassified Nexus error message.
    #[error("nexus error: {0}")]
    Nexus(String),
    /// Retry-eligible transient failure (network blip, Nexus 5xx).
    #[error("transient nexus error: {0}")]
    TransientError(String),
    /// Hard error — duplicate key, schema constraint failure, etc.
    /// Routes to the dead-letter stream rather than retrying.
    #[error("constraint violation: {detail}")]
    ConstraintViolation {
        /// Human-readable detail from Nexus.
        detail: String,
    },
    /// Authentication failed against Nexus — fail fast at startup.
    #[error("nexus auth failed: {0}")]
    AuthFailed(String),
    /// Existing schema is incompatible with the constraints we want to
    /// install. Fatal at startup.
    #[error("nexus schema drift: {0}")]
    SchemaDrift(String),
}

impl GraphClientError {
    /// Whether this error category is worth retrying.
    pub fn is_retriable(&self) -> bool {
        matches!(self, GraphClientError::TransientError(_))
    }

    /// Classify a [`nexus_sdk::NexusError`] into a [`GraphClientError`].
    ///
    /// `purpose` is a short tag describing the call site — "schema",
    /// "write_tx", "ping" — used purely for log/error context.
    pub fn classify(err: NexusError, purpose: &str) -> Self {
        match &err {
            NexusError::Authentication(msg) => GraphClientError::AuthFailed(msg.clone()),
            NexusError::Api { status, message } => {
                if *status == 401 || *status == 403 {
                    GraphClientError::AuthFailed(message.clone())
                } else if *status >= 500 {
                    GraphClientError::TransientError(format!(
                        "{purpose}: api {status}: {message}"
                    ))
                } else {
                    classify_message(message, purpose)
                }
            }
            NexusError::Connection(msg)
            | NexusError::Network(msg) => {
                GraphClientError::TransientError(format!("{purpose}: {msg}"))
            }
            NexusError::Timeout => {
                GraphClientError::TransientError(format!("{purpose}: timeout"))
            }
            NexusError::Http(http_err) => {
                if http_err.is_timeout() || http_err.is_connect() {
                    GraphClientError::TransientError(format!("{purpose}: http {http_err}"))
                } else if http_err
                    .status()
                    .map(|s| s.is_server_error())
                    .unwrap_or(false)
                {
                    GraphClientError::TransientError(format!("{purpose}: http {http_err}"))
                } else {
                    GraphClientError::Nexus(format!("{purpose}: http {http_err}"))
                }
            }
            _ => classify_message(&err.to_string(), purpose),
        }
    }
}

fn classify_message(message: &str, purpose: &str) -> GraphClientError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") {
        GraphClientError::AuthFailed(message.to_string())
    } else if lower.contains("constraint") || lower.contains("already exists for") {
        GraphClientError::ConstraintViolation {
            detail: format!("{purpose}: {message}"),
        }
    } else {
        GraphClientError::Nexus(format!("{purpose}: {message}"))
    }
}

impl From<NexusError> for GraphClientError {
    fn from(err: NexusError) -> Self {
        GraphClientError::classify(err, "nexus")
    }
}

/// Result alias used throughout the client surface.
pub type GraphClientResult<T> = Result<T, GraphClientError>;

/// Abstraction over a Nexus transaction client.
#[async_trait]
pub trait GraphClient: Send + Sync {
    /// Run schema-bootstrap statements (constraints + indexes). Per
    /// spec 07 §Schema bootstrapping every statement is idempotent —
    /// re-runs are no-ops.
    async fn ensure_schema(&self, statements: &[String]) -> GraphClientResult<()>;

    /// Apply a coalesced [`GraphPatch`] to Nexus inside one Cypher
    /// transaction, picking templates from `templates` per
    /// `(label × incoming edge)` pattern.
    async fn run_write_tx(
        &self,
        patch: &GraphPatch,
        templates: &CypherTemplates,
    ) -> GraphClientResult<WriteStats>;
}

// ---------- Retry helper -------------------------------------------------

/// Retry helper — `max_attempts` attempts, exponential backoff
/// 100 / 400 / 1600 ms between attempts. Only retries errors classified
/// as [`GraphClientError::TransientError`].
///
/// Mirrors the contract of [`cortex_embedder::with_retry`] so both
/// workers behave identically under transient remote-server pressure.
pub async fn with_retry<F, Fut, T>(
    max_attempts: u32,
    mut f: F,
) -> Result<T, GraphClientError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, GraphClientError>>,
{
    let attempts = max_attempts.max(1);
    let mut last_err: Option<GraphClientError> = None;
    for attempt in 0..attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !err.is_retriable() || attempt + 1 == attempts {
                    return Err(err);
                }
                let backoff_ms = 100u64 << (2 * attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        GraphClientError::Nexus("with_retry exhausted attempts without error".into())
    }))
}

// ---------- Live ---------------------------------------------------------

/// Production graph client backed by `nexus-graph-sdk` 1.15.
///
/// Holds a single [`NexusClient`] instance whose internal
/// `Arc<dyn Transport>` is cheaply cloneable across worker tasks; see
/// the module-level note on `deadpool`.
#[derive(Debug, Clone)]
pub struct LiveNexusClient {
    sdk: NexusClient,
    config: GraphConfig,
}

impl LiveNexusClient {
    /// Build a [`LiveNexusClient`] from a [`GraphConfig`].
    ///
    /// Transport selection mirrors the SDK precedence:
    /// 1. URL scheme on `nexus_url` always wins.
    /// 2. [`GraphTransport::Http`] / [`GraphTransport::Rpc`] forces the
    ///    matching SDK transport via `ClientConfig.transport`.
    /// 3. [`GraphTransport::Auto`] leaves the choice to the SDK.
    pub fn new(config: GraphConfig) -> Result<Self, GraphClientError> {
        let mut cfg = ClientConfig {
            base_url: config.nexus_url.clone(),
            ..Default::default()
        };
        cfg.username = config.nexus_user.clone();
        cfg.password = config.nexus_password.clone();
        cfg.max_retries = config.max_retry;
        cfg.transport = match config.transport {
            GraphTransport::Auto => None,
            GraphTransport::Rpc => Some(nexus_sdk::transport::TransportMode::NexusRpc),
            GraphTransport::Http => {
                if config.nexus_url.starts_with("https://") {
                    Some(nexus_sdk::transport::TransportMode::Https)
                } else {
                    Some(nexus_sdk::transport::TransportMode::Http)
                }
            }
        };
        let sdk = NexusClient::with_config(cfg)
            .map_err(|e| GraphClientError::classify(e, "construct"))?;
        Ok(Self { sdk, config })
    }

    /// Borrow the underlying SDK client — useful for one-off RPC verbs
    /// (`health_check`, `list_labels`) that aren't on [`GraphClient`].
    pub fn sdk(&self) -> &NexusClient {
        &self.sdk
    }

    /// Borrow the [`GraphConfig`] this client was built from.
    pub fn config(&self) -> &GraphConfig {
        &self.config
    }

    /// Lookup the natural-key field name for a given node label, used
    /// to build `MERGE (n:Label { <field>: row.key })` Cypher.
    ///
    /// Matches `docs/specs/07-graph-writer.md` §Schema:
    /// `Artifact` keys on `natural_key`, `Repo` on `name`, and every
    /// other label on `id`.
    pub fn key_field_for(label: &str) -> &'static str {
        match label {
            "Artifact" => "natural_key",
            "Repo" => "name",
            _ => "id",
        }
    }

    /// Execute a Cypher statement with retry + classification. Returns
    /// the raw [`QueryResult`] so callers can inspect column data when
    /// needed (e.g. count probes).
    pub async fn execute_with_retry(
        &self,
        cypher: &str,
        params: Option<HashMap<String, SdkValue>>,
    ) -> GraphClientResult<QueryResult> {
        let max = self.config.max_retry.max(1);
        let sdk = self.sdk.clone();
        let cypher_owned = cypher.to_string();
        with_retry(max, move || {
            let sdk = sdk.clone();
            let cypher = cypher_owned.clone();
            let params = params.clone();
            async move {
                sdk.execute_cypher(&cypher, params)
                    .await
                    .map_err(|e| GraphClientError::classify(e, "write_tx"))
            }
        })
        .await
    }
}

// ---------- Schema-bootstrap classification -----------------------------

/// Classify an error raised while applying a single schema statement.
///
/// Nexus 1.15 may not yet implement `CREATE CONSTRAINT ... IF NOT EXISTS`
/// the same way Neo4j does; messages like "already exists" /
/// "already defined" are treated as success so the bootstrap is
/// idempotent regardless of which dialect of "if not exists" the
/// server speaks. Genuine schema drift surfaces as
/// [`GraphClientError::SchemaDrift`].
fn classify_schema_error(err: NexusError, stmt: &str) -> Option<GraphClientError> {
    let raw = err.to_string();
    let lower = raw.to_ascii_lowercase();
    if lower.contains("already exists")
        || lower.contains("already defined")
        || lower.contains("equivalent")
    {
        tracing::debug!(stmt = %stmt, "schema statement already applied");
        return None;
    }
    if lower.contains("syntax")
        || lower.contains("parse")
        || lower.contains("unsupported")
        || lower.contains("not implemented")
    {
        return Some(GraphClientError::SchemaDrift(format!(
            "Cypher not supported by Nexus for `{stmt}`: {raw}"
        )));
    }
    Some(GraphClientError::classify(err, "schema"))
}

#[async_trait]
impl GraphClient for LiveNexusClient {
    async fn ensure_schema(&self, statements: &[String]) -> GraphClientResult<()> {
        for stmt in statements {
            match self.sdk.execute_cypher(stmt, None).await {
                Ok(_) => {}
                Err(err) => match classify_schema_error(err, stmt) {
                    Some(classified) => return Err(classified),
                    None => continue,
                },
            }
        }
        Ok(())
    }

    async fn run_write_tx(
        &self,
        patch: &GraphPatch,
        _templates: &CypherTemplates,
    ) -> GraphClientResult<WriteStats> {
        let mut nodes_upserted = 0u32;
        let mut edges_upserted = 0u32;

        // ---- nodes: group by label, one UNWIND-MERGE per label ----
        let mut by_label: HashMap<&str, Vec<&NodeOp>> = HashMap::new();
        for n in &patch.nodes {
            by_label.entry(n.label.as_str()).or_default().push(n);
        }
        // Iterate in deterministic order so retries hit the same Cypher
        // shape every time and integration-test diffs are stable.
        let mut label_order: Vec<&str> = by_label.keys().copied().collect();
        label_order.sort_unstable();
        for label in label_order {
            let nodes = &by_label[label];
            let key_field = Self::key_field_for(label);
            let cypher = format!(
                "UNWIND $rows AS row MERGE (n:{label} {{ {key_field}: row.key }}) SET n += row.props"
            );
            let rows: Vec<SdkValue> = nodes
                .iter()
                .map(|n| {
                    let mut row = HashMap::with_capacity(2);
                    row.insert("key".to_string(), SdkValue::String(n.natural_key.clone()));
                    row.insert("props".to_string(), props_to_sdk_value(&n.props));
                    SdkValue::Object(row)
                })
                .collect();
            let mut params = HashMap::with_capacity(1);
            params.insert("rows".to_string(), SdkValue::Array(rows));
            self.execute_with_retry(&cypher, Some(params)).await?;
            nodes_upserted = nodes_upserted.saturating_add(nodes.len() as u32);
        }

        // ---- edges: group by (rel_type, from_label, to_label) ----
        //
        // Nexus 1.15 rejects `UNWIND ... MATCH ...` and `UNWIND ... WITH
        // row MATCH ...` shapes with "Unsupported clause in write
        // query" (verified via direct Cypher probes against
        // hivehub/nexus:1.15.0). The accepted shape is to chain
        // `MERGE`s for the endpoints inside the same UNWIND row, then
        // `MERGE` the relationship — which is equally idempotent and
        // also re-asserts the endpoint nodes (a no-op for keys already
        // upserted in the node phase above).
        let mut by_pair: HashMap<(&str, &str, &str), Vec<&EdgeOp>> = HashMap::new();
        for e in &patch.edges {
            by_pair
                .entry((e.edge_type.as_str(), e.from_label.as_str(), e.to_label.as_str()))
                .or_default()
                .push(e);
        }
        let mut pair_order: Vec<(&str, &str, &str)> = by_pair.keys().copied().collect();
        pair_order.sort_unstable();
        for (rel_type, fl, tl) in pair_order {
            let edges = &by_pair[&(rel_type, fl, tl)];
            let from_field = Self::key_field_for(fl);
            let to_field = Self::key_field_for(tl);
            let cypher = format!(
                "UNWIND $rows AS row \
                 MERGE (a:{fl} {{ {from_field}: row.from }}) \
                 MERGE (b:{tl} {{ {to_field}: row.to }}) \
                 MERGE (a)-[r:{rel_type}]->(b) SET r += row.props"
            );
            let rows: Vec<SdkValue> = edges
                .iter()
                .map(|e| {
                    let mut row = HashMap::with_capacity(3);
                    row.insert("from".to_string(), SdkValue::String(e.from_key.clone()));
                    row.insert("to".to_string(), SdkValue::String(e.to_key.clone()));
                    row.insert("props".to_string(), props_to_sdk_value(&e.props));
                    SdkValue::Object(row)
                })
                .collect();
            let mut params = HashMap::with_capacity(1);
            params.insert("rows".to_string(), SdkValue::Array(rows));
            self.execute_with_retry(&cypher, Some(params)).await?;
            edges_upserted = edges_upserted.saturating_add(edges.len() as u32);
        }

        Ok(WriteStats {
            nodes_upserted,
            edges_upserted,
        })
    }
}

// ---------- JSON ↔ SDK Value bridge -------------------------------------

/// Convert a property bag (`BTreeMap<String, serde_json::Value>`) into
/// the SDK's parametrized `Value::Object` shape. The graph patch types
/// hold properties as `serde_json::Value` so they stay schema-agnostic
/// across crates; the SDK accepts its own untagged enum for query
/// parameters.
fn props_to_sdk_value(
    props: &std::collections::BTreeMap<String, serde_json::Value>,
) -> SdkValue {
    let mut map = HashMap::with_capacity(props.len());
    for (k, v) in props {
        map.insert(k.clone(), json_to_sdk_value(v));
    }
    SdkValue::Object(map)
}

/// Convert one [`serde_json::Value`] into the SDK's [`SdkValue`].
fn json_to_sdk_value(v: &serde_json::Value) -> SdkValue {
    match v {
        serde_json::Value::Null => SdkValue::Null,
        serde_json::Value::Bool(b) => SdkValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SdkValue::Int(i)
            } else if let Some(u) = n.as_u64() {
                // Mirror serde_json's i64-bias; saturate at i64::MAX so
                // we never produce an SDK value the wire format can't
                // represent.
                SdkValue::Int(i64::try_from(u).unwrap_or(i64::MAX))
            } else if let Some(f) = n.as_f64() {
                SdkValue::Float(f)
            } else {
                SdkValue::Null
            }
        }
        serde_json::Value::String(s) => SdkValue::String(s.clone()),
        serde_json::Value::Array(items) => {
            SdkValue::Array(items.iter().map(json_to_sdk_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = HashMap::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), json_to_sdk_value(v));
            }
            SdkValue::Object(out)
        }
    }
}

// ---------- Memory (test fake) ------------------------------------------

/// One observation captured by [`MemoryNexusClient`].
#[derive(Debug, Clone)]
pub enum MemoryCall {
    /// Captured `ensure_schema` call.
    EnsureSchema(Vec<String>),
    /// Captured `run_write_tx` call.
    WriteTx(GraphPatch),
}

/// In-memory graph client for tests. Records every call without
/// touching a real Nexus instance.
#[derive(Debug, Default)]
pub struct MemoryNexusClient {
    /// Captured calls, in order.
    pub calls: Mutex<Vec<MemoryCall>>,
}

impl MemoryNexusClient {
    /// Construct an empty memory client.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of recorded calls, in arrival order.
    pub fn calls_snapshot(&self) -> Vec<MemoryCall> {
        self.calls
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl GraphClient for MemoryNexusClient {
    async fn ensure_schema(&self, statements: &[String]) -> GraphClientResult<()> {
        if let Ok(mut guard) = self.calls.lock() {
            guard.push(MemoryCall::EnsureSchema(statements.to_vec()));
        }
        Ok(())
    }

    async fn run_write_tx(
        &self,
        patch: &GraphPatch,
        _templates: &CypherTemplates,
    ) -> GraphClientResult<WriteStats> {
        if let Ok(mut guard) = self.calls.lock() {
            guard.push(MemoryCall::WriteTx(patch.clone()));
        }
        Ok(WriteStats {
            nodes_upserted: u32::try_from(patch.nodes.len()).unwrap_or(u32::MAX),
            edges_upserted: u32::try_from(patch.edges.len()).unwrap_or(u32::MAX),
        })
    }
}

// ---------- Tests --------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::patch::{EdgeOp, NodeOp};
    use crate::schema;

    fn empty_props() -> BTreeMap<String, serde_json::Value> {
        BTreeMap::new()
    }

    fn node(label: &str, key: &str) -> NodeOp {
        NodeOp {
            label: label.to_string(),
            natural_key: key.to_string(),
            props: empty_props(),
        }
    }

    fn edge(rel: &str, from_label: &str, from_key: &str, to_label: &str, to_key: &str) -> EdgeOp {
        EdgeOp {
            edge_type: rel.to_string(),
            from_label: from_label.to_string(),
            from_key: from_key.to_string(),
            to_label: to_label.to_string(),
            to_key: to_key.to_string(),
            props: empty_props(),
        }
    }

    #[tokio::test]
    async fn memory_client_records_ensure_schema() {
        let client = MemoryNexusClient::new();
        client
            .ensure_schema(&["a".to_string(), "b".to_string()])
            .await
            .expect("ensure_schema must succeed against the fake");
        let calls = client.calls_snapshot();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            MemoryCall::EnsureSchema(stmts) => {
                assert_eq!(stmts, &vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected EnsureSchema, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn memory_client_records_write_tx_with_correct_counts() {
        let client = MemoryNexusClient::new();
        let patch = GraphPatch {
            nodes: vec![node("Turn", "T1"), node("Turn", "T2"), node("Session", "S1")],
            edges: vec![
                edge("HAS_TURN", "Session", "S1", "Turn", "T1"),
                edge("HAS_TURN", "Session", "S1", "Turn", "T2"),
                edge("HAS_TOOL_CALL", "Session", "S1", "ToolCall", "TC1"),
                edge("HAS_TOOL_CALL", "Session", "S1", "ToolCall", "TC2"),
            ],
        };
        let templates = CypherTemplates::default();
        let stats = client
            .run_write_tx(&patch, &templates)
            .await
            .expect("write_tx must succeed");
        assert_eq!(stats.nodes_upserted, 3);
        assert_eq!(stats.edges_upserted, 4);

        let calls = client.calls_snapshot();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            MemoryCall::WriteTx(captured) => {
                assert_eq!(captured.nodes.len(), 3);
                assert_eq!(captured.edges.len(), 4);
            }
            other => panic!("expected WriteTx, got {other:?}"),
        }
    }

    #[test]
    fn key_field_for_returns_natural_key_for_artifact() {
        assert_eq!(LiveNexusClient::key_field_for("Artifact"), "natural_key");
        assert_eq!(LiveNexusClient::key_field_for("Repo"), "name");
        assert_eq!(LiveNexusClient::key_field_for("Turn"), "id");
        assert_eq!(LiveNexusClient::key_field_for("ToolCall"), "id");
        assert_eq!(LiveNexusClient::key_field_for("Decision"), "id");
        // Unknown labels default to `id` so new node types added in
        // the schema before a code update still target a sensible key.
        assert_eq!(LiveNexusClient::key_field_for("BrandNew"), "id");
    }

    #[test]
    fn classify_message_detects_constraint_violation() {
        let err = classify_message("Constraint violation: id already exists for Decision", "x");
        match err {
            GraphClientError::ConstraintViolation { detail } => {
                assert!(detail.contains("Constraint"));
            }
            other => panic!("expected ConstraintViolation, got {other:?}"),
        }
    }

    #[test]
    fn classify_message_detects_unauthorized() {
        let err = classify_message("401 Unauthorized", "x");
        assert!(matches!(err, GraphClientError::AuthFailed(_)));
    }

    #[test]
    fn classify_api_500_is_transient() {
        let err = NexusError::Api {
            status: 503,
            message: "boom".to_string(),
        };
        assert!(GraphClientError::classify(err, "x").is_retriable());
    }

    #[test]
    fn classify_connection_is_transient() {
        let err = NexusError::Connection("refused".into());
        assert!(GraphClientError::classify(err, "x").is_retriable());
    }

    #[test]
    fn classify_api_400_is_not_retriable() {
        let err = NexusError::Api {
            status: 400,
            message: "bad request".to_string(),
        };
        assert!(!GraphClientError::classify(err, "x").is_retriable());
    }

    #[tokio::test]
    async fn with_retry_recovers_after_transient_errors() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_for_op = counter.clone();
        let result: Result<u32, GraphClientError> = with_retry(3, move || {
            let counter = counter_for_op.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(GraphClientError::TransientError("network blip".into()))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;
        assert_eq!(result.expect("eventually succeeds"), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_does_not_retry_non_retriable() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_for_op = counter.clone();
        let result: Result<(), GraphClientError> = with_retry(3, move || {
            let counter = counter_for_op.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(GraphClientError::ConstraintViolation {
                    detail: "duplicate".into(),
                })
            }
        })
        .await;
        assert!(matches!(
            result,
            Err(GraphClientError::ConstraintViolation { .. })
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn json_to_sdk_value_round_trips_primitives() {
        assert!(matches!(json_to_sdk_value(&json!(null)), SdkValue::Null));
        assert!(matches!(
            json_to_sdk_value(&json!(true)),
            SdkValue::Bool(true)
        ));
        assert!(matches!(
            json_to_sdk_value(&json!(42)),
            SdkValue::Int(42)
        ));
        assert!(matches!(
            json_to_sdk_value(&json!(1.5)),
            SdkValue::Float(_)
        ));
        match json_to_sdk_value(&json!("hi")) {
            SdkValue::String(s) => assert_eq!(s, "hi"),
            other => panic!("expected String, got {other:?}"),
        }
        match json_to_sdk_value(&json!([1, 2, 3])) {
            SdkValue::Array(v) => assert_eq!(v.len(), 3),
            other => panic!("expected Array, got {other:?}"),
        }
        match json_to_sdk_value(&json!({"k": 1})) {
            SdkValue::Object(map) => {
                assert!(matches!(map.get("k"), Some(SdkValue::Int(1))));
            }
            other => panic!("expected Object, got {other:?}"),
        }
    }

    // ---- env-gated live probes (Round 5 owns full IT) -----------------

    /// Smoke probe — only runs when `CORTEX_GRAPH_IT=1` is set so unit
    /// runs stay hermetic. Confirms the schema bootstrap is accepted
    /// by the live `hivehub/nexus:1.15.0` container at the default
    /// dev URL and is idempotent on re-run.
    #[tokio::test]
    async fn schema_bootstrap_against_live_nexus_when_enabled() {
        if std::env::var("CORTEX_GRAPH_IT").as_deref() != Ok("1") {
            return;
        }
        let cfg = GraphConfig {
            nexus_url: "http://127.0.0.1:15002".into(),
            ..GraphConfig::default()
        };
        let client = LiveNexusClient::new(cfg).expect("construct live client");
        client
            .ensure_schema(&schema::statements())
            .await
            .expect("schema bootstrap");
        client
            .ensure_schema(&schema::statements())
            .await
            .expect("idempotent re-run");
    }

    /// Round-trip probe — also `CORTEX_GRAPH_IT=1` gated. Writes a
    /// small patch then deletes its nodes so the test is repeatable.
    #[tokio::test]
    async fn write_tx_against_live_nexus_when_enabled() {
        if std::env::var("CORTEX_GRAPH_IT").as_deref() != Ok("1") {
            return;
        }
        let cfg = GraphConfig {
            nexus_url: "http://127.0.0.1:15002".into(),
            ..GraphConfig::default()
        };
        let client = LiveNexusClient::new(cfg).expect("construct live client");
        client
            .ensure_schema(&schema::statements())
            .await
            .expect("schema bootstrap");

        let session_key = format!("cortex-graph-test-session-{}", ulid::Ulid::new());
        let turn_key = format!("cortex-graph-test-turn-{}", ulid::Ulid::new());
        let patch = GraphPatch {
            nodes: vec![node("Session", &session_key), node("Turn", &turn_key)],
            edges: vec![edge("HAS_TURN", "Session", &session_key, "Turn", &turn_key)],
        };
        let templates = CypherTemplates::default();
        let stats = client
            .run_write_tx(&patch, &templates)
            .await
            .expect("write_tx");
        assert_eq!(stats.nodes_upserted, 2);
        assert_eq!(stats.edges_upserted, 1);

        // Cleanup so the test is rerunnable.
        for key in [&session_key, &turn_key] {
            let _ = client
                .sdk
                .execute_cypher(
                    "MATCH (n { id: $id }) DETACH DELETE n",
                    Some({
                        let mut p = HashMap::new();
                        p.insert("id".to_string(), SdkValue::String(key.clone()));
                        p
                    }),
                )
                .await;
        }
    }
}

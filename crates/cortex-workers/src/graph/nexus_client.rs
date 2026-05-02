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

use super::config::{GraphConfig, GraphTransport};
use super::cypher::CypherTemplates;
use super::patch::{EdgeOp, GraphPatch, NodeOp};

/// Per-transaction write counters returned by a successful
/// [`GraphClient::run_write_tx`] call.
///
/// `edges_upserted` reports edges that Nexus *confirmed it persisted*
/// (the per-row `RETURN count(*)` came back non-zero). Silent drops —
/// where Nexus accepts the HTTP call but the underlying `MATCH … MERGE`
/// found no endpoint and produced no relationship — are kept off the
/// upserted counter and surfaced through `edges_dropped` so the worker
/// can WARN and bump `cortex_graph_edges_dropped{edge_type}`. See
/// `phase2_graph_emit_session_and_provenance_edges/design.md` for the
/// failure-mode probe transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteStats {
    /// Number of node `MERGE`s that ran in the transaction.
    pub nodes_upserted: u32,
    /// Number of edge `MERGE`s Nexus confirmed it persisted.
    pub edges_upserted: u32,
    /// Edges the writer attempted to MERGE but Nexus silently dropped,
    /// keyed by `edge_type`. Non-empty values are the actionable signal
    /// for the operator: cross-batch endpoint races, schema drift,
    /// or a regression in the writer's MATCH-MERGE shape.
    pub edges_dropped: std::collections::BTreeMap<String, u32>,
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
    /// The patch references a (label) or (from-label, rel-type, to-label)
    /// for which no Cypher template was loaded. Indicates a mapper /
    /// template-registry mismatch — surfaced as a hard error so missing
    /// templates never silently degrade to dropped writes.
    #[error("missing cypher template `{0}`")]
    MissingTemplate(String),
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
                    GraphClientError::TransientError(format!("{purpose}: api {status}: {message}"))
                } else {
                    classify_message(message, purpose)
                }
            }
            NexusError::Connection(msg) | NexusError::Network(msg) => {
                GraphClientError::TransientError(format!("{purpose}: {msg}"))
            }
            NexusError::Timeout => GraphClientError::TransientError(format!("{purpose}: timeout")),
            NexusError::Http(http_err) => {
                let is_transient = http_err.is_timeout()
                    || http_err.is_connect()
                    || http_err
                        .status()
                        .map(|s| s.is_server_error())
                        .unwrap_or(false);
                if is_transient {
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
/// Mirrors the contract of [`crate::embedder::with_retry`] so both
/// workers behave identically under transient remote-server pressure.
pub async fn with_retry<F, Fut, T>(max_attempts: u32, mut f: F) -> Result<T, GraphClientError>
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
        // Nexus 1.15.0 silently drops every write that touches an `UNWIND`
        // row or that uses `$param` substitution inside a write clause:
        // the HTTP call returns 200 with zero rows but nothing is
        // persisted. (Reads with `$param` work fine; only the write
        // substitution path is broken.) Until a Nexus version that fixes
        // this ships, every write is rendered as a per-row Cypher
        // statement with values escaped into the literal — so MERGE,
        // SET, and the relationship-MATCH/MERGE pair all see literal
        // values rather than parameter slots.
        //
        // The `_templates` registry is kept as a parameter for API
        // stability; the live writer no longer reads from it. See task
        // `phase1_graph_writer_nexus_compat` for the full diagnosis.
        let mut nodes_upserted = 0u32;
        let mut edges_upserted = 0u32;

        // Iterate in deterministic order so retries hit the same Cypher
        // shape every time and integration-test diffs are stable.
        let mut nodes_sorted: Vec<&NodeOp> = patch.nodes.iter().collect();
        nodes_sorted.sort_by(|a, b| {
            a.label
                .cmp(&b.label)
                .then_with(|| a.natural_key.cmp(&b.natural_key))
        });
        for node in nodes_sorted {
            let cypher = render_node_merge(node);
            let result = self.execute_with_retry(&cypher, None).await?;
            assert_write_landed(&result, "node", &node.label, &node.natural_key)?;
            nodes_upserted = nodes_upserted.saturating_add(1);
        }

        let mut edges_sorted: Vec<&EdgeOp> = patch.edges.iter().collect();
        edges_sorted.sort_by(|a, b| {
            (
                a.edge_type.as_str(),
                a.from_label.as_str(),
                a.to_label.as_str(),
            )
                .cmp(&(
                    b.edge_type.as_str(),
                    b.from_label.as_str(),
                    b.to_label.as_str(),
                ))
                .then_with(|| {
                    (a.from_key.as_str(), a.to_key.as_str())
                        .cmp(&(b.from_key.as_str(), b.to_key.as_str()))
                })
        });
        let mut edges_dropped: std::collections::BTreeMap<String, u32> =
            std::collections::BTreeMap::new();
        for edge in edges_sorted {
            let cypher = render_edge_merge(edge);
            let result = self.execute_with_retry(&cypher, None).await?;
            let pair = format!("{}->{}", edge.from_key, edge.to_key);
            // Edges that Nexus silently drops (MATCH found no endpoint)
            // are isolated from the rest of the batch: we tally them
            // for the operator and continue, rather than aborting an
            // otherwise-good transaction. Cross-batch endpoint races
            // resolve themselves on the next replay.
            match assert_write_landed(&result, "edge", &edge.edge_type, &pair) {
                Ok(()) => {
                    edges_upserted = edges_upserted.saturating_add(1);
                }
                Err(GraphClientError::Nexus(detail)) if detail.contains("count=0") => {
                    *edges_dropped.entry(edge.edge_type.clone()).or_insert(0) += 1;
                    tracing::warn!(
                        edge_type = %edge.edge_type,
                        from_label = %edge.from_label,
                        from_key = %edge.from_key,
                        to_label = %edge.to_label,
                        to_key = %edge.to_key,
                        "edge silently dropped by Nexus (MATCH found no endpoint)"
                    );
                }
                Err(other) => return Err(other),
            }
        }

        Ok(WriteStats {
            nodes_upserted,
            edges_upserted,
            edges_dropped,
        })
    }
}

// ---------- Per-row Cypher renderers (Nexus 1.15 compat) ----------------

/// Render one node-MERGE statement with every value already escaped into
/// the Cypher string literal. Format:
///
/// ```text
/// MERGE (n:<Label> { <key_field>: "<key>" }) SET n.prop_a = "v", n.prop_b = 42 RETURN count(n) AS written
/// ```
///
/// `RETURN count(n) AS written` lets the caller assert the write
/// actually landed (Nexus 1.15 returns 0 for the silently-dropped
/// shapes; a real persist returns 1).
fn render_node_merge(node: &NodeOp) -> String {
    let key_field = LiveNexusClient::key_field_for(&node.label);
    let mut cy = format!(
        "MERGE (n:{label} {{ {kf}: {key} }})",
        label = node.label,
        kf = key_field,
        key = cypher_string_literal(&node.natural_key),
    );
    if !node.props.is_empty() {
        cy.push_str(" SET ");
        let mut first = true;
        for (k, v) in &node.props {
            if !first {
                cy.push_str(", ");
            }
            first = false;
            cy.push_str(&format!("n.{k} = {}", value_to_cypher_literal(v)));
        }
    }
    cy.push_str(" RETURN count(n) AS written");
    cy
}

/// Render one edge-MERGE statement (MATCH endpoints, MERGE the
/// relationship, optional SET on the relationship). Endpoints must
/// already exist — the writer always upserts nodes before edges.
fn render_edge_merge(edge: &EdgeOp) -> String {
    let from_kf = LiveNexusClient::key_field_for(&edge.from_label);
    let to_kf = LiveNexusClient::key_field_for(&edge.to_label);
    let mut cy = format!(
        "MATCH (a:{fl} {{ {fkf}: {fk} }}), (b:{tl} {{ {tkf}: {tk} }}) \
         MERGE (a)-[r:{rt}]->(b)",
        fl = edge.from_label,
        fkf = from_kf,
        fk = cypher_string_literal(&edge.from_key),
        tl = edge.to_label,
        tkf = to_kf,
        tk = cypher_string_literal(&edge.to_key),
        rt = edge.edge_type,
    );
    if !edge.props.is_empty() {
        cy.push_str(" SET ");
        let mut first = true;
        for (k, v) in &edge.props {
            if !first {
                cy.push_str(", ");
            }
            first = false;
            cy.push_str(&format!("r.{k} = {}", value_to_cypher_literal(v)));
        }
    }
    cy.push_str(" RETURN count(r) AS written");
    cy
}

/// Encode a string as a Cypher double-quoted string literal.
///
/// Earlier revisions reused `serde_json::to_string` on the assumption
/// that JSON's escape rules are a Cypher-compatible subset. They are
/// not: the 2026-04-28 archive backfill probed Nexus 1.15 directly and
/// found two distinct failure modes when display text contained
/// non-ASCII codepoints:
///
/// 1. The Nexus JSON body parser rejects every `\uXXXX` escape whose
///    codepoint is `>= 0x80` with `invalid unicode code point` —
///    serde_json's default ASCII-safe encoding produces exactly those
///    escapes, so a Turn name with an ellipsis (`…`, U+2026) silently
///    failed at HTTP time.
/// 2. The same parser also rejects raw multi-byte UTF-8 sequences
///    (e.g. `0xE2 0x80 0xA6` for U+2026) — even though those are valid
///    UTF-8 / valid JSON, Nexus's parser folds them into the same
///    `invalid unicode code point` bucket.
///
/// The fix is to render Cypher that is ASCII-only: escape the small
/// set Cypher's own parser understands (`\"`, `\\`, `\n`, `\r`, `\t`),
/// drop ASCII control bytes that would confuse Cypher, transliterate
/// `…` into the three-dot ASCII ellipsis (it's the truncation marker
/// the mapper produces, so a 1:1 substitution preserves intent), and
/// replace every other non-ASCII codepoint with `?`. The display
/// labels lose accents on non-ASCII repos, but no batch is ever
/// dropped — display fidelity is recoverable later when Nexus's body
/// parser is fixed.
fn cypher_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            '\u{2026}' => out.push_str("..."),
            c if (c as u32) < 0x80 => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push('"');
    out
}

/// Encode a `serde_json::Value` as a Cypher literal. Primitives map
/// 1:1; arrays/objects are flattened to their JSON encoding wrapped as
/// a string literal so Nexus 1.15 doesn't choke on nested structures.
fn value_to_cypher_literal(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => cypher_string_literal(s),
        serde_json::Value::Array(items) => {
            // Flatten primitive-only arrays as Cypher lists; otherwise
            // fall back to a JSON-string literal.
            if items.iter().all(|i| {
                matches!(
                    i,
                    serde_json::Value::Null
                        | serde_json::Value::Bool(_)
                        | serde_json::Value::Number(_)
                        | serde_json::Value::String(_)
                )
            }) {
                let inner: Vec<String> = items.iter().map(value_to_cypher_literal).collect();
                format!("[{}]", inner.join(", "))
            } else {
                cypher_string_literal(&serde_json::to_string(v).unwrap_or_default())
            }
        }
        serde_json::Value::Object(_) => {
            cypher_string_literal(&serde_json::to_string(v).unwrap_or_default())
        }
    }
}

/// Confirm a write Cypher's RETURN actually produced a row.
///
/// Nexus 1.15 has two failure-modes that look like success at the
/// HTTP layer:
///
/// 1. UNWIND-write / `$param`-write — silently dropped, response has
///    `rows: []` (empty). Caught here as `rows.is_empty() == true`.
///    (The phase1_graph_writer_nexus_compat task moved the live path
///    off these shapes, but defence in depth is cheap.)
/// 2. `MATCH (a:From {…}), (b:To {…}) MERGE (a)-[:R]->(b) RETURN count(*)`
///    where the `MATCH` finds nothing — Nexus returns `rows: [[0]]`.
///    Caught here by reading the first cell and rejecting integer `0`.
///    A *successful* write returns `rows: [[null]]` (Nexus 1.15 doesn't
///    surface real counts on writes), which we treat as success.
///
/// The phase2_graph_emit_session_and_provenance_edges audit found 15
/// of 17 `HAS_TURN` edges silently lost in production through case 2:
/// the MATCH on the from-endpoint failed against the live Nexus while
/// a sibling write transaction held the Session, and the previous
/// `rows.is_empty()` check let it slide.
fn assert_write_landed(
    result: &QueryResult,
    op: &str,
    op_kind: &str,
    op_key: &str,
) -> GraphClientResult<()> {
    if result.rows.is_empty() {
        return Err(GraphClientError::Nexus(format!(
            "{op} write not persisted (kind={op_kind}, key={op_key}, no rows)"
        )));
    }
    // Walk the first cell. Treat an integer `0` (any signed/unsigned
    // width) as a confirmed silent drop. `null`, positive integers,
    // and non-numeric cells stay classified as success — Nexus 1.15
    // returns `[[null]]` on every successful write, and tightening
    // beyond "is it literally zero" would re-introduce the false
    // positives that masked phase1's parametrised-write bug.
    if let Some(first_cell) = result
        .rows
        .first()
        .and_then(|row| row.as_array())
        .and_then(|cells| cells.first())
    {
        if let Some(count) = first_cell.as_i64() {
            if count == 0 {
                return Err(GraphClientError::Nexus(format!(
                    "{op} write not persisted (kind={op_kind}, key={op_key}, count=0)"
                )));
            }
        } else if let Some(count) = first_cell.as_u64() {
            if count == 0 {
                return Err(GraphClientError::Nexus(format!(
                    "{op} write not persisted (kind={op_kind}, key={op_key}, count=0)"
                )));
            }
        }
    }
    Ok(())
}

// ---------- JSON ↔ SDK Value bridge -------------------------------------

/// Convert one [`serde_json::Value`] into the SDK's [`SdkValue`].
#[doc(hidden)]
pub fn json_to_sdk_value(v: &serde_json::Value) -> SdkValue {
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
            edges_dropped: std::collections::BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn qresult(rows: Vec<serde_json::Value>) -> QueryResult {
        QueryResult {
            columns: vec!["written".to_string()],
            rows,
            execution_time_ms: Some(0),
            error: None,
        }
    }

    /// Pre-fix behaviour relied on `rows.is_empty()`. Empty rows still
    /// signal a silent drop and the new check must keep flagging it.
    #[test]
    fn assert_write_landed_rejects_empty_rows() {
        let r = qresult(vec![]);
        let err = assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").unwrap_err();
        match err {
            GraphClientError::Nexus(detail) => {
                assert!(detail.contains("no rows"), "msg={detail}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Post-fix behaviour: Nexus 1.15 returns `[[null]]` for every
    /// successful write. That's not a silent drop — the writer must
    /// keep counting it as success or every persisted edge is mis-
    /// classified.
    #[test]
    fn assert_write_landed_accepts_null_cell_as_success() {
        let r = qresult(vec![json!([null])]);
        assert!(assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").is_ok());
    }

    /// The phase2 fix: when Nexus returns `[[0]]` the MATCH found no
    /// endpoint and the MERGE created nothing. The pre-fix check let
    /// this slide because `rows.is_empty()` is false. The post-fix
    /// check must reject it with a `count=0` error so the worker
    /// can route it through the `edges_dropped` accounting path.
    #[test]
    fn assert_write_landed_rejects_zero_count_silent_drop() {
        let r = qresult(vec![json!([0])]);
        let err = assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").unwrap_err();
        match err {
            GraphClientError::Nexus(detail) => {
                assert!(
                    detail.contains("count=0") && detail.contains("HAS_TURN"),
                    "msg={detail}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Positive integers (rare on writes, but possible if Nexus ever
    /// surfaces real counts) should keep counting as success.
    #[test]
    fn assert_write_landed_accepts_positive_integer_count() {
        let r = qresult(vec![json!([3])]);
        assert!(assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").is_ok());
    }

    /// Non-numeric cells (Nexus could in theory return a node row)
    /// stay classified as success — tightening beyond "is it
    /// literally zero" risks reintroducing the false positives that
    /// masked the parametrised-write bug fixed in phase1.
    #[test]
    fn assert_write_landed_accepts_non_numeric_cell() {
        let r = qresult(vec![json!([{"id": "n-1"}])]);
        assert!(assert_write_landed(&r, "node", "Session", "s1").is_ok());
    }

    /// A row that isn't even a list (e.g. a bare scalar) keeps the
    /// previous "trust the wire shape" behaviour — only literal `0`
    /// in the first cell of an array row is a silent drop.
    #[test]
    fn assert_write_landed_accepts_non_array_row() {
        let r = qresult(vec![json!(null)]);
        assert!(assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").is_ok());
    }

    /// Spec-07 §Post-write verification: the writer must confirm the
    /// count is also non-zero when the cell is an unsigned integer
    /// (some nexus_sdk versions decode write-counts via `as_u64`).
    #[test]
    fn assert_write_landed_rejects_zero_unsigned_count() {
        let r = qresult(vec![json!([0u64])]);
        assert!(assert_write_landed(&r, "edge", "HAS_TURN", "s1->t1").is_err());
    }
}

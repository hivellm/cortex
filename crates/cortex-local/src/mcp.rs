//! Bounded newline JSON-RPC transport over the local engine only.
use crate::{
    classification::{ClassificationQueue, DEFAULT_MODEL},
    *,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    path::Path,
};

const MAX_FRAME: usize = 4 * 1024 * 1024;
const TOOLS: &[(&str, &str)] = &[
    ("cortex_feedback_record", "Record immutable declared feedback grounded in an exact scoped delivery."),
    ("cortex_feedback_get", "Read exact scoped feedback and derived eligibility."),
    ("cortex_feedback_list", "List bounded scoped feedback; declared actors do not prove independent votes or consumption."),
    ("cortex_audit_list", "List bounded scoped audit evidence."),
    ("cortex_audit_get", "Read one exact scoped audit record."),
    (
        "cortex_audit_metrics",
        "Inspect bounded evidence metrics and denominators.",
    ),
    (
        "cortex_audit_retain",
        "Preview or explicitly archive audit deliveries without deleting sources.",
    ),
    (
        "cortex_search",
        "Search scoped local records; returns identities and provenance.",
    ),
    (
        "cortex_query",
        "Alias for scoped local search; no external retrieval backends.",
    ),
    (
        "cortex_get",
        "Read a scoped record or an explicit historical revision.",
    ),
    (
        "cortex_capture_memory",
        "Capture explicit local memory with source; does not classify or infer acceptance.",
    ),
    (
        "cortex_revise",
        "Create an immutable revision with an expected current revision.",
    ),
    (
        "cortex_graph",
        "Inspect bounded local typed relationships in one repository.",
    ),
    ("cortex_status", "Read scoped local corpus counts."),
    (
        "cortex_pre_thinking",
        "Prepare bounded cited local context; does not prove model consumption.",
    ),
    (
        "cortex_memory_read",
        "Read bounded cited local memories with provenance.",
    ),
    (
        "cortex_classification_enqueue",
        "Queue advisory classification locally; makes no provider call.",
    ),
    (
        "cortex_classification_jobs",
        "List scoped classification jobs; makes no provider call.",
    ),
];

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(message.into())
}

pub fn descriptors() -> Value {
    Value::Array(TOOLS.iter().map(|(name, description)| {
        let mut properties = json!({
            "repository":{"type":"string"},"repo":{"type":"string"},"cwd":{"type":"string"},
            "scope":{"type":"object","properties":{"repo":{"type":"string"}}},
            "query":{"type":"string","maxLength":4096},"text":{"type":"string","maxLength":4096},
            "task":{"type":"string","maxLength":4096},"prompt":{"type":"string","maxLength":4096},
            "id":{"type":"string"},"revision":{"type":"integer","minimum":1},
            "limit":{"type":"integer","minimum":1,"maximum":100},
            "include_inactive":{"type":"boolean"},"depth":{"type":"integer","minimum":0,"maximum":5},
            "budget_bytes":{"type":"integer","minimum":256,"maximum":6144},
            "attribution":{"type":"object","properties":{"client":{"type":"string","maxLength":256},"session_id":{"type":"string","maxLength":256},"turn_id":{"type":"string","maxLength":256},"event_id":{"type":"string","maxLength":256},"agent_id":{"type":"string","maxLength":256}},"additionalProperties":false},
            "title":{"type":"string"},"body":{"type":"string"},
            "kind":{"enum":["decision","lesson","preference","note"]},
            "category":{"enum":["decision","lesson","preference","note"]},
            "authority":{"enum":["observation","proposed","accepted","deprecated","superseded"]},
            "status":{"enum":["observation","proposed","accepted","deprecated","superseded"]},
            "source":{},"links":{"type":"array"},"metadata":{},
            "expected_revision":{"type":"integer","minimum":1},"model":{"type":"string"},
            "after":{"type":"string"},"before":{"type":"string"},"cursor":{"type":"string"},
            "include_archived":{"type":"boolean"},"cutoff":{"type":"string"},"apply":{"type":"boolean","default":false},
            "evidence":{"enum":["explicit","inferred"]},"judgment":{"enum":["helpful","unhelpful","unknown"]},
            "query_id":{"type":"string"},"delivery_id":{"type":"string"},
            "reference":{"type":"object"},"comment":{"type":"string"},
            "feedback":{"type":"boolean","default":false},"as_of":{"type":"string"}
        });
        match *name {
            "cortex_feedback_record" => {
                properties["source"] = json!({"type":"object","properties":{"kind":{"type":"string"},"reference":{"type":"string"},"actor":{"type":"string"}},"required":["kind","reference","actor"],"additionalProperties":false});
                properties["reference"] = json!({"type":"object","properties":{"repository":{"type":"string"},"id":{"type":"string"},"revision":{"type":"integer","minimum":1},"content_sha256":{"type":"string"},"source_kind":{"type":"string"},"source_sha256":{"type":"string"},"body_clipped":{"type":"boolean"}},"required":["repository","id","revision","content_sha256","source_kind","source_sha256","body_clipped"],"additionalProperties":false});
            }
            "cortex_feedback_list" => properties["limit"]["default"] = json!(20),
            "cortex_audit_list" => {
                properties["kind"] = json!({"enum":["delivery","observation","archive_control"]});
                properties["limit"]["default"] = json!(20);
            }
            "cortex_audit_metrics" => {
                properties["limit"]["maximum"] = json!(10000);
                properties["limit"]["default"] = json!(10000);
            }
            "cortex_audit_retain" => properties["limit"]["default"] = json!(100),
            _ => {}
        }
        let required = match *name {
            "cortex_feedback_record" => vec!["id","source","evidence","judgment","query_id","delivery_id"],
            "cortex_feedback_get" => vec!["id"],
            "cortex_audit_get" => vec!["id"],
            "cortex_audit_retain" => vec!["cutoff"],
            "cortex_get" | "cortex_graph" | "cortex_classification_enqueue" => vec!["id"],
            "cortex_capture_memory" => vec!["title","body","source"],
            "cortex_revise" => vec!["id","expected_revision","title","body","authority"],
            _ => Vec::new(),
        };
        // All tools accept the same scope aliases; unsupported fields are rejected by dispatch.
        properties.as_object_mut().unwrap().retain(|key,_| allowed(name).contains(&key.as_str()));
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect())
}

fn allowed(name: &str) -> Vec<&'static str> {
    let mut fields = vec!["repository", "repo", "cwd", "scope"];
    fields.extend(match name {
        "cortex_feedback_record" => vec![
            "id",
            "source",
            "evidence",
            "judgment",
            "query_id",
            "delivery_id",
            "reference",
            "comment",
        ],
        "cortex_feedback_get" => vec!["id"],
        "cortex_feedback_list" => vec!["limit", "cursor"],
        "cortex_audit_list" => vec![
            "limit",
            "after",
            "before",
            "kind",
            "cursor",
            "include_archived",
        ],
        "cortex_audit_get" => vec!["id"],
        "cortex_audit_metrics" => vec!["limit", "after", "before", "include_archived"],
        "cortex_audit_retain" => vec!["cutoff", "limit", "apply"],
        "cortex_search" | "cortex_query" => vec![
            "query",
            "text",
            "limit",
            "include_inactive",
            "feedback",
            "as_of",
        ],
        "cortex_get" => vec!["id", "revision"],
        "cortex_capture_memory" => vec![
            "id",
            "title",
            "body",
            "source",
            "kind",
            "category",
            "authority",
            "status",
            "links",
            "metadata",
        ],
        "cortex_revise" => vec![
            "id",
            "expected_revision",
            "title",
            "body",
            "authority",
            "links",
            "metadata",
        ],
        "cortex_graph" => vec!["id", "depth", "limit"],
        "cortex_pre_thinking" | "cortex_memory_read" => {
            vec![
                "query",
                "text",
                "task",
                "prompt",
                "limit",
                "budget_bytes",
                "attribution",
            ]
        }
        "cortex_classification_enqueue" => vec!["id", "model"],
        "cortex_classification_jobs" => vec!["limit"],
        _ => vec![],
    });
    fields
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("required string argument missing"))
}
fn number(args: &Value, key: &str, default: u64, max: u64) -> Result<u64> {
    let value = match args.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| invalid("expected nonnegative integer"))?,
        None => default,
    };
    if value > max {
        return Err(invalid("numeric argument exceeds limit"));
    }
    Ok(value)
}
fn optional_text(args: &Value, key: &str) -> Result<Option<String>> {
    args.get(key)
        .map(|_| text(args, key).map(str::to_owned))
        .transpose()
}
fn boolean(args: &Value, key: &str) -> Result<bool> {
    args.get(key).map_or(Ok(false), |value| {
        value.as_bool().ok_or_else(|| invalid("expected boolean"))
    })
}
fn repository(args: &Value, default: Option<&str>) -> Result<String> {
    let mut chosen: Option<&str> = None;
    for key in ["repository", "repo"] {
        if let Some(value) = args.get(key) {
            let value = value
                .as_str()
                .ok_or_else(|| invalid("repository must be string"))?;
            if chosen.is_some_and(|old| old != value) {
                return Err(invalid("conflicting repository arguments"));
            }
            chosen = Some(value);
        }
    }
    if let Some(scope) = args.get("scope") {
        let scope = scope
            .as_object()
            .ok_or_else(|| invalid("scope requires explicit repo object"))?;
        if scope.len() != 1 || !scope.contains_key("repo") {
            return Err(invalid("only exact repository scope supported"));
        }
        let value = scope["repo"]
            .as_str()
            .ok_or_else(|| invalid("scope repo must be string"))?;
        if chosen.is_some_and(|old| old != value) {
            return Err(invalid("conflicting repository scope"));
        }
        chosen = Some(value);
    }
    if let Some(cwd) = args.get("cwd") {
        let cwd = cwd
            .as_str()
            .ok_or_else(|| invalid("cwd must be absolute string"))?;
        if !Path::new(cwd).is_absolute() {
            return Err(invalid("cwd must be absolute"));
        }
        return crate::runtime::resolve_repository(Path::new(cwd), chosen);
    }
    let repo = chosen
        .or(default)
        .ok_or_else(|| invalid("explicit repository or absolute cwd required"))?;
    crate::files::validate_scope(repo)?;
    Ok(repo.into())
}

pub fn call(
    store: &LocalStore,
    name: &str,
    args: Value,
    default_repository: Option<&str>,
) -> Result<Value> {
    if !TOOLS.iter().any(|(tool, _)| *tool == name) {
        return Err(invalid("unknown local tool"));
    }
    let object = args
        .as_object()
        .ok_or_else(|| invalid("arguments must be object"))?;
    if object
        .keys()
        .any(|key| !allowed(name).contains(&key.as_str()))
    {
        return Err(invalid("unsupported tool argument"));
    }
    let repository = repository(&args, default_repository)?;
    let (default_limit, maximum_limit) = match name {
        "cortex_audit_list" | "cortex_feedback_list" => (20, 100),
        "cortex_audit_metrics" => (10000, 10000),
        "cortex_audit_retain" => (100, 100),
        _ => (5, 100),
    };
    let limit = number(&args, "limit", default_limit, maximum_limit)? as usize;
    let mut query = None;
    for key in ["query", "text", "task", "prompt"] {
        if let Some(value) = args.get(key) {
            let value = value
                .as_str()
                .ok_or_else(|| invalid("query must be string"))?;
            if query.is_some_and(|old| old != value) {
                return Err(invalid("conflicting query arguments"));
            }
            if value.len() > 4096 {
                return Err(invalid("query exceeds 4096 bytes"));
            }
            query = Some(value);
        }
    }
    let query = query.unwrap_or("");
    match name {
        "cortex_feedback_record" => {
            let mut request = args.clone();
            let object = request
                .as_object_mut()
                .ok_or_else(|| invalid("arguments must be object"))?;
            for alias in ["repo", "cwd", "scope"] {
                object.remove(alias);
            }
            object.insert("repository".into(), json!(repository));
            for key in ["id", "query_id", "delivery_id"] {
                text(&request, key)?;
            }
            if request.get("comment").is_some() {
                text(&request, "comment")?;
            }
            if request
                .get("reference")
                .is_some_and(|value| !value.is_object())
            {
                return Err(invalid("reference must be object"));
            }
            Ok(serde_json::to_value(
                store.record_feedback(serde_json::from_value(request)?)?,
            )?)
        }
        "cortex_feedback_get" => Ok(serde_json::to_value(
            store.get_feedback(&repository, text(&args, "id")?)?,
        )?),
        "cortex_feedback_list" => Ok(serde_json::to_value(store.list_feedback(
            crate::feedback::FeedbackListQuery {
                repository,
                limit,
                cursor: optional_text(&args, "cursor")?,
            },
        )?)?),
        "cortex_audit_list" => Ok(serde_json::to_value(
            store.list_audit(crate::audit::AuditListQuery {
                repository,
                limit,
                after: optional_text(&args, "after")?,
                before: optional_text(&args, "before")?,
                kind: args
                    .get("kind")
                    .map(|_| {
                        text(&args, "kind")
                            .and_then(|value| Ok(serde_json::from_value(json!(value))?))
                    })
                    .transpose()?,
                cursor: optional_text(&args, "cursor")?,
                include_archived: boolean(&args, "include_archived")?,
            })?,
        )?),
        "cortex_audit_get" => Ok(serde_json::to_value(
            store.get_audit(&repository, text(&args, "id")?)?,
        )?),
        "cortex_audit_metrics" => Ok(serde_json::to_value(store.audit_metrics(
            crate::audit::AuditMetricsQuery {
                repository,
                limit,
                after: optional_text(&args, "after")?,
                before: optional_text(&args, "before")?,
                include_archived: boolean(&args, "include_archived")?,
            },
        )?)?),
        "cortex_audit_retain" => Ok(serde_json::to_value(store.retain_audit(
            crate::audit::AuditRetentionRequest {
                repository,
                limit,
                cutoff: text(&args, "cutoff")?.into(),
                apply: boolean(&args, "apply")?,
            },
        )?)?),
        "cortex_status" => Ok(serde_json::to_value(store.status(Some(&repository))?)?),
        "cortex_get" => Ok(serde_json::to_value(if args.get("revision").is_some() {
            store.get_revision(
                &repository,
                text(&args, "id")?,
                number(&args, "revision", 1, i64::MAX as u64)?,
            )?
        } else {
            store.get(&repository, text(&args, "id")?)?
        })?),
        "cortex_search" | "cortex_query" => {
            let include_inactive = match args.get("include_inactive") {
                Some(v) => v
                    .as_bool()
                    .ok_or_else(|| invalid("include_inactive must be boolean"))?,
                None => false,
            };
            let enabled = boolean(&args, "feedback")?;
            let as_of = optional_text(&args, "as_of")?;
            if !enabled && as_of.is_some() {
                return Err(invalid("as_of requires feedback=true"));
            }
            Ok(serde_json::to_value(store.search_with_feedback(
                SearchQuery {
                    repository,
                    text: query.into(),
                    limit,
                    include_inactive,
                },
                RankingOptions { enabled, as_of },
            )?)?)
        }
        "cortex_memory_read" | "cortex_pre_thinking" => {
            let budget = number(&args, "budget_bytes", 6144, 6144)? as usize;
            let attribution =
                serde_json::from_value(args.get("attribution").cloned().unwrap_or(json!({})))?;
            crate::audit::attribution(&attribution)?;
            Ok(serde_json::to_value(crate::recall::prepare(
                store,
                crate::recall::RecallRequest {
                    repository,
                    query: query.into(),
                    limit,
                    budget_bytes: budget,
                    origin: AuditOrigin::Mcp,
                    attribution,
                    fallback_recent: false,
                },
            )?)?)
        }
        "cortex_capture_memory" => {
            for (left, right) in [("kind", "category"), ("authority", "status")] {
                if matches!((args.get(left),args.get(right)),(Some(a),Some(b)) if a!=b) {
                    return Err(invalid("conflicting capture aliases"));
                }
            }
            let kind = match args.get("kind").or(args.get("category")) {
                Some(v) => serde_json::from_value(v.clone())?,
                None => RecordKind::Note,
            };
            if kind == RecordKind::Event {
                return Err(invalid("memory capture does not accept raw events"));
            }
            let authority = match args.get("authority").or(args.get("status")) {
                Some(v) => serde_json::from_value(v.clone())?,
                None => Authority::Proposed,
            };
            let raw = args
                .get("source")
                .ok_or_else(|| invalid("source required"))?;
            let source = if let Some(reference) = raw.as_str() {
                Source {
                    kind: "explicit_capture".into(),
                    locator: reference.into(),
                    payload: json!({"reference":reference}),
                    ..Default::default()
                }
            } else {
                serde_json::from_value(raw.clone())?
            };
            if source.kind.trim().is_empty() || source.locator.trim().is_empty() {
                return Err(invalid("source requires nonempty kind and locator"));
            }
            let request = CaptureRequest {
                id: args
                    .get("id")
                    .map(|_| text(&args, "id").map(str::to_owned))
                    .transpose()?,
                repository,
                kind,
                authority,
                title: text(&args, "title")?.into(),
                body: text(&args, "body")?.into(),
                source,
                links: serde_json::from_value(args.get("links").cloned().unwrap_or(json!([])))?,
                metadata: args.get("metadata").cloned().unwrap_or(json!({})),
            };
            Ok(serde_json::to_value(store.capture(request)?)?)
        }
        "cortex_revise" => Ok(serde_json::to_value(
            store.revise(
                &repository,
                text(&args, "id")?,
                number(&args, "expected_revision", 0, i64::MAX as u64)?,
                RevisionRequest {
                    title: text(&args, "title")?.into(),
                    body: text(&args, "body")?.into(),
                    authority: serde_json::from_value(
                        args.get("authority")
                            .cloned()
                            .ok_or_else(|| invalid("authority required"))?,
                    )?,
                    links: serde_json::from_value(args.get("links").cloned().unwrap_or(json!([])))?,
                    metadata: args.get("metadata").cloned().unwrap_or(json!({})),
                },
            )?,
        )?),
        "cortex_graph" => Ok(serde_json::to_value(store.graph(GraphQuery {
            repository,
            id: text(&args, "id")?.into(),
            depth: number(&args, "depth", 1, 5)? as usize,
            limit,
        })?)?),
        "cortex_classification_enqueue" => Ok(serde_json::to_value(
            ClassificationQueue::new(store.clone()).enqueue(
                &repository,
                text(&args, "id")?,
                args.get("model")
                    .map(|_| text(&args, "model"))
                    .transpose()?
                    .unwrap_or(DEFAULT_MODEL),
            )?,
        )?),
        "cortex_classification_jobs" => Ok(serde_json::to_value(
            ClassificationQueue::new(store.clone()).list(&repository, limit)?,
        )?),
        _ => Err(invalid("unknown local tool")),
    }
}

pub fn run(store: LocalStore, default_repository: Option<String>) -> Result<()> {
    run_io(
        &store,
        default_repository.as_deref(),
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

/// Same framing path used by stdio and deterministic integration tests.
pub fn run_io(
    store: &LocalStore,
    repository: Option<&str>,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    loop {
        let mut bytes = Vec::new();
        (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if bytes.is_empty() {
            break;
        }
        if bytes.len() > MAX_FRAME {
            if !bytes.ends_with(b"\n") {
                loop {
                    let chunk = input.fill_buf()?;
                    if chunk.is_empty() {
                        break;
                    }
                    let end = chunk.iter().position(|b| *b == b'\n');
                    let count = end.map_or(chunk.len(), |n| n + 1);
                    input.consume(count);
                    if end.is_some() {
                        break;
                    }
                }
            }
            serde_json::to_writer(
                &mut output,
                &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"frame exceeds 4 MiB"}}),
            )?;
            output.write_all(b"\n")?;
            output.flush()?;
            continue;
        }
        let response = match serde_json::from_slice::<Value>(&bytes) {
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"invalid JSON"}}),
            ),
            Ok(request) => dispatch(store, repository, request),
        };
        if let Some(response) = response {
            let mut encoded = serde_json::to_vec(&response)?;
            if encoded.len() > MAX_FRAME {
                encoded = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":response["id"],"error":{"code":-32603,"message":"result exceeds transport bound; reduce limit"}}),
                )?;
            }
            output.write_all(&encoded)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn dispatch(store: &LocalStore, repository: Option<&str>, request: Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("jsonrpc") != Some(&json!("2.0"))
        || !request["method"].is_string()
        || !(id.is_null() || id.is_string() || id.is_i64() || id.is_u64())
    {
        return Some(
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid JSON-RPC request"}}),
        );
    }
    // Notifications never perform tool side effects.
    request.get("id")?;
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"cortex-local","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":descriptors()}),
        "tools/call" => {
            let params = &request["params"];
            match call(
                store,
                params["name"].as_str().unwrap_or(""),
                params.get("arguments").cloned().unwrap_or(json!({})),
                repository,
            ) {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                }
                Err(error) => {
                    json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                }
            }
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

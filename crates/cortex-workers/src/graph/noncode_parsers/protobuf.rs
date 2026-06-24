//! Protobuf parser — emits Schema / Service / Endpoint nodes and
//! DEFINES_SCHEMA / ROUTES edges.

use crate::graph::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::{artifact_key, entity_key, Parser};

/// Deterministic parser for `.proto` files.
pub struct ProtobufParser;

impl Parser for ProtobufParser {
    fn matches(&self, path: &str) -> bool {
        path.ends_with(".proto")
    }

    fn parse(&self, content: &str, repo: &str, path: &str, content_hash: &str) -> GraphPatch {
        let artifact_k = artifact_key(repo, path, content_hash);
        let mut patch = GraphPatch::default();

        patch
            .nodes
            .push(NodeOp::with_identity("Artifact", &artifact_k).with_props([
                ("repo", repo),
                ("path", path),
                ("content_hash", content_hash),
            ]));

        let mut current_service: Option<String> = None;

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip comments
            if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                continue;
            }

            if let Some(msg_name) = parse_proto_block_name(trimmed, "message") {
                let schema_k = entity_key(repo, path, &msg_name);
                patch
                    .nodes
                    .push(NodeOp::with_identity("Schema", &schema_k).with_props([
                        ("name", msg_name.as_str()),
                        ("kind", "message"),
                        ("repo", repo),
                        ("path", path),
                    ]));
                patch.edges.push(make_edge(
                    "DEFINES_SCHEMA",
                    "Artifact",
                    &artifact_k,
                    "Schema",
                    &schema_k,
                ));
                continue;
            }

            if let Some(svc_name) = parse_proto_block_name(trimmed, "service") {
                let service_k = entity_key(repo, path, &svc_name);
                patch
                    .nodes
                    .push(NodeOp::with_identity("Service", &service_k).with_props([
                        ("name", svc_name.as_str()),
                        ("repo", repo),
                        ("path", path),
                    ]));
                patch.edges.push(make_edge(
                    "DEFINES_SCHEMA",
                    "Artifact",
                    &artifact_k,
                    "Service",
                    &service_k,
                ));
                current_service = Some(svc_name);
                continue;
            }

            // Closing brace — end of current service block
            if trimmed == "}" {
                current_service = None;
                continue;
            }

            if let Some(rpc_name) = parse_rpc_name(trimmed) {
                let svc_name = match &current_service {
                    Some(s) => s.clone(),
                    None => "unknown_service".to_string(),
                };
                let endpoint_k = entity_key(repo, path, &format!("{svc_name}.{rpc_name}"));
                let service_k = entity_key(repo, path, &svc_name);
                patch
                    .nodes
                    .push(NodeOp::with_identity("Endpoint", &endpoint_k).with_props([
                        ("name", rpc_name.as_str()),
                        ("service", svc_name.as_str()),
                        ("repo", repo),
                        ("path", path),
                    ]));
                patch.edges.push(make_edge(
                    "ROUTES",
                    "Service",
                    &service_k,
                    "Endpoint",
                    &endpoint_k,
                ));
            }
        }

        patch
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_proto_block_name(line: &str, keyword: &str) -> Option<String> {
    let prefix = format!("{keyword} ");
    if !line.starts_with(&prefix) {
        return None;
    }
    let rest = line[prefix.len()..].trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn parse_rpc_name(line: &str) -> Option<String> {
    // rpc MethodName(Request) returns (Response) {}
    if !line.starts_with("rpc ") {
        return None;
    }
    let rest = &line[4..].trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn make_edge(
    edge_type: &str,
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
) -> EdgeOp {
    EdgeOp {
        edge_type: edge_type.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        ..Default::default()
    }
    .with_confidence(EdgeConfidence::Extracted, None)
}

trait NodeOpExt {
    fn with_props<'a>(self, props: impl IntoIterator<Item = (&'static str, &'a str)>) -> Self;
}

impl NodeOpExt for NodeOp {
    fn with_props<'a>(mut self, props: impl IntoIterator<Item = (&'static str, &'a str)>) -> Self {
        for (k, v) in props {
            self.props
                .insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
syntax = "proto3";

package cortex.api.v1;

// User message
message User {
    string id = 1;
    string email = 2;
    int64 created_at = 3;
}

message CreateUserRequest {
    string email = 1;
}

message CreateUserResponse {
    User user = 1;
}

service UserService {
    rpc GetUser(User) returns (User) {}
    rpc CreateUser(CreateUserRequest) returns (CreateUserResponse) {}
    rpc ListUsers(ListUsersRequest) returns (ListUsersResponse) {}
}
"#;

    fn parse_fixture() -> GraphPatch {
        ProtobufParser.parse(FIXTURE, "myrepo", "proto/user.proto", "protohash")
    }

    #[test]
    fn proto_includes_artifact_node() {
        let p = parse_fixture();
        assert!(p.nodes.iter().any(|n| n.label == "Artifact"));
    }

    #[test]
    fn proto_parses_message_as_schema() {
        let p = parse_fixture();
        let has_user = p
            .nodes
            .iter()
            .any(|n| n.label == "Schema" && n.natural_key.contains("User"));
        assert!(has_user, "should emit Schema node for message User");
    }

    #[test]
    fn proto_parses_service_node() {
        let p = parse_fixture();
        let has_svc = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("UserService"));
        assert!(has_svc, "should emit Service node for UserService");
    }

    #[test]
    fn proto_parses_rpc_as_endpoint() {
        let p = parse_fixture();
        let has_get_user = p
            .nodes
            .iter()
            .any(|n| n.label == "Endpoint" && n.natural_key.contains("GetUser"));
        assert!(has_get_user, "should emit Endpoint for rpc GetUser");
    }

    #[test]
    fn proto_emits_defines_schema_edges() {
        let p = parse_fixture();
        let count = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEFINES_SCHEMA")
            .count();
        // 3 messages + 1 service = 4 DEFINES_SCHEMA edges
        assert!(count >= 4, "expected ≥4 DEFINES_SCHEMA edges, got {count}");
    }

    #[test]
    fn proto_emits_routes_edges() {
        let p = parse_fixture();
        let count = p.edges.iter().filter(|e| e.edge_type == "ROUTES").count();
        assert_eq!(count, 3, "3 rpc methods → 3 ROUTES edges");
    }

    #[test]
    fn proto_edges_are_extracted_confidence() {
        let p = parse_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(conf, Some("extracted"), "all proto edges must be extracted");
        }
    }
}

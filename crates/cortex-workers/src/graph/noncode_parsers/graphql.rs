//! GraphQL parser — emits Schema / Endpoint nodes and
//! DEFINES_SCHEMA / ROUTES edges.

use crate::graph::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::{artifact_key, entity_key, Parser};

/// Deterministic parser for `.graphql` / `.gql` files.
pub struct GraphQlParser;

/// Top-level GraphQL type names that are treated as Endpoint containers
/// (their fields map to Endpoints).
const OPERATION_TYPES: &[&str] = &["Query", "Mutation", "Subscription"];

impl Parser for GraphQlParser {
    fn matches(&self, path: &str) -> bool {
        path.ends_with(".graphql") || path.ends_with(".gql")
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

        let mut current_type: Option<(String, bool)> = None; // (name, is_operation)
        let mut brace_depth: i32 = 0;

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip comments and blank lines
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }

            // Count brace depth changes
            let open_count = trimmed.chars().filter(|&c| c == '{').count() as i32;
            let close_count = trimmed.chars().filter(|&c| c == '}').count() as i32;

            // Detect type/interface/union/enum/input at depth 0
            if brace_depth == 0 {
                if let Some(name) = parse_type_def(trimmed) {
                    let is_op = OPERATION_TYPES.contains(&name.as_str());
                    let schema_k = entity_key(repo, path, &name);
                    patch
                        .nodes
                        .push(NodeOp::with_identity("Schema", &schema_k).with_props([
                            ("name", name.as_str()),
                            ("kind", if is_op { "operation" } else { "type" }),
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
                    if open_count > close_count {
                        current_type = Some((name, is_op));
                    }
                }
            } else if brace_depth == 1 {
                // Inside a type block — emit Endpoint for operation type fields
                if let Some((ref type_name, true)) = current_type {
                    if let Some(field_name) = parse_field_name(trimmed) {
                        let endpoint_k =
                            entity_key(repo, path, &format!("{type_name}.{field_name}"));
                        let schema_k = entity_key(repo, path, type_name);
                        patch.nodes.push(
                            NodeOp::with_identity("Endpoint", &endpoint_k).with_props([
                                ("name", field_name.as_str()),
                                ("operation_type", type_name.as_str()),
                                ("repo", repo),
                                ("path", path),
                            ]),
                        );
                        patch.edges.push(make_edge(
                            "ROUTES",
                            "Schema",
                            &schema_k,
                            "Endpoint",
                            &endpoint_k,
                        ));
                    }
                }
            }

            brace_depth += open_count - close_count;
            if brace_depth <= 0 {
                brace_depth = 0;
                current_type = None;
            }
        }

        patch
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_type_def(line: &str) -> Option<String> {
    for keyword in &["type ", "interface ", "union ", "enum ", "input "] {
        if let Some(stripped) = line.strip_prefix(keyword) {
            let rest = stripped.trim_start();
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn parse_field_name(line: &str) -> Option<String> {
    // Field lines look like: `fieldName(args): ReturnType`
    // or `fieldName: ReturnType`
    // Skip closing braces, directives, and empty lines
    let trimmed = line.trim();
    if trimmed.starts_with('}') || trimmed.starts_with('#') || trimmed.is_empty() {
        return None;
    }
    let name: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || name == "}" {
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
# Cortex API Schema

type User {
    id: ID!
    email: String!
    createdAt: DateTime!
}

type Session {
    id: ID!
    userId: String!
    turns: [Turn!]!
}

interface Node {
    id: ID!
}

enum EventKind {
    TURN
    DECISION
    TOOL_CALL
}

type Query {
    user(id: ID!): User
    session(id: ID!): Session
    timeline(sessionId: ID!): [Event!]!
}

type Mutation {
    createUser(email: String!): User!
    captureDecision(title: String!, body: String!): Decision!
}

type Subscription {
    eventStream(sessionId: ID!): Event!
}
"#;

    fn parse_fixture() -> GraphPatch {
        GraphQlParser.parse(FIXTURE, "myrepo", "api/schema.graphql", "gqlhash")
    }

    #[test]
    fn graphql_includes_artifact_node() {
        let p = parse_fixture();
        assert!(p.nodes.iter().any(|n| n.label == "Artifact"));
    }

    #[test]
    fn graphql_parses_type_as_schema() {
        let p = parse_fixture();
        let has_user = p
            .nodes
            .iter()
            .any(|n| n.label == "Schema" && n.natural_key.contains("User"));
        assert!(has_user, "should emit Schema node for type User");
    }

    #[test]
    fn graphql_parses_interface_as_schema() {
        let p = parse_fixture();
        let has_node = p
            .nodes
            .iter()
            .any(|n| n.label == "Schema" && n.natural_key.contains("Node"));
        assert!(has_node, "should emit Schema node for interface Node");
    }

    #[test]
    fn graphql_parses_query_fields_as_endpoints() {
        let p = parse_fixture();
        let has_user_ep = p
            .nodes
            .iter()
            .any(|n| n.label == "Endpoint" && n.natural_key.contains("Query.user"));
        assert!(has_user_ep, "should emit Endpoint for Query.user");
    }

    #[test]
    fn graphql_parses_mutation_fields_as_endpoints() {
        let p = parse_fixture();
        let has_create = p
            .nodes
            .iter()
            .any(|n| n.label == "Endpoint" && n.natural_key.contains("Mutation.createUser"));
        assert!(has_create, "should emit Endpoint for Mutation.createUser");
    }

    #[test]
    fn graphql_parses_subscription_fields_as_endpoints() {
        let p = parse_fixture();
        let has_sub = p
            .nodes
            .iter()
            .any(|n| n.label == "Endpoint" && n.natural_key.contains("Subscription.eventStream"));
        assert!(has_sub, "should emit Endpoint for Subscription.eventStream");
    }

    #[test]
    fn graphql_emits_defines_schema_edges() {
        let p = parse_fixture();
        let count = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEFINES_SCHEMA")
            .count();
        // User, Session, Node (interface), EventKind (enum), Query, Mutation, Subscription
        assert!(count >= 6, "expected ≥6 DEFINES_SCHEMA edges, got {count}");
    }

    #[test]
    fn graphql_emits_routes_edges() {
        let p = parse_fixture();
        let count = p.edges.iter().filter(|e| e.edge_type == "ROUTES").count();
        // 3 Query + 2 Mutation + 1 Subscription = 6
        assert_eq!(count, 6, "expected 6 ROUTES edges, got {count}");
    }

    #[test]
    fn graphql_edges_are_extracted_confidence() {
        let p = parse_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(conf, Some("extracted"), "all gql edges must be extracted");
        }
    }
}

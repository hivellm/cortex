//! SQL parser — emits SchemaTable / Schema nodes and
//! DEFINES_SCHEMA / MIGRATES / READS_FROM / WRITES_TO edges.

use crate::graph::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::{artifact_key, entity_key, Parser};

/// Deterministic parser for `.sql` files.
pub struct SqlParser;

impl Parser for SqlParser {
    fn matches(&self, path: &str) -> bool {
        path.ends_with(".sql")
    }

    fn parse(&self, content: &str, repo: &str, path: &str, content_hash: &str) -> GraphPatch {
        let artifact_k = artifact_key(repo, path, content_hash);
        let mut patch = GraphPatch::default();

        // Always include the Artifact node as anchor.
        patch
            .nodes
            .push(NodeOp::with_identity("Artifact", &artifact_k).with_props([
                ("repo", repo),
                ("path", path),
                ("content_hash", content_hash),
            ]));

        let is_migration = is_migration_file(path, content);

        for line in content.lines() {
            let trimmed = line.trim();
            let upper = trimmed.to_ascii_uppercase();

            // CREATE TABLE / CREATE TABLE IF NOT EXISTS
            if let Some(name) = parse_create_table(&upper, trimmed) {
                let table_k = entity_key(repo, path, &name);
                patch
                    .nodes
                    .push(NodeOp::with_identity("SchemaTable", &table_k).with_props([
                        ("name", name.as_str()),
                        ("repo", repo),
                        ("path", path),
                    ]));
                patch.edges.push(make_edge(
                    "DEFINES_SCHEMA",
                    "Artifact",
                    &artifact_k,
                    "SchemaTable",
                    &table_k,
                ));
                if is_migration {
                    patch.edges.push(make_edge(
                        "MIGRATES",
                        "Artifact",
                        &artifact_k,
                        "SchemaTable",
                        &table_k,
                    ));
                }
                continue;
            }

            // CREATE SCHEMA
            if let Some(name) = parse_create_schema(&upper, trimmed) {
                let schema_k = entity_key(repo, path, &name);
                patch
                    .nodes
                    .push(NodeOp::with_identity("Schema", &schema_k).with_props([
                        ("name", name.as_str()),
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

            // INSERT INTO / UPDATE / DELETE FROM — WRITES_TO
            if let Some(name) = parse_write_target(&upper, trimmed) {
                let table_k = entity_key(repo, path, &name);
                ensure_table_node(&mut patch, repo, path, &table_k, &name);
                patch.edges.push(make_edge(
                    "WRITES_TO",
                    "Artifact",
                    &artifact_k,
                    "SchemaTable",
                    &table_k,
                ));
                continue;
            }

            // SELECT … FROM — READS_FROM
            if let Some(name) = parse_read_target(&upper, trimmed) {
                let table_k = entity_key(repo, path, &name);
                ensure_table_node(&mut patch, repo, path, &table_k, &name);
                patch.edges.push(make_edge(
                    "READS_FROM",
                    "Artifact",
                    &artifact_k,
                    "SchemaTable",
                    &table_k,
                ));
            }
        }

        patch
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/// Ensure a SchemaTable node exists in the patch (dedup by key).
fn ensure_table_node(patch: &mut GraphPatch, repo: &str, path: &str, key: &str, name: &str) {
    if !patch.nodes.iter().any(|n| n.natural_key == key) {
        patch
            .nodes
            .push(NodeOp::with_identity("SchemaTable", key).with_props([
                ("name", name),
                ("repo", repo),
                ("path", path),
            ]));
    }
}

fn is_migration_file(path: &str, content: &str) -> bool {
    let path_lower = path.to_ascii_lowercase();
    let content_upper = content.to_ascii_uppercase();
    path_lower.contains("migration")
        || path_lower.contains("migrate")
        || content_upper.contains("ALTER TABLE")
        || content_upper.contains("DROP TABLE")
        || content_upper.contains("DROP COLUMN")
        || content_upper.contains("ADD COLUMN")
        || content_upper.contains("RENAME COLUMN")
}

fn parse_create_table(upper: &str, original: &str) -> Option<String> {
    // CREATE TABLE [IF NOT EXISTS] name
    let rest = if upper.starts_with("CREATE TABLE IF NOT EXISTS ") {
        &original["CREATE TABLE IF NOT EXISTS ".len()..]
    } else if upper.starts_with("CREATE TABLE ") {
        &original["CREATE TABLE ".len()..]
    } else {
        return None;
    };
    Some(first_identifier(rest))
}

fn parse_create_schema(upper: &str, original: &str) -> Option<String> {
    let rest = if upper.starts_with("CREATE SCHEMA IF NOT EXISTS ") {
        &original["CREATE SCHEMA IF NOT EXISTS ".len()..]
    } else if upper.starts_with("CREATE SCHEMA ") {
        &original["CREATE SCHEMA ".len()..]
    } else {
        return None;
    };
    Some(first_identifier(rest))
}

fn parse_write_target(upper: &str, original: &str) -> Option<String> {
    // INSERT INTO name
    if upper.starts_with("INSERT INTO ") {
        let rest = &original["INSERT INTO ".len()..];
        return Some(first_identifier(rest));
    }
    // UPDATE name
    if upper.starts_with("UPDATE ") {
        let rest = &original["UPDATE ".len()..];
        return Some(first_identifier(rest));
    }
    // DELETE FROM name
    if upper.starts_with("DELETE FROM ") {
        let rest = &original["DELETE FROM ".len()..];
        return Some(first_identifier(rest));
    }
    None
}

fn parse_read_target(upper: &str, original: &str) -> Option<String> {
    // FROM on its own line (multi-line SELECT)
    if upper.starts_with("FROM ") {
        let rest = &original["FROM ".len()..];
        let name = first_identifier(rest);
        if !name.is_empty() && name != "(" {
            return Some(name);
        }
    }
    // FROM within a single-line SELECT ... FROM table
    if upper.contains("SELECT") {
        if let Some(pos) = upper.find(" FROM ") {
            let rest = &original[pos + " FROM ".len()..];
            let name = first_identifier(rest);
            if !name.is_empty() && name != "(" {
                return Some(name);
            }
        }
    }
    None
}

/// Extract the first SQL identifier from a string: strip leading whitespace,
/// strip optional backtick/double-quote delimiters, take until whitespace or `(`.
fn first_identifier(s: &str) -> String {
    let s = s.trim_start();
    let s = s.trim_start_matches('`').trim_start_matches('"');
    let end = s
        .find(|c: char| c.is_whitespace() || c == '(' || c == ';' || c == '`' || c == '"')
        .unwrap_or(s.len());
    s[..end].to_ascii_lowercase()
}

// ── NodeOp builder helper used locally ────────────────────────────────────────

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

    // Golden fixture: a small SQL file with CREATE TABLE, INSERT, SELECT, and ALTER.
    const FIXTURE: &str = r#"
CREATE SCHEMA IF NOT EXISTS public;

CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES users(id),
    total NUMERIC(10,2)
);

INSERT INTO orders (user_id, total) VALUES (1, 99.99);

SELECT u.email, o.total
FROM users u
JOIN orders o ON u.id = o.user_id
WHERE u.id = 1;

ALTER TABLE users ADD COLUMN display_name TEXT;
"#;

    fn parse_fixture() -> GraphPatch {
        SqlParser.parse(FIXTURE, "myrepo", "db/schema.sql", "abc123")
    }

    #[test]
    fn sql_parses_create_schema() {
        let p = parse_fixture();
        let has_schema = p
            .nodes
            .iter()
            .any(|n| n.label == "Schema" && n.natural_key.contains("public"));
        assert!(has_schema, "should emit Schema node for CREATE SCHEMA");
    }

    #[test]
    fn sql_parses_create_table_users() {
        let p = parse_fixture();
        let has_users = p
            .nodes
            .iter()
            .any(|n| n.label == "SchemaTable" && n.natural_key.contains("users"));
        assert!(has_users, "should emit SchemaTable node for users");
    }

    #[test]
    fn sql_parses_create_table_orders() {
        let p = parse_fixture();
        let has_orders = p
            .nodes
            .iter()
            .any(|n| n.label == "SchemaTable" && n.natural_key.contains("orders"));
        assert!(has_orders, "should emit SchemaTable node for orders");
    }

    #[test]
    fn sql_emits_defines_schema_edge() {
        let p = parse_fixture();
        let has_edge = p
            .edges
            .iter()
            .any(|e| e.edge_type == "DEFINES_SCHEMA" && e.from_label == "Artifact");
        assert!(has_edge, "should emit DEFINES_SCHEMA edge from Artifact");
    }

    #[test]
    fn sql_emits_writes_to_edge() {
        let p = parse_fixture();
        let has_edge = p.edges.iter().any(|e| e.edge_type == "WRITES_TO");
        assert!(has_edge, "should emit WRITES_TO edge for INSERT INTO");
    }

    #[test]
    fn sql_emits_reads_from_edge() {
        let p = parse_fixture();
        let has_edge = p.edges.iter().any(|e| e.edge_type == "READS_FROM");
        assert!(has_edge, "should emit READS_FROM edge for SELECT...FROM");
    }

    #[test]
    fn sql_emits_migrates_edge_for_alter() {
        let p = parse_fixture();
        let has_edge = p.edges.iter().any(|e| e.edge_type == "MIGRATES");
        assert!(has_edge, "ALTER TABLE should trigger MIGRATES edge");
    }

    #[test]
    fn sql_always_includes_artifact_node() {
        let p = parse_fixture();
        let has_artifact = p.nodes.iter().any(|n| n.label == "Artifact");
        assert!(has_artifact, "patch must include the Artifact node");
    }

    #[test]
    fn sql_edges_are_extracted_confidence() {
        let p = parse_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(
                conf,
                Some("extracted"),
                "all SQL edges must have extracted confidence, got {edge:?}"
            );
        }
    }
}

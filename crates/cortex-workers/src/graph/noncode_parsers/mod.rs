//! Phase23d — pluggable non-code parser registry.
//!
//! Infrastructure and data files (`.sql`, `.tf`, `.proto`, `.graphql`,
//! `Dockerfile`) are dispatched through a first-match-wins registry that
//! emits [`GraphPatch`] entries using the phase23a ontology vocabulary
//! (node labels + edge types). All output passes through the phase23c
//! reconciliation gate before graph upsert.
//!
//! Code files (`.rs`, `.ts`, etc.) return `None` from [`ParserRegistry::dispatch`]
//! and fall back to the existing tree-sitter extractor.

pub mod dockerfile;
pub mod graphql;
pub mod protobuf;
pub mod sql;
pub mod terraform;

use crate::graph::patch::GraphPatch;

// ── Parser trait ─────────────────────────────────────────────────────────────

/// Deterministic parser for a single non-code file type.
///
/// Implementations are expected to be `Send + Sync` (they live in a shared
/// registry) and produce only [`crate::graph::patch::EdgeConfidence::Extracted`]
/// edges — no LLM inference happens here.
pub trait Parser: Send + Sync {
    /// Return `true` if this parser handles `path`.
    /// Matching is done by extension or filename only (no content sniffing).
    fn matches(&self, path: &str) -> bool;

    /// Parse `content` and emit a [`GraphPatch`] using `repo`, `path`, and
    /// `content_hash` to form canonical natural-key triplets
    /// (`{repo}|{path}|{name}`).
    ///
    /// The Artifact node for the file itself is always included so the
    /// reconciliation gate has a fact-set anchor.
    fn parse(&self, content: &str, repo: &str, path: &str, content_hash: &str) -> GraphPatch;
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// First-match-wins registry of non-code parsers.
///
/// [`ParserRegistry::dispatch`] returns `Some(patch)` for files handled by a
/// non-code parser and `None` for everything else (code files fall through to
/// the tree-sitter extractor).
pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser>>,
}

impl ParserRegistry {
    /// Build the registry with all built-in parsers in priority order.
    #[must_use]
    pub fn new() -> Self {
        let mut r = ParserRegistry { parsers: vec![] };
        r.register(Box::new(sql::SqlParser));
        r.register(Box::new(terraform::TerraformParser));
        r.register(Box::new(protobuf::ProtobufParser));
        r.register(Box::new(graphql::GraphQlParser));
        r.register(Box::new(dockerfile::DockerfileParser));
        r
    }

    /// Register an additional parser at lowest priority.
    pub fn register(&mut self, parser: Box<dyn Parser>) {
        self.parsers.push(parser);
    }

    /// Dispatch `content` to the first matching parser.
    ///
    /// Returns `None` when no parser matches (callers fall back to the
    /// existing code extractor).
    pub fn dispatch(
        &self,
        content: &str,
        repo: &str,
        path: &str,
        content_hash: &str,
    ) -> Option<GraphPatch> {
        for parser in &self.parsers {
            if parser.matches(path) {
                return Some(parser.parse(content, repo, path, content_hash));
            }
        }
        None
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Build the canonical Artifact natural-key `{repo}|{path}|{content_hash}`.
pub(super) fn artifact_key(repo: &str, path: &str, content_hash: &str) -> String {
    format!("{repo}|{path}|{content_hash}")
}

/// Build a three-part entity natural-key `{repo}|{path}|{name}` for any
/// non-code node (SchemaTable, InfraResource, Schema, Service, Config, Endpoint).
pub(super) fn entity_key(repo: &str, path: &str, name: &str) -> String {
    format!("{repo}|{path}|{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_dispatches_sql() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch("CREATE TABLE t (id INT);", "repo", "db/schema.sql", "h1");
        assert!(patch.is_some(), "sql should be dispatched");
    }

    #[test]
    fn registry_dispatches_tf() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch(
            r#"resource "aws_s3_bucket" "b" {}"#,
            "repo",
            "infra/main.tf",
            "h2",
        );
        assert!(patch.is_some(), "terraform should be dispatched");
    }

    #[test]
    fn registry_dispatches_proto() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch(
            "message Foo { string id = 1; }",
            "repo",
            "proto/svc.proto",
            "h3",
        );
        assert!(patch.is_some(), "proto should be dispatched");
    }

    #[test]
    fn registry_dispatches_graphql() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch(
            "type Query { hello: String }",
            "repo",
            "api/schema.graphql",
            "h4",
        );
        assert!(patch.is_some(), "graphql should be dispatched");
    }

    #[test]
    fn registry_dispatches_dockerfile() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch("FROM ubuntu:22.04\nEXPOSE 8080", "repo", "Dockerfile", "h5");
        assert!(patch.is_some(), "dockerfile should be dispatched");
    }

    #[test]
    fn registry_returns_none_for_rust_source() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch("fn main() {}", "repo", "src/main.rs", "h6");
        assert!(
            patch.is_none(),
            "rust source must fall back to code extractor"
        );
    }

    #[test]
    fn registry_returns_none_for_ts_source() {
        let reg = ParserRegistry::new();
        let patch = reg.dispatch("const x = 1;", "repo", "src/index.ts", "h7");
        assert!(
            patch.is_none(),
            "ts source must fall back to code extractor"
        );
    }
}

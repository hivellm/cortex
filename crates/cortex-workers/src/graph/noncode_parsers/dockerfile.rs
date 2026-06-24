//! Dockerfile parser — emits Config / Service nodes and
//! DEPLOYS / DEPENDS_ON edges.

use crate::graph::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::{artifact_key, entity_key, Parser};

/// Deterministic parser for `Dockerfile`, `Dockerfile.*`, and `*.dockerfile` files.
pub struct DockerfileParser;

impl Parser for DockerfileParser {
    fn matches(&self, path: &str) -> bool {
        let filename = path.rsplit('/').next().unwrap_or(path);
        filename == "Dockerfile"
            || filename.starts_with("Dockerfile.")
            || path.ends_with(".dockerfile")
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

        // The Dockerfile itself becomes a Config node
        let config_k = entity_key(repo, path, "config");
        patch
            .nodes
            .push(NodeOp::with_identity("Config", &config_k).with_props([
                ("file", path),
                ("repo", repo),
                ("path", path),
            ]));
        patch.edges.push(make_edge(
            "DEPLOYS",
            "Artifact",
            &artifact_k,
            "Config",
            &config_k,
        ));

        let mut stage_index: u32 = 0;

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let upper = trimmed.to_ascii_uppercase();

            // FROM image[:tag] [AS alias]
            if upper.starts_with("FROM ") {
                let rest = &trimmed["FROM ".len()..].trim_start();
                // Extract image name (before optional AS)
                let image = extract_from_image(rest);
                if !image.is_empty() && image != "scratch" {
                    let stage_name =
                        extract_as_alias(rest).unwrap_or_else(|| format!("stage_{stage_index}"));
                    let service_k = entity_key(repo, path, &stage_name);
                    patch
                        .nodes
                        .push(NodeOp::with_identity("Service", &service_k).with_props([
                            ("name", stage_name.as_str()),
                            ("image", image.as_str()),
                            ("repo", repo),
                            ("path", path),
                        ]));
                    patch.edges.push(make_edge(
                        "DEPENDS_ON",
                        "Config",
                        &config_k,
                        "Service",
                        &service_k,
                    ));
                }
                stage_index += 1;
                continue;
            }

            // EXPOSE port — each exposed port becomes a Service endpoint
            if upper.starts_with("EXPOSE ") {
                let ports = trimmed["EXPOSE ".len()..].trim();
                for port in ports.split_whitespace() {
                    let port_clean = port.split('/').next().unwrap_or(port);
                    let endpoint_name = format!("port_{port_clean}");
                    let service_k = entity_key(repo, path, &endpoint_name);
                    patch
                        .nodes
                        .push(NodeOp::with_identity("Service", &service_k).with_props([
                            ("name", endpoint_name.as_str()),
                            ("port", port_clean),
                            ("repo", repo),
                            ("path", path),
                        ]));
                    patch.edges.push(make_edge(
                        "DEPLOYS", "Config", &config_k, "Service", &service_k,
                    ));
                }
            }
        }

        patch
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_from_image(rest: &str) -> String {
    // image is the first token before optional " AS alias"
    let upper = rest.to_ascii_uppercase();
    if let Some(as_pos) = upper.find(" AS ") {
        rest[..as_pos].trim().to_string()
    } else {
        rest.split_whitespace().next().unwrap_or("").to_string()
    }
}

fn extract_as_alias(rest: &str) -> Option<String> {
    let upper = rest.to_ascii_uppercase();
    let as_pos = upper.find(" AS ")?;
    let after = rest[as_pos + " AS ".len()..].trim();
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
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
# Build stage
FROM rust:1.78-alpine AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y libssl3 ca-certificates
COPY --from=builder /app/target/release/cortex-workers /usr/local/bin/
EXPOSE 8080 9090
CMD ["/usr/local/bin/cortex-workers"]
"#;

    fn parse_fixture() -> GraphPatch {
        DockerfileParser.parse(FIXTURE, "myrepo", "Dockerfile", "dockrhash")
    }

    #[test]
    fn dockerfile_includes_artifact_node() {
        let p = parse_fixture();
        assert!(p.nodes.iter().any(|n| n.label == "Artifact"));
    }

    #[test]
    fn dockerfile_includes_config_node() {
        let p = parse_fixture();
        assert!(
            p.nodes.iter().any(|n| n.label == "Config"),
            "should emit Config node for the Dockerfile itself"
        );
    }

    #[test]
    fn dockerfile_emits_deploys_from_artifact_to_config() {
        let p = parse_fixture();
        let has_edge = p
            .edges
            .iter()
            .any(|e| e.edge_type == "DEPLOYS" && e.from_label == "Artifact");
        assert!(has_edge, "should emit DEPLOYS from Artifact to Config");
    }

    #[test]
    fn dockerfile_parses_from_as_service() {
        let p = parse_fixture();
        let has_builder = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("builder"));
        assert!(has_builder, "should emit Service for FROM...AS builder");
    }

    #[test]
    fn dockerfile_parses_runtime_stage() {
        let p = parse_fixture();
        let has_runtime = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("runtime"));
        assert!(has_runtime, "should emit Service for FROM...AS runtime");
    }

    #[test]
    fn dockerfile_parses_exposed_ports_as_services() {
        let p = parse_fixture();
        let has_8080 = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("port_8080"));
        let has_9090 = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("port_9090"));
        assert!(has_8080, "should emit Service for EXPOSE 8080");
        assert!(has_9090, "should emit Service for EXPOSE 9090");
    }

    #[test]
    fn dockerfile_emits_depends_on_for_from() {
        let p = parse_fixture();
        let count = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEPENDS_ON")
            .count();
        // 2 FROM stages (rust:1.78, debian:bookworm) → 2 DEPENDS_ON edges
        assert_eq!(count, 2, "two FROM stages → two DEPENDS_ON edges");
    }

    #[test]
    fn dockerfile_emits_deploys_for_exposed_ports() {
        let p = parse_fixture();
        let deploys_from_config = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEPLOYS" && e.from_label == "Config")
            .count();
        // EXPOSE 8080 9090 → 2 DEPLOYS edges from Config
        assert_eq!(
            deploys_from_config, 2,
            "two EXPOSE ports → two DEPLOYS edges"
        );
    }

    #[test]
    fn dockerfile_edges_are_extracted_confidence() {
        let p = parse_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(
                conf,
                Some("extracted"),
                "all dockerfile edges must be extracted"
            );
        }
    }

    #[test]
    fn dockerfile_matches_dockerfile_variant_names() {
        let parser = DockerfileParser;
        assert!(parser.matches("Dockerfile"));
        assert!(parser.matches("Dockerfile.prod"));
        assert!(parser.matches("services/api/Dockerfile"));
        assert!(parser.matches("app.dockerfile"));
        assert!(!parser.matches("src/main.rs"));
        assert!(!parser.matches("docker-compose.yml"));
    }
}

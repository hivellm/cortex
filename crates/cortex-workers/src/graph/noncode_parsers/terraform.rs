//! Terraform parser — emits InfraResource / Service nodes and
//! PROVISIONS / DEPENDS_ON edges.

use crate::graph::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::{artifact_key, entity_key, Parser};

/// Deterministic parser for `.tf` / `.tfvars` files.
pub struct TerraformParser;

impl Parser for TerraformParser {
    fn matches(&self, path: &str) -> bool {
        path.ends_with(".tf") || path.ends_with(".tfvars")
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

        for block in iter_blocks(content) {
            match block.kind.as_str() {
                "resource" => {
                    if let (Some(rtype), Some(rname)) = (block.type_label, block.name_label) {
                        let full_name = format!("{rtype}.{rname}");
                        let resource_k = entity_key(repo, path, &full_name);
                        patch.nodes.push(
                            NodeOp::with_identity("InfraResource", &resource_k).with_props([
                                ("resource_type", rtype.as_str()),
                                ("name", rname.as_str()),
                                ("repo", repo),
                                ("path", path),
                            ]),
                        );
                        patch.edges.push(make_edge(
                            "PROVISIONS",
                            "Artifact",
                            &artifact_k,
                            "InfraResource",
                            &resource_k,
                        ));
                    }
                }
                "module" => {
                    if let Some(mname) = block.name_label {
                        let service_k = entity_key(repo, path, &mname);
                        patch
                            .nodes
                            .push(NodeOp::with_identity("Service", &service_k).with_props([
                                ("name", mname.as_str()),
                                ("repo", repo),
                                ("path", path),
                            ]));
                        patch.edges.push(make_edge(
                            "DEPENDS_ON",
                            "Artifact",
                            &artifact_k,
                            "Service",
                            &service_k,
                        ));
                    }
                }
                "data" => {
                    // data sources are read-only InfraResources
                    if let (Some(rtype), Some(rname)) = (block.type_label, block.name_label) {
                        let full_name = format!("data.{rtype}.{rname}");
                        let resource_k = entity_key(repo, path, &full_name);
                        patch.nodes.push(
                            NodeOp::with_identity("InfraResource", &resource_k).with_props([
                                ("resource_type", rtype.as_str()),
                                ("name", rname.as_str()),
                                ("kind", "data"),
                                ("repo", repo),
                                ("path", path),
                            ]),
                        );
                        patch.edges.push(make_edge(
                            "DEPENDS_ON",
                            "Artifact",
                            &artifact_k,
                            "InfraResource",
                            &resource_k,
                        ));
                    }
                }
                _ => {}
            }
        }

        patch
    }
}

// ── Block iterator ────────────────────────────────────────────────────────────

struct TfBlock {
    kind: String,
    type_label: Option<String>,
    name_label: Option<String>,
}

/// Iterate top-level HCL blocks by scanning for lines that match
/// `<keyword> "<type>" "<name>" {` or `<keyword> "<name>" {`.
fn iter_blocks(content: &str) -> Vec<TfBlock> {
    let mut blocks = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with('{') && !trimmed.contains('{') {
            continue;
        }
        let parts: Vec<&str> = trimmed
            .split_whitespace()
            .filter(|p| !p.starts_with('#') && !p.starts_with("//"))
            .collect();
        if parts.is_empty() {
            continue;
        }
        let kind = parts[0].to_string();
        // We only care about top-level blocks; skip locals/variable/output for now
        match kind.as_str() {
            "resource" | "data" => {
                // resource "type" "name" {
                let type_label = parts.get(1).map(|s| unquote(s));
                let name_label = parts.get(2).map(|s| unquote(s));
                if type_label.as_deref().is_some_and(|t| !t.is_empty())
                    && name_label.as_deref().is_some_and(|n| !n.is_empty())
                    && name_label.as_deref() != Some("{")
                {
                    blocks.push(TfBlock {
                        kind,
                        type_label,
                        name_label,
                    });
                }
            }
            "module" => {
                // module "name" {
                let name_label = parts.get(1).map(|s| unquote(s));
                if name_label
                    .as_deref()
                    .is_some_and(|n| !n.is_empty() && n != "{")
                {
                    blocks.push(TfBlock {
                        kind,
                        type_label: None,
                        name_label,
                    });
                }
            }
            _ => {}
        }
    }
    blocks
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').to_string()
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
terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

resource "aws_s3_bucket" "assets" {
  bucket = "my-assets-bucket"
  acl    = "private"
}

resource "aws_lambda_function" "api_handler" {
  function_name = "api-handler"
  runtime       = "provided.al2"
  handler       = "bootstrap"
}

data "aws_iam_policy_document" "assume_role" {
  statement {
    actions = ["sts:AssumeRole"]
  }
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "5.0"
  name    = "main-vpc"
}
"#;

    fn parse_fixture() -> GraphPatch {
        TerraformParser.parse(FIXTURE, "myrepo", "infra/main.tf", "tfhash")
    }

    #[test]
    fn tf_always_includes_artifact_node() {
        let p = parse_fixture();
        assert!(p.nodes.iter().any(|n| n.label == "Artifact"));
    }

    #[test]
    fn tf_parses_s3_bucket_resource() {
        let p = parse_fixture();
        let has_node = p
            .nodes
            .iter()
            .any(|n| n.label == "InfraResource" && n.natural_key.contains("aws_s3_bucket.assets"));
        assert!(has_node, "should emit InfraResource for aws_s3_bucket");
    }

    #[test]
    fn tf_parses_lambda_resource() {
        let p = parse_fixture();
        let has_node = p.nodes.iter().any(|n| {
            n.label == "InfraResource" && n.natural_key.contains("aws_lambda_function.api_handler")
        });
        assert!(
            has_node,
            "should emit InfraResource for aws_lambda_function"
        );
    }

    #[test]
    fn tf_parses_data_source() {
        let p = parse_fixture();
        let has_node = p.nodes.iter().any(|n| {
            n.label == "InfraResource" && n.natural_key.contains("data.aws_iam_policy_document")
        });
        assert!(has_node, "should emit InfraResource for data source");
    }

    #[test]
    fn tf_parses_module() {
        let p = parse_fixture();
        let has_node = p
            .nodes
            .iter()
            .any(|n| n.label == "Service" && n.natural_key.contains("vpc"));
        assert!(has_node, "should emit Service node for module block");
    }

    #[test]
    fn tf_emits_provisions_edges() {
        let p = parse_fixture();
        let count = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "PROVISIONS")
            .count();
        assert_eq!(count, 2, "two resource blocks → two PROVISIONS edges");
    }

    #[test]
    fn tf_emits_depends_on_edges() {
        let p = parse_fixture();
        let count = p
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEPENDS_ON")
            .count();
        // 1 data source + 1 module = 2
        assert_eq!(count, 2, "data source + module → two DEPENDS_ON edges");
    }

    #[test]
    fn tf_edges_are_extracted_confidence() {
        let p = parse_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(conf, Some("extracted"), "all tf edges must be extracted");
        }
    }
}

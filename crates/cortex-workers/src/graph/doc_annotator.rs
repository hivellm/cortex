//! Phase23e §2 — Gated LLM annotation pass for the docs-as-graph pipeline.
//!
//! `DocAnnotator` calls the local `claude` CLI with an article-analysis
//! prompt (Understand-Anything article-analyzer style), parses the JSON
//! output, and runs the result through the phase23c reconciliation gate
//! before merging it into the doc-pass [`GraphPatch`].
//!
//! Contract guarantees (enforced by the gate):
//! - `Claim` nodes are [`EdgeConfidence::Inferred`] and may only reference
//!   `Article` endpoints proven to exist by Phase 1 (the `FactSet`).
//! - Implicit edges (`BUILDS_ON`, `CONTRADICTS`, `CITES`, `EXEMPLIFIES`,
//!   `CATEGORIZED_UNDER`) require explicit textual evidence stored in the
//!   edge `description`.
//! - Duplicate wikilink `RELATED` edges are not re-emitted.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::graph::extraction_contract::{apply_gate, FactSet, GateConfig};
use crate::graph::patch::{ConflictPolicy, EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

use super::doc_pass::{article_key, claim_key};

// ── LLM output schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnnotationOutput {
    #[serde(default)]
    claims: Vec<ClaimRecord>,
    #[serde(default)]
    relations: Vec<RelationRecord>,
}

#[derive(Debug, Deserialize)]
struct ClaimRecord {
    text: String,
    #[serde(rename = "type", default)]
    claim_type: String,
}

#[derive(Debug, Deserialize)]
struct RelationRecord {
    /// Natural key of the target document (path portion).
    to_doc: String,
    /// One of: builds_on | contradicts | cites | exemplifies | categorized_under
    kind: String,
    /// Verbatim quote or paraphrase that supports the relation.
    evidence: String,
}

// ── DocAnnotator ─────────────────────────────────────────────────────────────

/// Phase23e §2 LLM annotation pass.
///
/// Calls the local `claude` CLI, parses the structured annotation output,
/// and merges it (after gate filtering) into an existing [`GraphPatch`].
pub struct DocAnnotator {
    /// Path to the `claude` binary (usually just `"claude"`).
    pub claude_bin: String,
    /// Timeout for the LLM call.
    pub timeout: Duration,
    /// Gate configuration (significance threshold etc.).
    pub gate_config: GateConfig,
}

impl Default for DocAnnotator {
    fn default() -> Self {
        Self {
            claude_bin: "claude".to_string(),
            timeout: Duration::from_secs(120),
            gate_config: GateConfig::default(),
        }
    }
}

impl DocAnnotator {
    /// Run the LLM annotation pass on `content` and merge the gated
    /// output into `base_patch` (which is the Phase 1 doc-pass output).
    ///
    /// Returns the enriched [`GraphPatch`] and a count of gate rejections.
    ///
    /// Fails open: if the LLM call fails or the output cannot be parsed,
    /// returns `base_patch` unchanged with `0` rejections.
    pub async fn annotate(
        &self,
        content: &str,
        repo: &str,
        path: &str,
        base_patch: GraphPatch,
    ) -> (GraphPatch, usize) {
        // Build Phase 1 FactSet from the deterministic pass output.
        let facts = FactSet::from_extracted(&base_patch);

        // Collect existing RELATED edges so we don't duplicate them.
        let existing_related: std::collections::HashSet<(String, String)> = base_patch
            .edges
            .iter()
            .filter(|e| e.edge_type == "RELATED")
            .map(|e| (e.from_key.clone(), e.to_key.clone()))
            .collect();

        let raw_output = match self.call_llm(content).await {
            Some(o) => o,
            None => return (base_patch, 0),
        };

        let annotation: AnnotationOutput = match serde_json::from_str(&raw_output) {
            Ok(a) => a,
            Err(_) => return (base_patch, 0),
        };

        let article_k = article_key(repo, path);
        let mut annotation_patch = GraphPatch::default();

        // Claims → Claim nodes (Inferred) + HAS_CLAIM edges
        for (i, claim) in annotation.claims.iter().enumerate() {
            if claim.text.trim().is_empty() {
                continue;
            }
            let ck = claim_key(repo, path, i);
            let mut props = BTreeMap::new();
            props.insert(
                "text".to_string(),
                serde_json::Value::String(claim.text.clone()),
            );
            props.insert(
                "claim_type".to_string(),
                serde_json::Value::String(claim.claim_type.clone()),
            );
            props.insert(
                "repo".to_string(),
                serde_json::Value::String(repo.to_string()),
            );
            annotation_patch.nodes.push(NodeOp {
                label: "Claim".to_string(),
                natural_key: ck.clone(),
                external_id: Some(ck.clone()),
                conflict_policy: ConflictPolicy::Match,
                props,
            });
            annotation_patch.edges.push(
                make_edge("HAS_CLAIM", "Article", &article_k, "Claim", &ck)
                    .with_confidence(EdgeConfidence::Inferred, None),
            );
        }

        // Cross-doc relations → implicit edges (Inferred, with evidence)
        for rel in &annotation.relations {
            let kind_upper = to_edge_type(&rel.kind);
            if kind_upper.is_empty() || rel.evidence.trim().is_empty() {
                continue;
            }
            let target_k = article_key(repo, &rel.to_doc);
            // Skip if this duplicates an existing RELATED wikilink edge.
            if kind_upper == "RELATED"
                && existing_related.contains(&(article_k.clone(), target_k.clone()))
            {
                continue;
            }
            let edge = make_edge_with_evidence(
                &kind_upper,
                "Article",
                &article_k,
                "Article",
                &target_k,
                &rel.evidence,
            )
            .with_confidence(EdgeConfidence::Inferred, None);
            annotation_patch.edges.push(edge);
        }

        // Gate the annotation output against Phase 1 facts.
        let (gated, rejections) = apply_gate(annotation_patch, &facts, &self.gate_config);

        // Merge gated annotation into the base patch.
        let mut out = base_patch;
        out.nodes.extend(gated.nodes);
        out.edges.extend(gated.edges);

        (out, rejections.len())
    }

    /// Call `claude -p - --output-format json` with the annotation prompt,
    /// parse the outer JSON envelope, and return the inner result string.
    /// Returns `None` on timeout or command failure.
    async fn call_llm(&self, content: &str) -> Option<String> {
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let prompt = build_prompt(content);

        let mut cmd = Command::new(&self.claude_bin);
        cmd.args([
            "-p",
            "-",
            "--output-format",
            "json",
            "--dangerously-skip-permissions",
        ]);
        cmd.env("CORTEX_ADAPTER_DISABLE", "1");
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.ok()?;
        }

        let output =
            tokio::time::timeout(self.timeout, child.wait_with_output())
                .await
                .ok()?
                .ok()?;

        if !output.status.success() {
            return None;
        }

        // Parse outer JSON envelope: {"result": "...", ...}
        let envelope: serde_json::Value =
            serde_json::from_slice(&output.stdout).ok()?;
        envelope
            .get("result")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_prompt(content: &str) -> String {
    format!(
        r#"Analyze the following markdown document and extract structured knowledge.

Return ONLY valid JSON in this exact schema (no prose):
{{
  "claims": [
    {{"text": "...", "type": "factual|normative|directive"}}
  ],
  "relations": [
    {{"to_doc": "path/to/other-doc", "kind": "builds_on|contradicts|cites|exemplifies|categorized_under", "evidence": "verbatim or paraphrased text span"}}
  ]
}}

Rules:
- Only emit a relation when there is explicit textual evidence in the document.
- The "evidence" field MUST contain the supporting text span (required).
- Do NOT invent relations based on topic similarity alone.
- "to_doc" is the file path or slug of the referenced document.
- If there are no claims or relations, return empty arrays.

Document:
---
{content}
---"#,
    )
}

fn to_edge_type(kind: &str) -> String {
    match kind.to_ascii_lowercase().as_str() {
        "builds_on" => "BUILDS_ON",
        "contradicts" => "CONTRADICTS",
        "cites" => "CITES",
        "exemplifies" => "EXEMPLIFIES",
        "categorized_under" => "CATEGORIZED_UNDER",
        _ => "",
    }
    .to_string()
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
}

fn make_edge_with_evidence(
    edge_type: &str,
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
    evidence: &str,
) -> EdgeOp {
    let mut edge = make_edge(edge_type, from_label, from_key, to_label, to_key);
    edge.description = Some(evidence.to_string());
    edge
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::doc_pass::DocPass;

    /// Parse a canned JSON annotation (as the LLM would return) and verify
    /// the gate and dedup logic without invoking the actual CLI.
    fn apply_annotation(
        doc_content: &str,
        repo: &str,
        path: &str,
        annotation_json: &str,
    ) -> (GraphPatch, usize) {
        let base_patch = DocPass { repo: repo.to_string() }.scan_file(doc_content, path);
        let facts = FactSet::from_extracted(&base_patch);
        let existing_related: std::collections::HashSet<(String, String)> = base_patch
            .edges
            .iter()
            .filter(|e| e.edge_type == "RELATED")
            .map(|e| (e.from_key.clone(), e.to_key.clone()))
            .collect();

        let annotation: AnnotationOutput =
            serde_json::from_str(annotation_json).expect("test JSON must be valid");
        let article_k = article_key(repo, path);
        let mut annotation_patch = GraphPatch::default();

        for (i, claim) in annotation.claims.iter().enumerate() {
            if claim.text.trim().is_empty() {
                continue;
            }
            let ck = claim_key(repo, path, i);
            let mut props = BTreeMap::new();
            props.insert(
                "text".to_string(),
                serde_json::Value::String(claim.text.clone()),
            );
            props.insert(
                "claim_type".to_string(),
                serde_json::Value::String(claim.claim_type.clone()),
            );
            props.insert(
                "repo".to_string(),
                serde_json::Value::String(repo.to_string()),
            );
            annotation_patch.nodes.push(NodeOp {
                label: "Claim".to_string(),
                natural_key: ck.clone(),
                external_id: Some(ck.clone()),
                conflict_policy: ConflictPolicy::Match,
                props,
            });
            annotation_patch.edges.push(
                make_edge("HAS_CLAIM", "Article", &article_k, "Claim", &ck)
                    .with_confidence(EdgeConfidence::Inferred, None),
            );
        }

        for rel in &annotation.relations {
            let kind_upper = to_edge_type(&rel.kind);
            if kind_upper.is_empty() || rel.evidence.trim().is_empty() {
                continue;
            }
            let target_k = article_key(repo, &rel.to_doc);
            if kind_upper == "RELATED"
                && existing_related.contains(&(article_k.clone(), target_k.clone()))
            {
                continue;
            }
            let edge = make_edge_with_evidence(
                &kind_upper,
                "Article",
                &article_k,
                "Article",
                &target_k,
                &rel.evidence,
            )
            .with_confidence(EdgeConfidence::Inferred, None);
            annotation_patch.edges.push(edge);
        }

        let (gated, rejections) =
            apply_gate(annotation_patch, &facts, &GateConfig::default());
        let mut out = base_patch;
        out.nodes.extend(gated.nodes);
        out.edges.extend(gated.edges);
        (out, rejections.len())
    }

    #[test]
    fn annotator_emits_claim_nodes() {
        let doc = "---\ntitle: foo\n---\n# Doc\n\nWe use the [[other-doc]] approach.\n";
        let annotation = r#"{"claims":[{"text":"We use a deterministic approach","type":"directive"}],"relations":[]}"#;
        let (patch, _) = apply_annotation(doc, "repo", "foo.md", annotation);
        let has_claim = patch.nodes.iter().any(|n| n.label == "Claim");
        assert!(has_claim, "should emit Claim node from annotation");
    }

    #[test]
    fn annotator_emits_contradicts_edge_with_evidence() {
        let doc = "# A\n\nContrary to [[other-doc]], we argue X.\n";
        let annotation = r#"{"claims":[],"relations":[{"to_doc":"other-doc","kind":"contradicts","evidence":"contrary to other-doc, we argue X"}]}"#;
        let (patch, _) = apply_annotation(doc, "repo", "a.md", annotation);
        let has_edge = patch
            .edges
            .iter()
            .any(|e| e.edge_type == "CONTRADICTS" && e.description.is_some());
        assert!(has_edge, "should emit CONTRADICTS edge with evidence");
    }

    #[test]
    fn annotator_requires_evidence_for_relations() {
        let doc = "# A\n\nSimilar to [[other-doc]].\n";
        // Empty evidence — relation should be dropped
        let annotation = r#"{"claims":[],"relations":[{"to_doc":"other-doc","kind":"cites","evidence":""}]}"#;
        let (patch, _) = apply_annotation(doc, "repo", "a.md", annotation);
        let has_cites = patch.edges.iter().any(|e| e.edge_type == "CITES");
        assert!(!has_cites, "relation without evidence must be dropped");
    }

    #[test]
    fn annotator_does_not_duplicate_wikilink_edges() {
        // Doc has [[other-doc]] wikilink → RELATED edge in Phase 1
        let doc = "# A\n\nSee [[other-doc]].\n";
        // LLM also outputs a RELATED relation for the same pair — should be deduped
        // (RELATED would be treated as unknown kind anyway since it's not in the map)
        // Use cites to avoid confusing with existing wikilink
        let annotation = r#"{"claims":[],"relations":[{"to_doc":"other-doc","kind":"cites","evidence":"as in other-doc"}]}"#;
        let (patch, _) = apply_annotation(doc, "repo", "a.md", annotation);
        // One RELATED from wikilink, one CITES from annotation
        let related_count = patch.edges.iter().filter(|e| e.edge_type == "RELATED").count();
        let cites_count = patch.edges.iter().filter(|e| e.edge_type == "CITES").count();
        assert_eq!(related_count, 1, "only one RELATED edge (from wikilink)");
        assert_eq!(cites_count, 1, "CITES edge from annotation is preserved");
    }

    #[test]
    fn annotator_gate_rejects_endpoint_not_in_facts() {
        let doc = "# A\n\nNo wikilinks here.\n";
        // Relation to a doc that was NOT in the Phase 1 FactSet (not wikilinked)
        let annotation = r#"{"claims":[],"relations":[{"to_doc":"unknown-doc","kind":"contradicts","evidence":"see unknown-doc"}]}"#;
        // The gate should reject the CONTRADICTS edge because "unknown-doc" is not in FactSet.
        let (patch, rejections) = apply_annotation(doc, "repo", "a.md", annotation);
        let has_contradicts = patch.edges.iter().any(|e| e.edge_type == "CONTRADICTS");
        // Gate rejects Inferred edges whose endpoints aren't in facts.
        assert!(!has_contradicts, "endpoint not in facts must be rejected");
        assert!(rejections > 0, "gate should report at least one rejection");
    }
}

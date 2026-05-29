//! phase18 §2.2 + §2.3 — Branch node + TimelineEvent entity definitions.
//!
//! `Branch` and `TimelineEvent` are graph-only entities — they are
//! NOT written via the envelope ingestion pipeline (no `Kind`
//! variant, no classifier, no embedder). They land on Nexus via
//! the `cortex-ops branch` and `cortex-ops timeline` CLIs (§4.2)
//! and are mutated directly by the writer functions exposed here.
//!
//! The split matches ADR-019 §1.2 (branch identity is operator-
//! controlled, not classifier-derived) and design.md §1.3
//! (timeline_event is a thin projection over heterogeneous facts,
//! not a first-class envelope kind).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::patch::{ConflictPolicy, GraphPatch, NodeOp};
use super::bitemporal::{keys, DEFAULT_BRANCH};

/// Branch node label per ADR-019 §1.2. The reserved name `main`
/// exists per project; auto-created by the migration CLI in §2.12.
pub const BRANCH_LABEL: &str = "Branch";

/// TimelineEvent node label per design.md §1.3.
pub const TIMELINE_EVENT_LABEL: &str = "TimelineEvent";

/// Phase18 §2.2 — `branch.status` enum. ADR-021 references the
/// values; the writer pins them so the SUPERSEDES-edge backfill
/// can switch on the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    /// Branch is open; new facts can land on it.
    Active,
    /// Branch was folded back into a parent via `MERGED_INTO`.
    Merged,
    /// Branch was abandoned without a merge.
    Abandoned,
}

impl BranchStatus {
    /// Stable lowercase label for the graph property.
    pub fn as_str(self) -> &'static str {
        match self {
            BranchStatus::Active => "active",
            BranchStatus::Merged => "merged",
            BranchStatus::Abandoned => "abandoned",
        }
    }
}

/// Phase18 §2.2 — `branch.merge_strategy` enum per ADR-021 §1.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Every branch fact folds onto the parent post-merge.
    Accept,
    /// Only facts flagged `merge_kept` on the branch fold onto the parent.
    Partial,
    /// No branch fact surfaces on parent retrievals (audit-only).
    Discard,
}

impl MergeStrategy {
    /// Stable lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            MergeStrategy::Accept => "accept",
            MergeStrategy::Partial => "partial",
            MergeStrategy::Discard => "discard",
        }
    }
}

/// Full Branch node payload the writer commits to Nexus. Field
/// names mirror design.md §1.2 verbatim so a future reader can
/// cross-check without translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Composite global id `<project_id>:<branch_name>` per ADR-019.
    pub id: String,
    /// Lower-cased project slug.
    pub project_id: String,
    /// Branch name (regex-validated; `main` reserved per ADR-019).
    pub name: String,
    /// Parent branch composite id. `None` only for `main`.
    pub parent_branch_id: Option<String>,
    /// Event id the branch forked from.
    pub fork_point_event_id: Option<String>,
    /// Valid-time at the fork (RFC3339 second-precision per ADR-018).
    pub fork_valid_time: Option<String>,
    /// active | merged | abandoned.
    pub status: BranchStatus,
    /// accept | partial | discard. `None` until the branch merges.
    pub merge_strategy: Option<MergeStrategy>,
    /// Event id of the merge. `None` until the branch merges.
    pub merge_point_event_id: Option<String>,
    /// Free-text reason. `None` until the branch abandons.
    pub abandonment_reason: Option<String>,
    /// Creation timestamp (RFC3339 second-precision per ADR-018).
    pub created_at: String,
    /// Agent or operator id that created the branch.
    pub created_by: String,
}

impl Branch {
    /// Build the reserved `main` branch for a project. Used by
    /// §2.12 backfill and by the per-project auto-create path in
    /// the writer.
    pub fn main_for(project_id: impl Into<String>, created_at: impl Into<String>) -> Self {
        let project_id = project_id.into();
        Self {
            id: format!("{project_id}:{DEFAULT_BRANCH}"),
            project_id: project_id.clone(),
            name: DEFAULT_BRANCH.to_string(),
            parent_branch_id: None,
            fork_point_event_id: None,
            fork_valid_time: None,
            status: BranchStatus::Active,
            merge_strategy: None,
            merge_point_event_id: None,
            abandonment_reason: None,
            created_at: created_at.into(),
            created_by: "system".to_string(),
        }
    }

    /// Convert to a `NodeOp` ready to push onto a `GraphPatch`. The
    /// caller is responsible for ensuring `self.name` matches the
    /// ADR-019 regex (the constructor validates at call time; this
    /// converter trusts the input).
    pub fn as_node_op(&self) -> NodeOp {
        let mut props: BTreeMap<String, Value> = BTreeMap::new();
        props.insert("id".to_string(), Value::String(self.id.clone()));
        props.insert(
            keys::PROJECT_ID.to_string(),
            Value::String(self.project_id.clone()),
        );
        props.insert("name".to_string(), Value::String(self.name.clone()));
        if let Some(parent) = &self.parent_branch_id {
            props.insert(
                "parent_branch_id".to_string(),
                Value::String(parent.clone()),
            );
        }
        if let Some(fork_point) = &self.fork_point_event_id {
            props.insert(
                "fork_point_event_id".to_string(),
                Value::String(fork_point.clone()),
            );
        }
        if let Some(fork_time) = &self.fork_valid_time {
            props.insert(
                "fork_valid_time".to_string(),
                Value::String(fork_time.clone()),
            );
        }
        props.insert(
            "status".to_string(),
            Value::String(self.status.as_str().to_string()),
        );
        if let Some(strategy) = self.merge_strategy {
            props.insert(
                "merge_strategy".to_string(),
                Value::String(strategy.as_str().to_string()),
            );
        }
        if let Some(merge_event) = &self.merge_point_event_id {
            props.insert(
                "merge_point_event_id".to_string(),
                Value::String(merge_event.clone()),
            );
        }
        if let Some(reason) = &self.abandonment_reason {
            props.insert(
                "abandonment_reason".to_string(),
                Value::String(reason.clone()),
            );
        }
        props.insert(
            "created_at".to_string(),
            Value::String(self.created_at.clone()),
        );
        props.insert(
            "created_by".to_string(),
            Value::String(self.created_by.clone()),
        );
        NodeOp {
            label: BRANCH_LABEL.to_string(),
            natural_key: self.id.clone(),
            external_id: Some(self.id.clone()),
            conflict_policy: ConflictPolicy::default(),
            props,
        }
    }
}

/// Phase18 §2.3 — `timeline_event.kind` controlled vocabulary
/// per design.md §1.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    /// Git commit landed.
    Commit,
    /// Architecture decision record.
    Adr,
    /// In-session decision.
    Decision,
    /// Project release / tag.
    Release,
    /// Operational incident.
    Incident,
    /// Captured learning.
    Learning,
    /// Rulebook task started.
    TaskStart,
    /// Rulebook task archived.
    TaskArchive,
    /// Branch fork event.
    BranchFork,
    /// Branch merge event.
    BranchMerge,
    /// Branch abandonment event.
    BranchAbandon,
    /// Cross-project link landed.
    CrossProjectLink,
}

impl TimelineKind {
    /// Stable lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            TimelineKind::Commit => "commit",
            TimelineKind::Adr => "adr",
            TimelineKind::Decision => "decision",
            TimelineKind::Release => "release",
            TimelineKind::Incident => "incident",
            TimelineKind::Learning => "learning",
            TimelineKind::TaskStart => "task_start",
            TimelineKind::TaskArchive => "task_archive",
            TimelineKind::BranchFork => "branch_fork",
            TimelineKind::BranchMerge => "branch_merge",
            TimelineKind::BranchAbandon => "branch_abandon",
            TimelineKind::CrossProjectLink => "cross_project_link",
        }
    }
}

/// Full TimelineEvent node payload per design.md §1.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// Stable id (ULID).
    pub id: String,
    /// Lower-cased project slug.
    pub project_id: String,
    /// Composite branch id (`<project>:<branch>`).
    pub branch_id: String,
    /// When the underlying fact starts being true.
    pub valid_time: String,
    /// When Cortex first persisted the timeline event.
    pub recorded_at: String,
    /// Discriminator.
    pub kind: TimelineKind,
    /// Short title (≤ 80 chars).
    pub title: String,
    /// Markdown summary (≤ 2 KiB).
    pub summary: String,
    /// Pointer to the underlying entity.
    pub ref_entity_id: String,
    /// Label of the underlying entity (`Decision`, `Branch`, …).
    pub ref_entity_kind: String,
    /// Free-form tag list.
    pub tags: Vec<String>,
}

impl TimelineEvent {
    /// Convert to a `NodeOp` ready to push onto a `GraphPatch`.
    pub fn as_node_op(&self) -> NodeOp {
        let mut props: BTreeMap<String, Value> = BTreeMap::new();
        props.insert("id".to_string(), Value::String(self.id.clone()));
        props.insert(
            keys::PROJECT_ID.to_string(),
            Value::String(self.project_id.clone()),
        );
        props.insert(
            keys::BRANCH_ID.to_string(),
            Value::String(self.branch_id.clone()),
        );
        props.insert(
            keys::VALID_FROM.to_string(),
            Value::String(self.valid_time.clone()),
        );
        props.insert(
            keys::RECORDED_AT.to_string(),
            Value::String(self.recorded_at.clone()),
        );
        props.insert(
            "kind".to_string(),
            Value::String(self.kind.as_str().to_string()),
        );
        props.insert("title".to_string(), Value::String(self.title.clone()));
        props.insert("summary".to_string(), Value::String(self.summary.clone()));
        props.insert(
            "ref_entity_id".to_string(),
            Value::String(self.ref_entity_id.clone()),
        );
        props.insert(
            "ref_entity_kind".to_string(),
            Value::String(self.ref_entity_kind.clone()),
        );
        if !self.tags.is_empty() {
            props.insert(
                "tags".to_string(),
                Value::Array(self.tags.iter().cloned().map(Value::String).collect()),
            );
        }
        NodeOp {
            label: TIMELINE_EVENT_LABEL.to_string(),
            natural_key: self.id.clone(),
            external_id: Some(self.id.clone()),
            conflict_policy: ConflictPolicy::default(),
            props,
        }
    }
}

/// Convenience: push a Branch onto a fresh `GraphPatch` ready for
/// the writer.
pub fn patch_from_branch(branch: &Branch) -> GraphPatch {
    let mut patch = GraphPatch::empty();
    patch.nodes.push(branch.as_node_op());
    patch
}

/// Convenience: push a TimelineEvent onto a fresh `GraphPatch`.
pub fn patch_from_timeline_event(event: &TimelineEvent) -> GraphPatch {
    let mut patch = GraphPatch::empty();
    patch.nodes.push(event.as_node_op());
    patch
}

/// Phase18 §1.2 ADR-019 branch-name regex enforcement. Returns
/// `Err(reason)` when the name does not match `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$`.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    if name.len() < 2 || name.len() > 64 {
        return Err(format!(
            "branch name must be 2..=64 chars, got {len}",
            len = name.len()
        ));
    }
    let bytes = name.as_bytes();
    let first_ok = bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit();
    let last = bytes[bytes.len() - 1];
    let last_ok = last.is_ascii_lowercase() || last.is_ascii_digit();
    if !first_ok || !last_ok {
        return Err("branch name must start and end with [a-z0-9]".to_string());
    }
    for (i, b) in bytes.iter().enumerate() {
        let ok = b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(*b, b'.' | b'_' | b'/' | b'-');
        if !ok {
            return Err(format!(
                "branch name char at index {i} not in [a-z0-9._/-]"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_branch_factory_yields_composite_id() {
        let b = Branch::main_for("cortex", "2026-05-29T07:00:00Z");
        assert_eq!(b.id, "cortex:main");
        assert_eq!(b.project_id, "cortex");
        assert_eq!(b.name, DEFAULT_BRANCH);
        assert!(b.parent_branch_id.is_none());
        assert_eq!(b.status, BranchStatus::Active);
        assert_eq!(b.created_by, "system");
    }

    #[test]
    fn branch_node_op_carries_all_set_fields() {
        let mut b = Branch::main_for("cortex", "2026-05-29T07:00:00Z");
        b.parent_branch_id = Some("cortex:main".to_string());
        b.fork_point_event_id = Some("01HZFORK0000000000000000A".to_string());
        b.fork_valid_time = Some("2026-04-01T12:00:00Z".to_string());
        b.status = BranchStatus::Merged;
        b.merge_strategy = Some(MergeStrategy::Accept);
        b.merge_point_event_id = Some("01HZMERGE000000000000000A".to_string());
        let op = b.as_node_op();
        assert_eq!(op.label, BRANCH_LABEL);
        assert_eq!(op.natural_key, "cortex:main");
        for required in [
            "id",
            keys::PROJECT_ID,
            "name",
            "parent_branch_id",
            "fork_point_event_id",
            "fork_valid_time",
            "status",
            "merge_strategy",
            "merge_point_event_id",
            "created_at",
            "created_by",
        ] {
            assert!(
                op.props.contains_key(required),
                "branch NodeOp missing prop `{required}`"
            );
        }
        assert_eq!(op.props.get("status").and_then(|v| v.as_str()), Some("merged"));
        assert_eq!(
            op.props.get("merge_strategy").and_then(|v| v.as_str()),
            Some("accept")
        );
    }

    #[test]
    fn branch_node_op_omits_unset_optional_fields() {
        let b = Branch::main_for("cortex", "2026-05-29T07:00:00Z");
        let op = b.as_node_op();
        assert!(!op.props.contains_key("parent_branch_id"));
        assert!(!op.props.contains_key("merge_strategy"));
        assert!(!op.props.contains_key("merge_point_event_id"));
        assert!(!op.props.contains_key("abandonment_reason"));
    }

    #[test]
    fn timeline_event_node_op_carries_all_fields() {
        let ev = TimelineEvent {
            id: "01HZTLE000000000000000000A".into(),
            project_id: "cortex".into(),
            branch_id: "cortex:main".into(),
            valid_time: "2026-05-29T07:00:00Z".into(),
            recorded_at: "2026-05-29T07:00:00Z".into(),
            kind: TimelineKind::BranchFork,
            title: "feat/spec-11-v2 forked".into(),
            summary: "Branch forked at commit abc123.".into(),
            ref_entity_id: "cortex:feat/spec-11-v2".into(),
            ref_entity_kind: BRANCH_LABEL.into(),
            tags: vec!["fork".into()],
        };
        let op = ev.as_node_op();
        assert_eq!(op.label, TIMELINE_EVENT_LABEL);
        assert_eq!(op.props.get("kind").and_then(|v| v.as_str()), Some("branch_fork"));
        assert_eq!(
            op.props.get("ref_entity_kind").and_then(|v| v.as_str()),
            Some(BRANCH_LABEL)
        );
        assert!(op.props.contains_key("tags"));
    }

    #[test]
    fn validate_branch_name_accepts_canonical_shapes() {
        for ok in [
            "main",
            "feat/spec-11-v2",
            "fix.retry",
            "exp_2026-04",
            "a1",
        ] {
            assert!(validate_branch_name(ok).is_ok(), "should accept `{ok}`");
        }
    }

    #[test]
    fn validate_branch_name_rejects_off_shape_inputs() {
        for bad in [
            "",                                                   // empty
            "x",                                                  // too short
            "feat/X",                                             // uppercase
            "/leading-slash",                                     // leading punct
            "trailing-",                                          // trailing punct
            "with space",                                         // space
            "tab\there",                                          // tab
            "way-too-long-".repeat(10).as_str(),                  // > 64
        ] {
            assert!(
                validate_branch_name(bad).is_err(),
                "should reject `{bad}`"
            );
        }
    }
}

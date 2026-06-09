//! Phase18 §4.2 — `cortex-ops branch` subcommands.
//!
//! Surfaces the operator path for the spec-30 `Branch` node:
//!
//! - `list` — every branch for a project,
//! - `show` — one branch's full payload,
//! - `create` — fork a new branch off `--from <parent>` at an
//!   optional valid-time anchor,
//! - `merge` — fold a branch back into its parent with the
//!   ADR-021 strategy choice,
//! - `abandon` — close a branch without merge (audit-only).
//!
//! Reads land on `LiveNexusClient::sdk().execute_cypher`; writes
//! emit inlined-literal Cypher per the same Nexus 2.2.0 binding
//! workaround the §4.1 timeline reader uses.

use std::process::ExitCode;

use chrono::{DateTime, Utc};

use cortex_workers::graph::bitemporal::DEFAULT_BRANCH;
use cortex_workers::graph::branch::{
    validate_branch_name, BranchStatus, MergeStrategy, BRANCH_LABEL,
};

/// Phase18 §4.2 — `cortex-ops branch list <project>`.
pub(super) fn branch_list(project: String, nexus: Option<String>, json: bool) -> ExitCode {
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let cypher = format!(
        "MATCH (b:{label}) WHERE b.project_id = '{proj}' \
         RETURN b.id AS id, b.name AS name, b.parent_branch_id AS parent_branch_id, \
                b.status AS status, b.merge_strategy AS merge_strategy, \
                b.created_at AS created_at, b.created_by AS created_by \
         ORDER BY b.created_at ASC",
        label = BRANCH_LABEL,
        proj = sanitize_literal(&project),
    );

    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus cypher: {e}");
            return ExitCode::from(2);
        }
    };

    if json {
        let payload = serde_json::json!({
            "project": project,
            "nexus_url": nexus_url,
            "branches": rows,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!(
        "cortex-ops branch list @ {nexus_url}\n  project={project}  branches={count}",
        count = rows.len()
    );
    if rows.is_empty() {
        return ExitCode::SUCCESS;
    }
    println!(
        "  {:<26}  {:<10}  {:<24}  {:<16}",
        "id", "status", "parent_branch_id", "created_at"
    );
    for row in &rows {
        let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("<?>");
        let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("<?>");
        let parent = row
            .get("parent_branch_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<root>");
        let created = row
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("<?>");
        println!("  {id:<26}  {status:<10}  {parent:<24}  {created:<16}");
    }
    ExitCode::SUCCESS
}

/// Phase18 §4.2 — `cortex-ops branch show <project> <branch>`.
pub(super) fn branch_show(
    project: String,
    branch: String,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
    if let Err(e) = validate_branch_name_or_main(&branch) {
        eprintln!("ERROR: {e}");
        return ExitCode::from(2);
    }
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let composite = compose_id(&project, &branch);
    let cypher = format!(
        "MATCH (b:{label} {{ id: '{id}' }}) RETURN b",
        label = BRANCH_LABEL,
        id = sanitize_literal(&composite),
    );
    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus cypher: {e}");
            return ExitCode::from(2);
        }
    };
    if rows.is_empty() {
        eprintln!("ERROR: branch {composite:?} not found");
        return ExitCode::from(1);
    }
    let row = &rows[0];
    if json {
        println!("{}", serde_json::to_string_pretty(row).unwrap_or_default());
    } else {
        println!("cortex-ops branch show @ {nexus_url}\n  id={composite}");
        println!("{}", serde_json::to_string_pretty(row).unwrap_or_default());
    }
    ExitCode::SUCCESS
}

/// Phase18 §4.2 — `cortex-ops branch create`.
pub(super) fn branch_create(
    project: String,
    name: String,
    from: String,
    valid_time: Option<String>,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
    if let Err(e) = validate_branch_name(&name) {
        eprintln!("ERROR: --name: {e}");
        return ExitCode::from(2);
    }
    // `from` is a branch *name* (the spec syntax is `<branch>@<date>`
    // but the @-suffix is parsed into `valid_time` upstream by clap).
    if let Err(e) = validate_branch_name_or_main(&from) {
        eprintln!("ERROR: --from: {e}");
        return ExitCode::from(2);
    }
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let now_rfc3339 = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let fork_valid_time = match valid_time.as_deref().map(parse_rfc3339_or_date).transpose() {
        Ok(v) => v.flatten(),
        Err(e) => {
            eprintln!("ERROR: --from @<date>: {e}");
            return ExitCode::from(2);
        }
    };
    let child_id = compose_id(&project, &name);
    let parent_id = compose_id(&project, &from);
    let fork_time_lit = fork_valid_time
        .as_deref()
        .map(|s| format!("'{}'", sanitize_literal(s)))
        .unwrap_or_else(|| "NULL".to_string());

    // Inline-literal Cypher: create the branch node + FORKED_FROM
    // edge. The MERGE on the Branch ensures idempotency on retry.
    let cypher = format!(
        "MERGE (parent:{label} {{ id: '{parent}' }}) \
         MERGE (b:{label} {{ id: '{child}' }}) \
         ON CREATE SET b.project_id = '{proj}', b.name = '{name}', \
                       b.parent_branch_id = '{parent}', \
                       b.fork_valid_time = {fork_time_lit}, \
                       b.status = 'active', \
                       b.created_at = '{now}', \
                       b.created_by = 'cortex-ops' \
         MERGE (b)-[:FORKED_FROM]->(parent) \
         RETURN b",
        label = BRANCH_LABEL,
        parent = sanitize_literal(&parent_id),
        child = sanitize_literal(&child_id),
        proj = sanitize_literal(&project),
        name = sanitize_literal(&name),
        now = sanitize_literal(&now_rfc3339),
        fork_time_lit = fork_time_lit,
    );

    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus cypher: {e}");
            return ExitCode::from(2);
        }
    };

    if json {
        let payload = serde_json::json!({
            "nexus_url": nexus_url,
            "branch_id": child_id,
            "parent_branch_id": parent_id,
            "status": BranchStatus::Active.as_str(),
            "fork_valid_time": fork_valid_time,
            "result": rows.first(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops branch create @ {nexus_url}\n  id={child_id}\n  parent={parent_id}\n  status=active\n  created_at={now_rfc3339}"
        );
    }
    ExitCode::SUCCESS
}

/// Phase18 §4.2 — `cortex-ops branch merge`.
pub(super) fn branch_merge(
    project: String,
    branch: String,
    strategy: String,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
    if let Err(e) = validate_branch_name(&branch) {
        eprintln!("ERROR: <branch>: {e}");
        return ExitCode::from(2);
    }
    let strategy_enum = match strategy.as_str() {
        "accept" => MergeStrategy::Accept,
        "partial" => MergeStrategy::Partial,
        "discard" => MergeStrategy::Discard,
        other => {
            eprintln!("ERROR: --strategy must be accept|partial|discard, got {other:?}");
            return ExitCode::from(2);
        }
    };
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let now_rfc3339 = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let merge_point_event_id = new_ulid_like(&now_rfc3339);
    let child_id = compose_id(&project, &branch);

    // Locate the parent_branch_id off the existing Branch node so the
    // merge edge stamps the right endpoint. A missing branch is a hard
    // failure (status code 1).
    let lookup_cypher = format!(
        "MATCH (b:{label} {{ id: '{id}' }}) RETURN b.parent_branch_id AS parent_branch_id",
        label = BRANCH_LABEL,
        id = sanitize_literal(&child_id),
    );
    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let parent_branch_id =
        match runtime.block_on(async { client.sdk().execute_cypher(&lookup_cypher, None).await }) {
            Ok(out) => out.rows.first().and_then(|row| {
                row.get("parent_branch_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
            Err(e) => {
                eprintln!("ERROR: nexus lookup: {e}");
                return ExitCode::from(2);
            }
        };
    let Some(parent_id) = parent_branch_id else {
        eprintln!(
            "ERROR: branch {child_id:?} either does not exist or carries no parent_branch_id (the `main` branch cannot be merged)"
        );
        return ExitCode::from(1);
    };

    let cypher = format!(
        "MATCH (b:{label} {{ id: '{child}' }}), (parent:{label} {{ id: '{parent}' }}) \
         SET b.status = 'merged', \
             b.merge_strategy = '{strategy}', \
             b.merge_point_event_id = '{merge_event}' \
         MERGE (b)-[r:MERGED_INTO]->(parent) \
         SET r.strategy = '{strategy}', \
             r.merge_point_event_id = '{merge_event}' \
         RETURN b",
        label = BRANCH_LABEL,
        child = sanitize_literal(&child_id),
        parent = sanitize_literal(&parent_id),
        strategy = strategy_enum.as_str(),
        merge_event = sanitize_literal(&merge_point_event_id),
    );
    let rows = match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus merge: {e}");
            return ExitCode::from(2);
        }
    };

    if json {
        let payload = serde_json::json!({
            "nexus_url": nexus_url,
            "branch_id": child_id,
            "parent_branch_id": parent_id,
            "strategy": strategy_enum.as_str(),
            "merge_point_event_id": merge_point_event_id,
            "merged_at": now_rfc3339,
            "result": rows.first(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops branch merge @ {nexus_url}\n  id={child_id}\n  parent={parent_id}\n  strategy={s}\n  merge_point_event_id={merge_point_event_id}\n  merged_at={now_rfc3339}",
            s = strategy_enum.as_str()
        );
    }
    ExitCode::SUCCESS
}

/// Phase18 §4.2 — `cortex-ops branch abandon`.
pub(super) fn branch_abandon(
    project: String,
    branch: String,
    reason: String,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
    if let Err(e) = validate_branch_name(&branch) {
        eprintln!("ERROR: <branch>: {e}");
        return ExitCode::from(2);
    }
    if reason.trim().is_empty() {
        eprintln!(
            "ERROR: --reason must be non-empty (ADR-022: abandoned branches always carry a reason)"
        );
        return ExitCode::from(2);
    }
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let child_id = compose_id(&project, &branch);
    let cypher = format!(
        "MATCH (b:{label} {{ id: '{child}' }}) \
         SET b.status = 'abandoned', b.abandonment_reason = '{reason}' \
         RETURN b",
        label = BRANCH_LABEL,
        child = sanitize_literal(&child_id),
        reason = sanitize_literal(&reason),
    );
    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime.block_on(async { client.sdk().execute_cypher(&cypher, None).await }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus abandon: {e}");
            return ExitCode::from(2);
        }
    };
    if rows.is_empty() {
        eprintln!("ERROR: branch {child_id:?} not found");
        return ExitCode::from(1);
    }
    if json {
        let payload = serde_json::json!({
            "nexus_url": nexus_url,
            "branch_id": child_id,
            "status": BranchStatus::Abandoned.as_str(),
            "abandonment_reason": reason,
            "result": rows.first(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops branch abandon @ {nexus_url}\n  id={child_id}\n  status=abandoned\n  reason={reason}"
        );
    }
    ExitCode::SUCCESS
}

// ---- helpers ----------------------------------------------------------------

fn connect_nexus(
    nexus: Option<String>,
) -> Result<(cortex_workers::graph::nexus_client::LiveNexusClient, String), ExitCode> {
    use cortex_workers::graph::config::GraphConfig;
    use cortex_workers::graph::nexus_client::LiveNexusClient;

    let nexus_url = nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string());
    let cfg = GraphConfig {
        nexus_url: nexus_url.clone(),
        ..GraphConfig::default()
    };
    LiveNexusClient::new(cfg)
        .map(|c| (c, nexus_url.clone()))
        .map_err(|e| {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            ExitCode::from(2)
        })
}

fn build_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Runtime::new().map_err(|e| {
        eprintln!("ERROR: build tokio runtime: {e}");
        ExitCode::from(1)
    })
}

fn compose_id(project: &str, branch: &str) -> String {
    cortex_workers::temporal::branch_filter::compose_id(project, branch)
}

fn sanitize_literal(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '\n' | '\r'))
        .collect()
}

fn validate_branch_name_or_main(name: &str) -> Result<(), String> {
    if name == DEFAULT_BRANCH {
        return Ok(());
    }
    validate_branch_name(name)
}

fn parse_rfc3339_or_date(raw: &str) -> Result<Option<String>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| {
            Some(
                dt.with_timezone(&Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            )
        })
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

/// Build a ULID-shaped id off the current timestamp. Cortex stamps
/// the merge_point_event_id so the SUPERSEDES walk on classifier
/// retrievals can read a stable reference; a Crockford-ULID-shaped
/// 26-char string keeps the writer aligned with the rest of the
/// stamping pipeline without pulling the `ulid` crate just for the
/// CLI.
fn new_ulid_like(now_rfc3339: &str) -> String {
    let digits: String = now_rfc3339
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect();
    format!("MRG-{digits:0<22}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339_or_date_accepts_full_rfc3339() {
        let out = parse_rfc3339_or_date("2026-04-01T12:34:56Z").unwrap();
        assert_eq!(out.as_deref(), Some("2026-04-01T12:34:56Z"));
    }

    #[test]
    fn parse_rfc3339_or_date_expands_day_precision_to_midnight_utc() {
        let out = parse_rfc3339_or_date("2026-04-01").unwrap();
        assert_eq!(out.as_deref(), Some("2026-04-01T00:00:00Z"));
    }

    #[test]
    fn parse_rfc3339_or_date_handles_empty_input() {
        assert!(parse_rfc3339_or_date("").unwrap().is_none());
        assert!(parse_rfc3339_or_date("   ").unwrap().is_none());
    }

    #[test]
    fn sanitize_literal_strips_quote_backslash_newline() {
        assert_eq!(sanitize_literal("ok-id"), "ok-id");
        assert_eq!(sanitize_literal("a'b\"c\\d\ne\rf"), "abcdef");
    }

    #[test]
    fn validate_branch_name_or_main_accepts_reserved_main() {
        assert!(validate_branch_name_or_main(DEFAULT_BRANCH).is_ok());
    }

    #[test]
    fn validate_branch_name_or_main_rejects_uppercase_branch() {
        assert!(validate_branch_name_or_main("FeatureX").is_err());
    }

    #[test]
    fn new_ulid_like_yields_26_char_id() {
        let id = new_ulid_like("2026-04-01T12:34:56Z");
        assert_eq!(id.len(), 26, "id was {id}");
        assert!(id.starts_with("MRG-"));
    }
}

//! `cortex-ops timeline-backfill` subcommand.
//!
//! Derives synthetic `TimelineEvent` nodes from the progress-bearing
//! nodes that already live in Nexus (`Decision`, `Analysis`, `Memory`,
//! `LawViolation`, `Learning`, `Knowledge`) across ALL projects, so
//! the Bitemporal Timeline view surfaces real cross-project history
//! without requiring every writer to emit `TimelineEvent` nodes
//! directly.
//!
//! Each source node is mapped to one `TimelineEvent` whose `id` is
//! `tl:<Label>:<source-id>`.  The write is a Cypher `MERGE` so the
//! command is idempotent — re-running updates `SET` fields without
//! creating duplicates.
//!
//! Nexus 2.2.0 has a parameter-binding bug.  Every string value that
//! is interpolated into a Cypher query passes through
//! `sanitize_literal` (drops `' " \ \n \r`) exactly like the rest of
//! the `cortex-ops` handlers.

use std::collections::HashMap;
use std::process::ExitCode;

use chrono::DateTime;

// ── Source-label table ────────────────────────────────────────────────────────

/// Each entry is `(Nexus label, TimelineEvent.kind value)`.
const SOURCE_LABELS: &[(&str, &str)] = &[
    ("Decision", "decision"),
    ("Analysis", "analysis"),
    ("Memory", "memory"),
    ("LawViolation", "incident"),
    ("Learning", "learning"),
    ("Knowledge", "learning"),
];
// Turns are chat / work turns (not commits). They are aggregated
// separately into ONE `session` event per `session_id` (see the
// session pass below) so the timeline shows work sessions per project
// instead of one noisy row per message.

// ── Public entry point ────────────────────────────────────────────────────────

/// Entry point for `cortex-ops timeline-backfill`.
pub(super) fn timeline_backfill(
    nexus: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let nexus_url = resolve_nexus_url(nexus.clone());

    // Connect + runtime (skipped in dry-run mode — we still iterate
    // rows from the query pass, but we do NOT open the client).
    // Under dry-run we skip the MERGE writes, so the client is only
    // needed for the write pass.  However we still need the client for
    // the MATCH queries even in dry-run (we query to count/preview).
    // Build them unconditionally; if Nexus is unreachable under
    // --dry-run the operator just gets zero rows listed.
    let (client, runtime) = match (connect_nexus(nexus.clone()), build_runtime()) {
        (Ok(c), Ok(rt)) => (c, rt),
        (Err(code), _) | (_, Err(code)) => return code,
    };

    let mut total = 0usize;
    let mut errors = 0usize;
    let mut by_label: HashMap<String, usize> = HashMap::new();
    let mut by_project: HashMap<String, usize> = HashMap::new();
    // Nexus closes a connection per request, so Windows exhausts
    // ephemeral sockets after a few hundred sequential writes
    // (`IncompleteMessage`). Collect every MERGE fragment and flush
    // them in batches of one chained Cypher statement, cutting the
    // request count ~40x. `(fragment, label, project)` per write.
    let mut pending: Vec<(String, String, String)> = Vec::new();

    for (label, kind) in SOURCE_LABELS {
        let cypher = format!(
            "MATCH (n:{label}) WHERE n.valid_from IS NOT NULL \
             RETURN n.id AS id, \
                    coalesce(n.title, n.display, n.label, n.caption, n.user_message, n.id) AS title, \
                    n.project_id AS project_id, \
                    n.valid_from AS valid_from, n.recorded_at AS recorded_at \
             LIMIT 5000"
        );

        let rows = match runtime
            .block_on(async { client.sdk().execute_cypher(&cypher, None).await })
        {
            Ok(out) => out.rows,
            Err(e) => {
                eprintln!("WARN: timeline-backfill: query {label}: {e}");
                errors += 1;
                continue;
            }
        };

        for row in &rows {
            // Rows are positional arrays: [0]=id [1]=title [2]=project_id
            //                             [3]=valid_from [4]=recorded_at
            let arr = match row.as_array() {
                Some(a) => a,
                None => {
                    eprintln!("WARN: timeline-backfill: unexpected non-array row for {label}");
                    errors += 1;
                    continue;
                }
            };

            let id = cell_str(arr, 0);
            let title = cell_str(arr, 1);
            let project_id = cell_str(arr, 2);
            let valid_from_raw = cell_str(arr, 3);
            let recorded_at_raw = cell_str_opt(arr, 4);

            if id.is_empty() || project_id.is_empty() {
                continue;
            }

            let valid_from_unix = match parse_rfc3339_or_date(Some(&valid_from_raw)) {
                Ok(Some(v)) => v,
                _ => continue, // unparseable — skip
            };
            let recorded_at_unix = recorded_at_raw
                .as_deref()
                .and_then(|s| parse_rfc3339_or_date(Some(s)).ok().flatten())
                .unwrap_or(valid_from_unix);

            let branch_id = format!("{project_id}:main");
            let entity_id = id.clone();
            let display_title = if title.is_empty() {
                id.clone()
            } else {
                title.clone()
            };
            let summary = if title.is_empty() {
                format!("{kind} · {project_id}")
            } else {
                title.clone()
            };
            let tl_id = format!("tl:{label}:{}", sanitize_literal(&id));

            if dry_run {
                if !json {
                    println!("{tl_id} <- {label} ({project_id}) @ {valid_from_raw}");
                }
                *by_label.entry(label.to_string()).or_insert(0) += 1;
                *by_project.entry(project_id.clone()).or_insert(0) += 1;
                total += 1;
                continue;
            }

            // Build inline-literal Cypher for the MERGE write.
            let tl_id_s = sanitize_literal(&tl_id);
            let kind_s = sanitize_literal(kind);
            let project_id_s = sanitize_literal(&project_id);
            let branch_id_s = sanitize_literal(&branch_id);
            let entity_id_s = sanitize_literal(&entity_id);
            let title_s = sanitize_literal(&display_title);
            let summary_s = sanitize_literal(&summary);

            // Each chained MERGE in a batch MUST bind a UNIQUE variable —
            // reusing `t` across clauses cross-applies every SET to the
            // wrong nodes. Key the variable off the global push index.
            let v = format!("e{}", pending.len());
            let fragment = format!(
                "MERGE ({v}:TimelineEvent {{ id: '{tl_id_s}' }}) \
                 SET {v}.kind='{kind_s}', \
                     {v}.project_id='{project_id_s}', \
                     {v}.branch_id='{branch_id_s}', \
                     {v}.entity_id='{entity_id_s}', \
                     {v}.valid_from_unix={valid_from_unix}, \
                     {v}.recorded_at_unix={recorded_at_unix}, \
                     {v}.title='{title_s}', \
                     {v}.summary='{summary_s}'"
            );
            pending.push((fragment, label.to_string(), project_id.clone()));
        }
    }

    // ── Turn → session aggregation ──────────────────────────────────────────────
    // Turns are individual chat/work messages, not progress events. Collapse
    // every Turn into ONE `session` TimelineEvent per `session_id`: the
    // session's earliest valid_from anchors it, its first user message is the
    // title, and the turn count is the summary. This is the only progress
    // signal the non-cortex repos have, and it stays one clean row per work
    // session instead of hundreds of noisy message rows.
    {
        // Group Turns into sessions in Nexus (small payload — streaming
        // every turn to the client corrupted the keep-alive connection
        // before the writes). One row per (session, project).
        let agg = "MATCH (n:Turn) WHERE n.session_id IS NOT NULL AND n.valid_from IS NOT NULL \
             RETURN n.session_id AS sid, n.project_id AS project_id, \
                    min(n.valid_from) AS first, count(n) AS turns"
            .to_string();
        let rows = match runtime.block_on(async { client.sdk().execute_cypher(&agg, None).await }) {
            Ok(out) => out.rows,
            Err(e) => {
                eprintln!("WARN: timeline-backfill: query Turn sessions: {e}");
                errors += 1;
                Vec::new()
            }
        };
        for row in &rows {
            let arr = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            let sid = cell_str(arr, 0);
            let project_id = cell_str(arr, 1);
            let first_raw = cell_str(arr, 2);
            let turns = arr.get(3).and_then(|v| v.as_i64()).unwrap_or(0);
            if sid.is_empty() || project_id.is_empty() {
                continue;
            }
            let valid_from_unix = match parse_rfc3339_or_date(Some(&first_raw)) {
                Ok(Some(v)) => v,
                _ => continue,
            };
            // The session's first user message becomes the title (small
            // per-session query, ordered by valid_from).
            // Clip the message IN Cypher: a full user_message can be a
            // multi-KB paste (e.g. a task-notification), and returning it
            // whole makes Nexus close the keep-alive connection, which
            // then breaks the following write (`IncompleteMessage`).
            // substring keeps the response tiny so the connection stays
            // healthy for the MERGE.
            let title_q = format!(
                "MATCH (n:Turn {{ session_id: '{}' }}) \
                 WHERE n.user_message IS NOT NULL AND n.project_id = '{}' \
                 RETURN substring(n.user_message, 0, 100) ORDER BY n.valid_from LIMIT 1",
                sanitize_literal(&sid),
                sanitize_literal(&project_id),
            );
            let raw_title = match runtime
                .block_on(async { client.sdk().execute_cypher(&title_q, None).await })
            {
                Ok(out) => out
                    .rows
                    .first()
                    .and_then(|r| r.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                Err(_) => String::new(),
            };
            let title = if raw_title.trim().is_empty() {
                format!("Session {}", sid.chars().take(8).collect::<String>())
            } else {
                clip(raw_title.trim(), 100)
            };
            let summary = format!("{turns} turns - {project_id}");
            // Include project in the synthetic id: the same session_id can
            // appear under more than one project (cross-repo session).
            let tl_id = format!(
                "tl:session:{}:{}",
                sanitize_literal(&project_id),
                sanitize_literal(&sid)
            );
            if dry_run {
                if !json {
                    println!("{tl_id} <- session ({project_id}) {turns} turns @ {first_raw}");
                }
                *by_label.entry("session".to_string()).or_insert(0) += 1;
                *by_project.entry(project_id.clone()).or_insert(0) += 1;
                total += 1;
                continue;
            }
            let branch_id = format!("{project_id}:main");
            let v = format!("e{}", pending.len());
            let fragment = format!(
                "MERGE ({v}:TimelineEvent {{ id: '{id}' }}) \
                 SET {v}.kind='session', {v}.project_id='{proj}', {v}.branch_id='{branch}', \
                     {v}.entity_id='{ent}', {v}.valid_from_unix={vf}, {v}.recorded_at_unix={vf}, \
                     {v}.title='{title}', {v}.summary='{summary}'",
                id = sanitize_literal(&tl_id),
                proj = sanitize_literal(&project_id),
                branch = sanitize_literal(&branch_id),
                ent = sanitize_literal(&sid),
                vf = valid_from_unix,
                title = sanitize_literal(&title),
                summary = sanitize_literal(&summary),
            );
            pending.push((fragment, "session".to_string(), project_id.clone()));
        }
    }

    // ── Batch flush ─────────────────────────────────────────────────────────────
    // Chain up to BATCH MERGE clauses into one Cypher statement so the
    // whole backfill costs ~N/BATCH write requests instead of N — under
    // the per-process connection ceiling that was failing the sessions.
    const BATCH: usize = 40;
    for chunk in pending.chunks(BATCH) {
        let joined = chunk
            .iter()
            .map(|(f, _, _)| f.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let batch_cypher = format!("{joined} RETURN 1 AS ok");
        match exec_with_retry(&runtime, &client, &batch_cypher) {
            Ok(_) => {
                for (_, label, project) in chunk {
                    *by_label.entry(label.clone()).or_insert(0) += 1;
                    *by_project.entry(project.clone()).or_insert(0) += 1;
                    total += 1;
                }
            }
            Err(_) => {
                // A single malformed fragment makes Nexus reject the
                // whole chained statement (it closes the connection →
                // IncompleteMessage). Fall back to per-item writes so
                // the one poison fragment fails alone and every other
                // event in the chunk still lands.
                for (fragment, label, project) in chunk {
                    let single = format!("{fragment} RETURN 1 AS ok");
                    match exec_with_retry(&runtime, &client, &single) {
                        Ok(_) => {
                            *by_label.entry(label.clone()).or_insert(0) += 1;
                            *by_project.entry(project.clone()).or_insert(0) += 1;
                            total += 1;
                        }
                        Err(e) => {
                            eprintln!("WARN: timeline-backfill: write [{label}/{project}]: {e}");
                            errors += 1;
                        }
                    }
                }
            }
        }
    }

    // ── Report ────────────────────────────────────────────────────────────────

    if json {
        let payload = serde_json::json!({
            "nexus_url": nexus_url,
            "dry_run": dry_run,
            "total": total,
            "by_label": by_label,
            "by_project": by_project,
            "errors": errors,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops timeline-backfill @ {nexus_url}\n  total={total}  errors={errors}  dry_run={dry_run}"
        );
        if !by_label.is_empty() {
            let mut label_pairs: Vec<_> = by_label.iter().collect();
            label_pairs.sort_by_key(|(k, _)| k.as_str());
            let label_line: Vec<String> = label_pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            println!("  by_label: {}", label_line.join("  "));
        }
        if !by_project.is_empty() {
            let mut proj_pairs: Vec<_> = by_project.iter().collect();
            proj_pairs.sort_by_key(|(k, _)| k.as_str());
            let proj_line: Vec<String> = proj_pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            println!("  by_project: {}", proj_line.join("  "));
        }
    }

    if errors > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

// ── Helpers (mirrored from backfill_cross_project.rs) ────────────────────────

fn connect_nexus(
    nexus: Option<String>,
) -> Result<cortex_workers::graph::nexus_client::LiveNexusClient, ExitCode> {
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
    LiveNexusClient::new(cfg).map_err(|e| {
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

/// Sanitize a string value for inline interpolation into a Cypher
/// query. Drops quotes / backslash / backtick, all ASCII control
/// chars, AND every non-ASCII character. Nexus 2.2.0's request-body
/// JSON parser rejects non-ASCII bytes ("invalid unicode code point")
/// and drops the connection, so accented text / em-dashes / `·` etc.
/// must be stripped to keep the write alive. Lossy but Nexus-safe.
fn sanitize_literal(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '`'))
        .collect()
}

/// Execute a write Cypher with up to 3 attempts. Nexus closes
/// keep-alive connections server-side, so a pooled connection can be
/// stale and fail the send with `IncompleteMessage`; reqwest does not
/// auto-retry POSTs, so retry here (a fresh connection succeeds). A
/// short backoff lets the dead connection drain from the pool.
fn exec_with_retry(
    runtime: &tokio::runtime::Runtime,
    client: &cortex_workers::graph::nexus_client::LiveNexusClient,
    cypher: &str,
) -> Result<(), String> {
    let mut last = String::new();
    const ATTEMPTS: u32 = 6;
    for attempt in 0..ATTEMPTS {
        match runtime.block_on(async { client.sdk().execute_cypher(cypher, None).await }) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last = format!("{e:?}");
                if attempt + 1 < ATTEMPTS {
                    // Exponential backoff lets the server-closed
                    // connection drain from the pool before the retry
                    // grabs a fresh one: 120, 240, 480, 960, 1920 ms.
                    let backoff = 120u64 * (1u64 << attempt);
                    std::thread::sleep(std::time::Duration::from_millis(backoff));
                }
            }
        }
    }
    Err(last)
}

/// Clip a string to at most `max` chars, appending `…` when truncated.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Resolve the Nexus URL without constructing a live client (used in
/// the report line so dry-run runs can display it).
fn resolve_nexus_url(nexus: Option<String>) -> String {
    nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string())
}

/// Extract the string value of a positional cell from a JSON array
/// row.  Returns an empty string for `null` / absent / non-string cells.
fn cell_str(arr: &[serde_json::Value], idx: usize) -> String {
    arr.get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Like `cell_str` but returns `None` for null / absent cells so the
/// caller can distinguish "absent" from "empty string".
fn cell_str_opt(arr: &[serde_json::Value], idx: usize) -> Option<String> {
    let v = arr.get(idx)?;
    if v.is_null() {
        return None;
    }
    Some(v.as_str().unwrap_or("").to_string())
}

/// Parse an RFC-3339 string OR a day-precision (`YYYY-MM-DD`) string
/// into epoch seconds.  Returns `Ok(None)` for empty/blank input,
/// `Err(msg)` for unparseable input.
fn parse_rfc3339_or_date(raw: Option<&str>) -> Result<Option<i64>, String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| Some(dt.timestamp()))
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tl_id_derivation() {
        let label = "Decision";
        let id = "ADR-018";
        let tl_id = format!("tl:{label}:{}", sanitize_literal(id));
        assert_eq!(tl_id, "tl:Decision:ADR-018");
    }

    #[test]
    fn tl_id_derivation_sanitizes_unsafe_chars() {
        let label = "Memory";
        let id = "abc'def\"gh\\ij\nkl";
        let tl_id = format!("tl:{label}:{}", sanitize_literal(id));
        assert_eq!(tl_id, "tl:Memory:abcdefghijkl");
    }

    #[test]
    fn sanitize_literal_clean_passthrough() {
        assert_eq!(sanitize_literal("cortex:main"), "cortex:main");
        assert_eq!(sanitize_literal("ADR-018"), "ADR-018");
    }

    #[test]
    fn sanitize_literal_drops_forbidden_chars() {
        assert_eq!(sanitize_literal("a'b\"c\\d\ne\rf"), "abcdef");
    }

    #[test]
    fn parse_rfc3339_accepts_full_timestamp() {
        let out = parse_rfc3339_or_date(Some("2026-05-01T10:00:00Z")).unwrap();
        assert!(out.is_some());
    }

    #[test]
    fn parse_rfc3339_expands_date_only() {
        let day = parse_rfc3339_or_date(Some("2026-05-01")).unwrap().unwrap();
        let full = parse_rfc3339_or_date(Some("2026-05-01T00:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(day, full);
    }

    #[test]
    fn parse_rfc3339_returns_none_for_empty() {
        assert!(parse_rfc3339_or_date(None).unwrap().is_none());
        assert!(parse_rfc3339_or_date(Some("")).unwrap().is_none());
        assert!(parse_rfc3339_or_date(Some("  ")).unwrap().is_none());
    }

    #[test]
    fn parse_rfc3339_rejects_garbage() {
        assert!(parse_rfc3339_or_date(Some("not-a-date")).is_err());
    }
}

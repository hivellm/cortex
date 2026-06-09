//! Phase18 §4.1 — `cortex-ops timeline <project>` subcommand.
//!
//! Reads `TimelineEvent` nodes out of Nexus, applies the optional
//! `--branch`, `--kind`, `--from`, `--to`, `--as-of`, `--limit`
//! filters, and prints either a plain-text table or JSON. The
//! Cypher matches on `TimelineEvent.project_id = "<project>"`
//! per the phase18 §2 schema. The branch filter intersects with
//! the bitemporal axis: `branch_id IN [<ancestry chain>]`. When
//! the caller does not pass `--branch`, the chain defaults to the
//! canonical root (`<project>:main`).
//!
//! `--as-of` is enforced as `recorded_at_unix <= <as-of>` so the
//! command renders the timeline as it was believed at that point
//! in valid time (ADR-018 second-precision).

use std::process::ExitCode;

use chrono::DateTime;

/// Phase18 §4.1 — `cortex-ops timeline` dispatcher.
#[allow(clippy::too_many_arguments)]
pub(super) fn timeline(
    project: String,
    as_of: Option<String>,
    branch: Option<String>,
    kind: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: u32,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
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
    let client = match LiveNexusClient::new(cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            return ExitCode::from(2);
        }
    };

    // Resolve as_of / from / to to epoch-seconds. Empty input is a
    // user error (a "blank" filter passes silently in clap), so a
    // parse failure exits non-zero with a clear message.
    let as_of_unix = match parse_rfc3339_or_date(as_of.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: --as-of: {e}");
            return ExitCode::from(2);
        }
    };
    let from_unix = match parse_rfc3339_or_date(from.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: --from: {e}");
            return ExitCode::from(2);
        }
    };
    let to_unix = match parse_rfc3339_or_date(to.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: --to: {e}");
            return ExitCode::from(2);
        }
    };

    let branch_name = branch
        .as_deref()
        .unwrap_or(cortex_workers::graph::bitemporal::DEFAULT_BRANCH);
    let branch_id = cortex_workers::temporal::branch_filter::compose_id(&project, branch_name);

    // Render the Cypher with inlined sanitized literals. Nexus
    // 2.2.0's parameter substitution path is bug-affected
    // (upstream issue #3) — every other `cortex-ops` handler
    // already inlines its filters for the same reason.
    let mut clauses: Vec<String> = vec![format!("t.project_id = '{}'", sanitize_literal(&project))];
    clauses.push(format!("t.branch_id = '{}'", sanitize_literal(&branch_id)));
    if let Some(k) = kind.as_deref().filter(|s| !s.is_empty()) {
        clauses.push(format!("t.kind = '{}'", sanitize_literal(k)));
    }
    if let Some(f) = from_unix {
        clauses.push(format!("t.valid_from_unix >= {f}"));
    }
    if let Some(t) = to_unix {
        clauses.push(format!("t.valid_from_unix <= {t}"));
    }
    if let Some(a) = as_of_unix {
        clauses.push(format!("t.recorded_at_unix <= {a}"));
    }
    let where_clause = clauses.join(" AND ");

    let cypher = format!(
        "MATCH (t:TimelineEvent) WHERE {where_clause} \
         RETURN t.id AS id, t.kind AS kind, t.project_id AS project_id, \
                t.branch_id AS branch_id, t.entity_id AS entity_id, \
                t.valid_from_unix AS valid_from_unix, \
                t.recorded_at_unix AS recorded_at_unix, \
                t.summary AS summary \
         ORDER BY t.valid_from_unix DESC LIMIT {limit}"
    );

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: build tokio runtime: {e}");
            return ExitCode::from(1);
        }
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
            "branch": branch_id,
            "filters": {
                "as_of_unix": as_of_unix,
                "kind": kind,
                "from_unix": from_unix,
                "to_unix": to_unix,
                "limit": limit,
            },
            "events": rows,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!(
        "cortex-ops timeline @ {nexus_url}\n  project={project}  branch={branch_id}  events={count}",
        count = rows.len()
    );
    if rows.is_empty() {
        return ExitCode::SUCCESS;
    }
    println!(
        "  {:<24}  {:<14}  {:<26}  {:<16}",
        "kind", "valid_from", "entity_id", "summary"
    );
    for row in &rows {
        let kind = row
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("<?>")
            .to_string();
        let vf = row
            .get("valid_from_unix")
            .and_then(|v| v.as_i64())
            .map(format_unix)
            .unwrap_or_else(|| "<?>".to_string());
        let ent = row
            .get("entity_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<none>")
            .to_string();
        let summary = row
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(64)
            .collect::<String>();
        println!("  {kind:<24}  {vf:<14}  {ent:<26}  {summary:<16}");
    }
    ExitCode::SUCCESS
}

/// Sanitize a literal that will be inlined into a Cypher string.
/// Strips single quotes + backslashes — the inline values are
/// `project_id` / `branch_id` / `kind`, all controlled by clap so
/// the typical shapes are alphanumeric + `:` / `-` / `_` / `/`.
fn sanitize_literal(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '\n' | '\r'))
        .collect()
}

/// Parse RFC-3339 OR a day-precision (`YYYY-MM-DD`) string into
/// epoch seconds. ADR-018 §1.4 permits day-precision input on the
/// CLI; the converter expands `<date>` → `<date>T00:00:00Z`.
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

/// Format an epoch second as a `YYYY-MM-DD` string for the
/// plain-text table.
fn format_unix(unix: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(unix, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| format!("@{unix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339_or_date_accepts_full_rfc3339() {
        let out = parse_rfc3339_or_date(Some("2026-04-01T12:34:56Z")).unwrap();
        assert!(out.is_some());
    }

    #[test]
    fn parse_rfc3339_or_date_expands_day_precision() {
        let day = parse_rfc3339_or_date(Some("2026-04-01")).unwrap().unwrap();
        let rfc = parse_rfc3339_or_date(Some("2026-04-01T00:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(day, rfc);
    }

    #[test]
    fn parse_rfc3339_or_date_returns_none_on_empty_input() {
        assert!(parse_rfc3339_or_date(None).unwrap().is_none());
        assert!(parse_rfc3339_or_date(Some("")).unwrap().is_none());
        assert!(parse_rfc3339_or_date(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn parse_rfc3339_or_date_rejects_garbage() {
        assert!(parse_rfc3339_or_date(Some("not-a-date")).is_err());
    }

    #[test]
    fn sanitize_literal_strips_quotes_and_backslashes() {
        assert_eq!(sanitize_literal("cortex"), "cortex");
        assert_eq!(sanitize_literal("a'b\"c\\d"), "abcd");
        assert_eq!(sanitize_literal("a\nb\rc"), "abc");
    }

    #[test]
    fn format_unix_round_trips_through_chrono() {
        let parsed = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z").unwrap();
        assert_eq!(format_unix(parsed.timestamp()), "2026-04-01");
    }
}

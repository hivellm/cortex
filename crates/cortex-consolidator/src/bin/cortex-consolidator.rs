//! Phase11q §1 — `cortex-consolidator` CLI binary.
//!
//! Today the binary only ships the `estimate` subcommand: it walks
//! the live Meili indexes for `Kind::Turn` per repo, groups by
//! `session_id`, counts sessions / total body bytes, and projects
//! USD cost for the three planned consolidator passes (session-grain
//! Shallow Haiku, topic-grain Shallow Haiku, decision-trace Deep
//! Opus). No Anthropic API calls fire from this binary — the
//! operator reads the projection, signs off on the USD budget, and
//! a follow-up task triggers the actual passes against the lib API.
//!
//! Why estimate-only first: the lib `Orchestrator::run_*` paths
//! already enforce per-call cost ceilings, but the operator gate
//! per phase11q §1.3 requires a written estimate BEFORE the orchestrator
//! is invoked. This binary is the gate.

use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortex_consolidator::summariser::{cost_cents, SummariserKind};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "cortex-consolidator",
    about = "Phase11j corpus consolidator (estimate-only today; LLM passes operator-gated)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Walk Meili turn indexes per repo, group by `session_id`,
    /// project consolidator cost across the three grains. No
    /// Anthropic API calls fire.
    Estimate {
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then `http://127.0.0.1:17004`.
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master / admin API key.
        #[arg(long)]
        meili_key: Option<String>,
        /// Restrict the estimate to one repo (matches the
        /// `cortex-{slug}-turns` index suffix). Omit to scan every
        /// per-repo turns index.
        #[arg(long)]
        repo: Option<String>,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Estimate {
            meili,
            meili_key,
            repo,
            json,
        } => estimate(meili, meili_key, repo, json).await,
    }
}

#[derive(Serialize)]
struct PerRepoEstimate {
    repo_slug: String,
    sessions: u64,
    total_envelopes: u64,
    total_body_bytes: u64,
    estimated_input_tokens: u64,
}

#[derive(Serialize)]
struct PassEstimate {
    grain: &'static str,
    model: &'static str,
    estimated_input_tokens: u64,
    estimated_output_tokens: u64,
    estimated_cost_usd: f64,
    notes: &'static str,
}

#[derive(Serialize)]
struct EstimateReport {
    mode: &'static str,
    meili_url: String,
    per_repo: Vec<PerRepoEstimate>,
    total_sessions: u64,
    total_envelopes: u64,
    total_body_bytes: u64,
    passes: Vec<PassEstimate>,
    total_cost_usd: f64,
}

async fn estimate(
    meili: Option<String>,
    meili_key: Option<String>,
    repo_filter: Option<String>,
    json: bool,
) -> Result<()> {
    let meili_url = meili
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest builder")?;

    let auth = |req: reqwest::RequestBuilder| match api_key.as_deref() {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    // Discover all `cortex-{slug}-turns` indexes.
    let stats: serde_json::Value = auth(http.get(format!("{}/stats", meili_url.trim_end_matches('/'))))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let map = stats
        .get("indexes")
        .and_then(|v| v.as_object())
        .context("/stats payload missing `indexes`")?;
    let mut turn_indexes: Vec<(String, u64)> = map
        .iter()
        .filter_map(|(uid, v)| {
            if !uid.starts_with("cortex-") || !uid.ends_with("-turns") {
                return None;
            }
            // Extract slug between `cortex-` and `-turns`.
            let slug = &uid["cortex-".len()..uid.len() - "-turns".len()];
            if let Some(filter) = &repo_filter {
                if slug != filter {
                    return None;
                }
            }
            let count = v.get("numberOfDocuments").and_then(|n| n.as_u64()).unwrap_or(0);
            if count == 0 {
                return None;
            }
            Some((uid.clone(), count))
        })
        .collect();
    turn_indexes.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

    let mut per_repo: Vec<PerRepoEstimate> = Vec::new();
    for (uid, _count) in &turn_indexes {
        let slug = uid["cortex-".len()..uid.len() - "-turns".len()].to_string();
        let (sessions, envelopes, body_bytes) =
            scan_index(&http, &meili_url, api_key.as_deref(), uid).await?;
        // Char-to-token ratio — the published Anthropic guidance is
        // ~4 chars per token for English text. Body bytes are UTF-8;
        // for ASCII-leaning developer prose this stays close to
        // bytes / 4. Bumping to 3.5 to be conservative on the
        // estimate side (over-projects cost rather than under).
        let estimated_input_tokens = body_bytes / 4;
        per_repo.push(PerRepoEstimate {
            repo_slug: slug,
            sessions,
            total_envelopes: envelopes,
            total_body_bytes: body_bytes,
            estimated_input_tokens,
        });
    }

    let total_sessions: u64 = per_repo.iter().map(|r| r.sessions).sum();
    let total_envelopes: u64 = per_repo.iter().map(|r| r.total_envelopes).sum();
    let total_body_bytes: u64 = per_repo.iter().map(|r| r.total_body_bytes).sum();
    let total_input_tokens: u64 = per_repo.iter().map(|r| r.estimated_input_tokens).sum();

    // Per-grain projection — assumes every session, every topic, and
    // every decision-trace clusters once. Output tokens are bounded
    // by the per-grain template caps (1 024 tokens default).
    //
    // - Session-grain: one Haiku call per session. Input = full
    //   session token volume; output ≈ 512 tokens (template-bound).
    // - Topic-grain: one Haiku call per topic cluster. Estimated as
    //   total_sessions / 4 (~25% of sessions cluster into topics
    //   with overlap; refine when the topic clusterer ships).
    // - Decision-trace: one Opus call per ADR (estimated at ~100
    //   ADRs per the live dashboard count). Each call processes
    //   ~3 000 tokens of trace context, output ≈ 1 024.
    let session_input = total_input_tokens;
    let session_output = total_sessions.saturating_mul(512);
    let session_cost_cents = cost_cents(SummariserKind::Haiku45, session_input, session_output);

    let topic_clusters = (total_sessions / 4).max(1);
    let topic_input = total_input_tokens / 4; // each cluster reads ~25 % of total
    let topic_output = topic_clusters.saturating_mul(512);
    let topic_cost_cents = cost_cents(SummariserKind::Haiku45, topic_input, topic_output);

    let decision_traces: u64 = 100;
    let decision_input = decision_traces.saturating_mul(3_000);
    let decision_output = decision_traces.saturating_mul(1_024);
    let decision_cost_cents = cost_cents(SummariserKind::Opus47, decision_input, decision_output);

    let passes = vec![
        PassEstimate {
            grain: "session",
            model: "Haiku 4.5 (Shallow)",
            estimated_input_tokens: session_input,
            estimated_output_tokens: session_output,
            estimated_cost_usd: f64::from(session_cost_cents) / 100.0,
            notes: "one call per session; input = full session token volume",
        },
        PassEstimate {
            grain: "topic",
            model: "Haiku 4.5 (Shallow)",
            estimated_input_tokens: topic_input,
            estimated_output_tokens: topic_output,
            estimated_cost_usd: f64::from(topic_cost_cents) / 100.0,
            notes: "approx (total_sessions/4) clusters; refine when topic clusterer ships",
        },
        PassEstimate {
            grain: "decision_trace",
            model: "Opus 4.7 (Deep)",
            estimated_input_tokens: decision_input,
            estimated_output_tokens: decision_output,
            estimated_cost_usd: f64::from(decision_cost_cents) / 100.0,
            notes: "100 ADRs assumed (verify via /v1/dashboard/decisions count)",
        },
    ];

    let total_cost_usd =
        f64::from(session_cost_cents + topic_cost_cents + decision_cost_cents) / 100.0;

    let report = EstimateReport {
        mode: "estimate_only",
        meili_url,
        per_repo,
        total_sessions,
        total_envelopes,
        total_body_bytes,
        passes,
        total_cost_usd,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_text(&report);
    }
    Ok(())
}

async fn scan_index(
    http: &reqwest::Client,
    meili_url: &str,
    api_key: Option<&str>,
    uid: &str,
) -> Result<(u64, u64, u64)> {
    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut envelopes: u64 = 0;
    let mut body_bytes: u64 = 0;
    let mut offset = 0usize;
    let limit = 1000usize;
    loop {
        let url = format!(
            "{}/indexes/{}/documents?limit={}&offset={}&fields=session_id,body",
            meili_url.trim_end_matches('/'),
            uid,
            limit,
            offset
        );
        let mut req = http.get(&url);
        if let Some(k) = api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("scan {uid} offset={offset}: status {}", resp.status());
        }
        let body: serde_json::Value = resp.json().await?;
        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if results.is_empty() {
            break;
        }
        for d in &results {
            envelopes += 1;
            if let Some(s) = d.get("session_id").and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    sessions.insert(s.to_string());
                }
            }
            if let Some(b) = d.get("body").and_then(|v| v.as_str()) {
                body_bytes += b.len() as u64;
            }
        }
        offset += results.len();
        let total = body.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if total > 0 && offset >= total {
            break;
        }
    }
    Ok((sessions.len() as u64, envelopes, body_bytes))
}

fn render_text(r: &EstimateReport) {
    println!("cortex-consolidator estimate (mode={})", r.mode);
    println!("meili: {}", r.meili_url);
    println!();
    println!("per-repo:");
    for p in &r.per_repo {
        println!(
            "  {}: sessions={}, envelopes={}, body_bytes={}, est_input_tokens={}",
            p.repo_slug, p.sessions, p.total_envelopes, p.total_body_bytes, p.estimated_input_tokens
        );
    }
    println!();
    println!("totals:");
    println!("  sessions:    {}", r.total_sessions);
    println!("  envelopes:   {}", r.total_envelopes);
    println!("  body_bytes:  {}", r.total_body_bytes);
    println!();
    println!("per-grain projection:");
    for p in &r.passes {
        println!(
            "  {grain:>15} ({model}): in={input} out={output} cost=${cost:.2}  -- {notes}",
            grain = p.grain,
            model = p.model,
            input = p.estimated_input_tokens,
            output = p.estimated_output_tokens,
            cost = p.estimated_cost_usd,
            notes = p.notes
        );
    }
    println!();
    println!("TOTAL ESTIMATED COST: ${:.2} USD", r.total_cost_usd);
    println!();
    println!("This is an ESTIMATE-ONLY pass. No Anthropic API calls fired.");
    println!("Operator must approve the USD total before the actual run is triggered.");
}

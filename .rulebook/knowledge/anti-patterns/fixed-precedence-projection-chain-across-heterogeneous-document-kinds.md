# Fixed-precedence projection chain across heterogeneous document kinds

**Category**: retrieval
**Tags**: analysis:relevance, phase6g, F-009, meili, projection, anti-pattern

## Description

When a search lane projects upstream documents into a unified hit shape, do NOT use a single fixed precedence chain (`summary > title > body`) when the documents have heterogeneous shapes per `kind`. Curated kinds (decisions, analyses, memories) have a real `summary` and benefit from `summary`-first; raw artifact kinds (code/doc files) have empty summaries and the file path as `title`, so the chain stops at the path and the actual searchable content (`body`) never reaches the snippet text. The bug is invisible from the index — Meili / Vectorizer rank correctly against `body` — but the projected snippet text is wrong, masking the right document behind a path string. Fix: switch to a kind-aware chain (`projection_chain(kind, &summary, &title, &body)`) that picks the most-content-bearing field per kind. Test guard: pin the artifact-with-body case so a regression to the fixed chain blows up immediately. Discovered in cortex 2026-04-28: `free_search "JWT refresh"` returned `main.rs` and `docker-compose.yml` but missed `vectorizer_lane.rs` — the only file containing `LoginCreds` / `refresh_token` — because every artifact hit projected `text = "<path>"`.

## Example

// Anti-pattern (pre-phase6g):
let text = doc.summary.or(doc.title).or(doc.body).unwrap_or_default();
// `kind=artifact` always lands as `text = "<path>"`.

// Fix (phase6g):
fn projection_chain<'a>(kind: Option<&str>, summary: &'a Option<String>,
                        title: &'a Option<String>, body: &'a Option<String>)
    -> [&'a Option<String>; 3] {
    match kind {
        Some("artifact") | Some("law_violation") => [body, summary, title],
        Some("decision") | Some("analysis") | Some("memory") => [summary, title, body],
        Some("turn") | Some("tool_call") | Some("agent_call") => [summary, body, title],
        _ => [summary, title, body],
    }
}

## When NOT to Use

When all document kinds genuinely have the same field semantics. The kind-aware chain only pays off when the per-kind shapes differ (which they almost always do once you index more than one event source).

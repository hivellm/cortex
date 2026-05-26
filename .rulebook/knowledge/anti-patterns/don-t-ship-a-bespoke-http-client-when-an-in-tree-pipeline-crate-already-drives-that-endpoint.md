# Don't ship a bespoke HTTP client when an in-tree pipeline crate already drives that endpoint

**Category**: architecture
**Tags**: adapter, contract-bug, wire-shape, pre-thinking, spec-10, spec-12

## Description

In a polyrepo / multi-crate workspace, a "thin" hand-rolled HTTP client in adapter A targeting endpoint E will silently drift from the type-safe definition of E maintained in crate C the moment C changes. Two artifacts of E — the wire shape and the serializer / deserializer — must be authored in exactly one place. Adapters consume them via dependency, not by re-declaring them.

Concrete instance: `cortex-adapter-claude-code` had a private `PreThinkingRequest { prompt, session_id, cwd, max_bundle_bytes }` and POSTed to `/v1/query?intent=pre_change_context`. The actual API consumes `Json<cortex_api::QueryRequest> { intent, query, scope, limit, k, include, budget_ms }` and ignores URL query parameters. Result: 100 % of pre-thinking calls 422'd, the adapter fail-opened, and the model never saw enrichment — even though every component on the API side was healthy.

Same lesson applies to response shape. Claude Code's hook contract uses camelCase nested under `hookSpecificOutput`; serializing snake_case at the top level produces a JSON blob the harness silently ignores.

## Example

// Wrong — adapter declares its own shape, drifts from the API:
#[derive(Serialize)]
struct PreThinkingRequest<'a> { prompt: &'a str, session_id: &'a str, cwd: Option<&'a str>, max_bundle_bytes: u64 }
client.post("/v1/query?intent=pre_change_context").json(&req).send().await; // 422 forever

// Right — depend on cortex-api + cortex-pre-thinking, drive the pipeline:
let query_fn = Arc::new(ClosureQueryFn(move |req: cortex_api::QueryRequest| async move {
    http.post(&url).json(&req).send().await.ok()?.json::<QueryResponse>().await.ok()
}));
let output = cortex_pre_thinking::pipeline::run(&input, query_fn, metrics).await;
// output.bundle is the Markdown string, already clipped to budget

## When to Use

When an adapter / consumer talks to a service whose request and response types are already defined in a sibling crate, depend on that crate and use its types verbatim. If a budget / clipping / formatting pipeline (e.g. `cortex_pre_thinking::pipeline::run`) exists, drive it instead of reimplementing a degraded subset.

## When NOT to Use

External services with unstable schemas where you genuinely want to insulate the adapter from upstream churn — there, a per-adapter DTO is appropriate, but it must come with contract tests against a real instance, not just unit tests.

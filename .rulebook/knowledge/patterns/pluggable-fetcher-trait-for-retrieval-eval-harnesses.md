# Pluggable fetcher trait for retrieval-eval harnesses

**Category**: testing
**Tags**: analysis:relevance, phase6e, testing, harness, F-008

## Description

Split the relevance harness into (a) a `SnippetFetcher` async trait that returns `QueryResponse`, and (b) pure scoring functions over the response. Production runs ship an `HttpFetcher` (reqwest) implementation; unit + integration tests inject a `FakeFetcher` keyed by query id. This lets the recall@k / MRR math be unit-tested without booting cortex-api, and lets golden-shape tests assert the report end-to-end with no network.

## Example

#[async_trait::async_trait]
pub trait SnippetFetcher: Send + Sync {
    async fn fetch(&self, q: &LabeledQuery) -> Result<QueryResponse>;
    async fn status_snapshot(&self) -> StatusSnapshot { StatusSnapshot::all_healthy() }
}
// HttpFetcher hits /v1/query; FakeFetcher returns canned responses.

## When to Use

Any harness or replay tool that scores a model/service against a labeled fixture set — keep IO at the edge so the metric math stays trivially testable.

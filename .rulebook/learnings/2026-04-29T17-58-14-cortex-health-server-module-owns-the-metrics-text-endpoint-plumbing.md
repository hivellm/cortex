# cortex-health server module owns the /metrics text endpoint plumbing
**Source**: manual
**Date**: 2026-04-29
**Related Task**: phase8b_pipeline_stage_metrics
**Tags**: observability, prometheus, cortex-health, phase8b
When extending cortex-health::server to mount a Prometheus-text `/metrics` endpoint alongside `/healthz`, keep the renderer optional (`MetricsRenderer = Arc<dyn Fn() -> String + Send + Sync>`) and surface both `router_with_metrics()` (mergeable into existing axum apps like cortex-api / cortex-ingestion) and `serve_standalone_with_metrics()` (one-call helper for worker bins). Each crate owns its own renderer — cortex-health stays metrics-format-agnostic. The `/metrics` route is conditionally registered: when `metrics: None`, the route is absent (404) instead of returning an empty body, so an external scraper sees an explicit "not configured" signal. Tests verify both the present and absent cases.
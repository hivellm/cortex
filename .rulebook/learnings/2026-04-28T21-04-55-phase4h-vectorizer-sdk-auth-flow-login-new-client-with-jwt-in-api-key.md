# phase4h — vectorizer-sdk auth flow: login → new client with JWT in api_key
**Source**: manual
**Date**: 2026-04-28
**Related Task**: phase4h_doctor_vec_nexus_probes
**Tags**: phase4h, cortex-ops, doctor, vectorizer, nexus, auth
vectorizer-sdk 3.0.3 has no `set_token`/mutator on `VectorizerClient`. The auth flow is: build a pre-auth client (`api_key: None`) → call `client.login(user, pass)` → get back a `JwtToken { access_token }` → build a NEW client with `ClientConfig { api_key: Some(token.access_token), .. }`. The HTTP transport sniffs the three-segment JWT shape and sends it as `Authorization: Bearer ...`. cortex-embedder's `LiveVectorizerClient::login` already encoded this pattern; phase4h's doctor probe reuses it.

For the Nexus probe, wrap the existing `cortex_graph::LiveNexusClient` rather than reaching into `nexus-graph-sdk` directly — that client already handles transport selection (HTTP vs RPC) and auth env vars (`CORTEX_NEXUS_USER`/`_PASSWORD`) via `GraphConfig::from_env()`. Cypher rows arrive as `Vec<serde_json::Value>` of arrays, so destructure with `as_array()` + index `[0]` (repo name) / `[1]` (count).

For the coverage report, suspicious-vs-inconsistent priority matters: never let the ratio probe fire when the row is already inconsistent. A missing partition is the more urgent signal — keep the suspicious flag silent until the inconsistency is fixed.
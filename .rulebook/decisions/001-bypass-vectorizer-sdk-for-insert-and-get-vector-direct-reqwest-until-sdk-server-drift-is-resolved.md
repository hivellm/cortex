# 1. Bypass vectorizer-sdk for /insert and /get_vector — direct reqwest until SDK-server drift is resolved

**Status**: superseded
**Date**: 2026-04-22
**Related Tasks**: phase1_embedder

## Context

_No context provided._

## Decision

ORIGINAL (round 5, against SDK 3.0.0): bypass the SDK for `POST /insert` and `GET /collections/{c}/vectors/{id}` via direct `reqwest`; keep the SDK for `get_collection_info`, `create_collection`, `delete_collection`, `search_vectors`.

**SUPERSEDED** in round 7 by the SDK 3.0.3 upgrade. Two of the three bypass paths have been deleted:

- **Login**: `LiveVectorizerClient::login` now wraps `SdkClient::login(user, password) -> JwtToken`. The `reqwest`-based `login_token()` helper in `tests/common/mod.rs` has been removed in favour of the SDK call.
- **Insert**: `LiveVectorizerClient::upsert_chunks` now calls `self.sdk.insert_texts(collection, batch)` directly. The hand-rolled `insert_one` → `POST /insert` path has been deleted. `BatchResponse` tolerates both pre-v3 and v3 server response shapes via `serde(alias)`.

**STILL BYPASSED** (server bug, not SDK bug): `GET /collections/{c}/vectors/{id}` still returns a synthetic 200 for any id, so `VectorizerClient::exists` drives `GET /collections/{c}/vectors?limit+offset` via a single `reqwest` path (`LiveVectorizerClient::list_stored_chunk_ids`) and intersects against `payload.chunk_id` from the list response. SDK 3.0.3 does not yet expose `list_vectors` — when it does, this workaround can be deleted too. Tracking as knowledge entry `vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side`.

## Consequences

_No consequences documented._

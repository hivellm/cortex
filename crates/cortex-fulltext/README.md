# cortex-fulltext

> Spec: [`docs/specs/08-fulltext-indexer.md`](../../docs/specs/08-fulltext-indexer.md)

The Meilisearch indexer for Cortex. Consumes
`cortex.events.enriched` and projects each event into a per-`(repo,
family)` Meilisearch index, with typo-tolerance, faceted filters,
and the source-attribution invariant the dashboard's keyword lane
depends on.

```
cortex.events.enriched ──▶ cortex-fulltext-worker ──▶ Meilisearch
                                                     cortex-{repo}-{family}
```

## Index naming

Every Meili index follows `cortex-{repo_slug}-{family}` so each
project lives in isolation and queries scope deterministically to
one repo. The family is derived from the event:

| Family       | Kinds routed in                                                   |
|--------------|-------------------------------------------------------------------|
| `code`       | `tool_call`, plus `artifact.*` whose path/topic looks code-shaped |
| `docs`       | `artifact.*` whose path/topic looks doc-shaped                    |
| `decisions`  | `decision`                                                        |
| `turns`      | `turn`, `agent_call`                                              |
| `governance` | `law_violation`                                                   |
| `analyses`   | `analysis`                                                        |
| `misc`       | catch-all                                                          |

`Kind::Artifact` events read `context_path` first (for the file
extension) and fall back to `classifier.topics` to decide between
`code` and `docs` — the kind-only path drops to `misc` honestly
rather than silently piling everything into `docs`.

## Configuration

| Variable                     | Default                            | Notes                                              |
|------------------------------|------------------------------------|----------------------------------------------------|
| `CORTEX_FULLTEXT_SYNAP_URL`  | `http://127.0.0.1:15003`           | Synap base URL.                                    |
| `CORTEX_FULLTEXT_MEILI_URL`  | `http://127.0.0.1:15004`           | Meilisearch base URL.                              |
| `CORTEX_FULLTEXT_MEILI_KEY`  | `cortex-dev-master-key`            | Meili master key.                                  |
| `CORTEX_FULLTEXT_PREFIX`     | `cortex-`                          | Per-deployment namespace prefix.                   |
| `CORTEX_FULLTEXT_BATCH`      | `64`                               | Documents per Meili upsert batch.                  |

## Run

```bash
# from a checkout of the Cortex repo
cargo run --release -p cortex-fulltext --bin cortex-fulltext-worker
```

Boot seeds the legacy family set
(`cortex-{family}` without a slug) so old client code that hardcodes
those names still finds an index. Per-`(repo, family)` indexes are
materialised lazily on the first upsert via
`MeiliFulltextIndexer::ensure_settings`.

## Settings

The crate ships a baked-in settings document
([`src/settings.rs`](src/settings.rs) → `SETTINGS_V1`) applied to
every index it creates. Only Meili-recognised keys reach the wire;
tooling-only fields (e.g. `"version": "v1"`) are stripped at the
client boundary.

## Body construction

The `body` field on each Meili document follows the spec-08 contract:

- raw text below the cap is used directly,
- when the raw text is oversize, the classifier `summary` is used,
- when neither is available, the raw text is truncated at a UTF-8
  boundary up to the cap.

This keeps full-text search useful even for events whose payload was
too large to ship verbatim.

## Tests

```bash
cargo test -p cortex-fulltext
```

Routing matrix is unit-pinned: every `Kind` has a known family, and
the `analyses` index name is asserted per repo. Indexer integration
tests against a live Meili are gated on `CORTEX_IT=1`.

## Stability

Pre-1.0. Index naming is the public contract — the dashboard
read-side and the bootstrap fan-out both depend on the
`cortex-{repo}-{family}` shape. Renames go through a documented
migration.

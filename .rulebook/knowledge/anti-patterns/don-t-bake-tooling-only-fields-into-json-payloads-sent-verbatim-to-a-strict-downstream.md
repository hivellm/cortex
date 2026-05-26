# Don't bake tooling-only fields into JSON payloads sent verbatim to a strict downstream

**Category**: code
**Tags**: meilisearch, cortex-fulltext, settings, schema, boundary-stripping

## Description

cortex-fulltext shipped a `settings.v1.json` file that started with `{"version": "v1", ...}`. The same JSON was PATCHed verbatim to Meilisearch's `/indexes/{uid}/settings`, which rejects unknown top-level fields. The result: the worker hard-failed on every boot. The "version" tag was useful for the indexer's own loader (track schema upgrades) but was never meant for the wire. Either keep tooling-only fields in a sidecar (`settings.v1.json` + `settings.v1.meta.json`), or strip them in the client before transmission. We chose the strip-in-client fix because it kept the file shape backward-compatible.

## Example

// In cortex-fulltext::meili_client::ensure_index, before the PATCH:
let mut settings_owned = settings.clone();
if let Some(map) = settings_owned.as_object_mut() {
    map.remove("version"); // Meili rejects unknown top-level fields
}

## When to Use

When you have configuration JSON consumed by both your own loader and a strict third-party API: keep the tooling-only fields, but always strip them at the boundary before forwarding upstream.

## When NOT to Use

When the third party explicitly accepts arbitrary metadata (some APIs ignore unknown fields). In that case, the marker can pass through.

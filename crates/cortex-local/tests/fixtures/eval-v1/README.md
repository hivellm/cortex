# Public fictional retrieval corpus v1

License: CC0-1.0. All project names, records, incidents, commands and observations in `corpus.json` are invented for this evaluation. No user memory, production event, credential or private document was copied. Dates and numerical results describe fictional fixtures, not measured Cortex behavior.

## Authorship and freeze

The corpus and judgments were authored before executing searches or graph queries against the evaluation mechanism. Only Rust data contracts and existing envelope examples were inspected to produce valid inputs. No query result informed a relevance label. Keep this version immutable once results have been observed; make a separately named version for future judgment changes, recording their reason.

The split contains 12 development queries and eight held-out queries. Held-out means excluded from tuning against this fixture, not independently authored or externally blinded: the same agent wrote both partitions. Publishing the held-out text makes accidental contamination possible. Report that limitation and do not describe this small set as a general benchmark.

## Inputs and stable identity

- 18 memories span exact repositories `Atlas`, `atlas` and `AtlasLab`. Repeated titles and overlapping bodies make scope leakage visible.
- Accepted decisions and lessons coexist with two proposals and two superseded records. `supersedes_key` explicitly connects each accepted correction to its predecessor; source authority is stated separately rather than inferred from the link.
- Eight archive envelopes include four readable turns and four tool calls. Each retains original fictional prompts, answers, command arguments and output under its own `event_id`.
- Four graph capture requests carry explicit IDs, provenance and typed links. The fourth record shares export vocabulary but has no relationship to the connected three-node chain.
- Query keys resolve through memory `key`, event `event_id` and graph record `id`. The importer/harness records actual imported IDs and hashes from its documented identity contract; this fixture does not guess those IDs.

Pin the corpus bytes and source/payload hashes before execution. Canonical record hashes can vary across homes because timestamps and absolute source locators vary. Compare a documented stable projection for cross-home checks, retaining each actual source locator as provenance; do not claim full-record byte equality across homes.

`manifest.json` now pins those inputs, imported IDs and expected provenance. `daily.ndjson` contains the exact snapshot/evidence strings used to build an isolated legacy SQLite source; `events.ndjson` and `events.ndjson.zst` retain identical archive content. `graph.json` adds a source hash of each original graph request before that hash is filled. `scripts/generate_local_eval_fixtures.py` authors a new version without consulting Cortex and refuses to overwrite materialized files. Only regeneration requires Python `zstandard`; normal evaluation consumes committed bytes using the standard library. `.gitattributes` fixes text LF and treats compressed data as binary.

## Relevance judgments

`expected_keys` is an unordered set of records that directly answer the stated information need, not a promise that literal FTS retrieves every item. Evaluate retrieval quality against these fixed judgments, including misses. An accepted-decision intent excludes a mere proposal or a superseded predecessor even when their wording matches. Proposal lookup deliberately expects the proposed search record.

The broad development query `export` accepts all export evidence in exact `Atlas`, including the unconnected style-review graph record. A broad lexical topic request differs from graph connectivity. Historical-turn and historical-tool intents judge original archived observations rather than treating them as accepted decisions. Multiple-relevant questions expect both policy and directly corroborating historical evidence where listed.

Natural-language held-out questions deliberately include paraphrase, grammatical words, and vocabulary absent from source text. Literal AND search may miss these items. Preserve those failures as measured limits; do not shorten or rewrite queries after seeing results.

`empty=true` means the corpus provides no answer to the requested information need in that exact repository. In particular, the embedding-provider proposal supplies no approved price; retrieving it does not answer h08. Unknown repository `AtlasProduction` must not fall back to another scope. Scope and inactive-record violations should be reported separately from ordinary relevance misses.

## Graph judgments

Graph roots and edges are explicit, not inferred from text. Production traversal follows relationships in both directions while preserving edge orientation and labels. g01 expects one hop from report to log; g02 expects the two-hop report/log/test chain, preserving `derived_from` and `supports` labels. This fixture starts at outward-facing edges and does not independently exercise inbound traversal. The unconnected style review must stay absent. g03 asks for the same ID in lowercase `atlas` and expects no nodes or edges; an API-level not-found response may represent that absence, but must never expose the `Atlas` graph. These cases do not measure ranking, path optimality or truncation behavior.

Proposed records are active in general search. Returning a proposal for an accepted-decision question can be irrelevant, but is not by itself an inactive-record or API-contract violation. For h08, the proposal is evidence that approval is absent; it still does not supply the requested approved price.

## Coverage limits

This fixture covers exact scope, authority distractors, corrections, provenance-bearing historical observations, multiple relevant records, no-answer requests and a small typed graph. It contains English prose only, no adversarial instructions, no large corpus, no provider calls and no latency baseline. It tests retrieval evidence, not observed model use, classification accuracy or productivity. Quality metrics and thresholds belong to the separately versioned harness; they must not redefine these judgments to make the current engine pass.

# Phase34 section 1.12 - External review

## Kimi and Grok - authorized Rust snapshots

Date: 2026-10-17. The user explicitly authorized the selected Rust source to be
sent to the configured Kimi/Grok integrations. No environment files, credential
values, operator memories or other repository files were included. `schema.sql`
was excluded from the Kimi package because the authorization was Rust-only.

All full-file SHA-256 values matched the existing manifests. Every supplied
numbered line was also compared with its source file before transmission. Both
reviews used native Codex authentication and the configured `9router` provider,
with an external temporary working directory, ephemeral sessions, read-only
sandbox, explicit model IDs and a 240-second deadline per owned subprocess.

| Request | Rust files | Payload bytes | Result |
| --- | --- | ---: | --- |
| `kimi/kimi-k3` | storage `daily_memory/mod.rs`, `types.rs`, `delivery.rs` | 46,566 | Exit 0; 83.829 seconds; complete review |
| `xai/grok-4.6` | API `daily_memory.rs`, `ingest/proxy.rs`, `feedback.rs` | 52,828 | Exit 0; 193.746 seconds; complete review |

The five configured MCP servers were disabled per invocation. Complete JSONL
events contain no tool calls: each reviewer produced reasoning and its final
answer only. The CLI nevertheless attempted an automatic Context7 plugin OAuth
handshake, which failed; no source payload or model tool call went to Context7.
The CLI also reported unavailable model metadata and an ignored `features.skills`
override. These startup limitations are retained in raw stderr. Model identity
here denotes the explicit requested integration route, not independent attestation
of the upstream provider's internal routing.

No fallback HTTP review was needed. The separately authorized local 9router DB
was opened read-only only to inspect the API-key table's column names while Grok
was pending; no key value was read, printed, persisted or used by this reviewer.

### Artifact integrity

Raw prompts, manifests, JSONL events, stderr, timing records and full responses
remain under the ignored `.cortex-state/daily-runtime/reviews/` directory.
Authorized prompt files contain exactly the UTF-8 bytes sent through stdin.

| Artifact | SHA-256 |
| --- | --- |
| `kimi-authorized-prompt.txt` | `0ea355b8f16fe9c0ee40ae53d1485e964e28ebf11222aca9130ba3e3ad9100f1` |
| `kimi-response.txt` | `ff871b9ba39e8695664bd7de9b61e51f07f1afed611028e171bdfa6335800aa4` |
| `grok-authorized-prompt.txt` | `bfd29f4774b94e2a0707f533c2b17bad2ae20330fa5d0bcbb5a79cebc8e28a4c` |
| `grok-response.txt` | `f3dfc7c4146653e46c1d9766f3300535c84993a72d1a9e44f1ad58fecd220416` |

### Kimi adjudication

Kimi reported two P1 and two P2 candidates. No actionable defect was validated
from those candidates:

- Retry check/insert race: rejected. `daily_memory/mod.rs:108` starts
  `TransactionBehavior::Immediate` before the retry lookup, serializing competing
  writers. The proposed interleaving cannot occur inside these transactions.
- Case-sensitive or byte-exact repository identity: rejected as a defect.
  Exact identity is the declared scope contract; canonicalizing it at storage
  would merge distinct identities and weaken isolation.
- Global pins before repository pins: a possible ranking preference, not an
  established contract violation. Globals require explicit authorized inclusion;
  the ordinary pre-thinking path does not include them.
- Storage search limit 50 versus delivery limit 5: rejected as an interface bug.
  The API caps both its cache key and search request at five (`daily_memory.rs:58`
  and `daily_memory.rs:417`), then records only the memories actually rendered.

Kimi explicitly lacked SQL schema, API authorization and Jev implementation
context. Its raw response labels the retry claim proven despite overlooking the
supplied immediate transaction; the raw assertion is not adopted as a finding.

### Grok adjudication

Grok reported two P1 and three P2 candidates. One narrow concurrency defect is
supported by the repository code; the remainder is rejected or bounded by the
documented contract/context:

**P2 - Concurrent feedback writes can misreport `upserted`.**
`crates/cortex-api/src/feedback.rs:172` checks whether the row exists, then
`feedback.rs:182` performs a separate UPSERT. The new durable handler can run
these operations on separate concurrent SQLite connections. If two first posts
for the same `query_id` both finish their SELECT before either UPSERT, both return
`upserted=true`, although one overwrites the other. This contradicts the explicit
response-field contract at `feedback.rs:63`. The prior in-memory handler held its
shared metadata mutex across both operations. Confidence: high from the concrete
SQL interleaving; no runtime reproduction was performed by this reviewer.

Disposition: fixed after coordinator authorization. `record_feedback` now starts
an immediate SQLite transaction before the existence SELECT, performs the UPSERT
and commits before returning its result. Authorization and last-write-wins
feedback semantics are unchanged.

Regression `concurrent_file_backed_feedback_reports_exactly_one_insert` opens two
independent file-backed writer connections and holds a third connection's writer
reservation. Per-thread busy callbacks prove both requests reached contention
before the reservation is released. The former implementation would already have
read an absent row in both workers; the fixed implementation waits before either
existence check. Exactly one returned receipt must report `upserted=true`.
rustfmt and diff-check passed; root coordinator owns Cargo gates. The reviewed
source snapshot hashes describe the pre-fix input, not the subsequently corrected
feedback file.

Other Grok claims:

- A timeout can return 503 while blocking SQLite work later commits: true
  indeterminate-outcome behavior, not proof of lost data or unbounded workers.
  The permit stays with the worker, intentionally bounding in-flight work.
  Callers needing safe retries must retain their retry key; a missing HTTP
  acknowledgement is not proof that a write did not commit.
- Unauthenticated legacy feedback routes: not reachable through the production
  recovery router. `http.rs:216` and `http.rs:220` mount only durable handlers.
- Skipping an oversized memory prefix: a deliberate byte-budget rendering
  choice. No requirement to discard all subsequent fitting records was found.
- Receipt authorization/read versus later feedback write: receipt rows are
  insert-only; no supported concurrent receipt mutation exists. The separate
  `upserted` race above is valid, but the claimed receipt TOCTOU bypass is not.
- Optional correction revision on the API wire: storage validates that
  `supersedes` requires `expected_revision` and an accepted successor in
  `daily_memory/mod.rs:420`; no lost-update bypass was established.

Grok's package excluded storage, schema, router and Jev internals. Those omissions
explain several conditional claims. This report records independently checked
dispositions rather than treating external reviewer output as authority.

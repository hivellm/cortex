## 1. Bucketizer
- [ ] 1.1 NEW `crates/cortex-retention/src/turn_digest.rs`
- [ ] 1.2 `enumerate_buckets(now, after_days) -> Iterator<Bucket { repo, year_week, top_topic, event_ids: Vec<String> }>` reading from Parquet + the classifier topic table
- [ ] 1.3 Filter out buckets with fewer than `min_bucket_size` events (default 5) — single-turn weeks are not worth a digest call

## 2. Digest call
- [ ] 2.1 Add prompt template `digest_turns_to_memory` in `crates/cortex-classifier/src/prompts.rs` (system + user, ≤4 KB system)
- [ ] 2.2 `digest_bucket(bucket) -> DigestResult { body, tokens_in, tokens_out }` calls Sonnet via the existing classifier client
- [ ] 2.3 Truncate over-long input to a header sample + tail sample (preserve first/last 3 turns + 8 random middle turns) to keep token cost bounded
- [ ] 2.4 Validate output: 200–400 tokens, must mention `repo` and `year_week` (cheap regex check); on failure, retry once then mark bucket `failed`

## 3. Persistence
- [ ] 3.1 Emit a `cortex.events.enriched` event of `kind="memory"` with `memory_type="turn_digest"` and the digest body
- [ ] 3.2 Embed via the embedder worker; upsert into `cortex.memory.fp32`
- [ ] 3.3 Insert `(:Memory{memory_type:'turn_digest', repo, year_week, topic, body})` in Nexus
- [ ] 3.4 For each `event_id` in the bucket, add `(:Memory)-[:SUMMARIZES]->(:Turn{event_id})`
- [ ] 3.5 Tag source turns in the Parquet archive with `payload.summarized_by = <digest_event_id>` (Parquet rewrite uses the 9b helper)

## 4. Demotion hook
- [ ] 4.1 `--demote` flag: after a bucket is persisted, move the source turns from `cortex.turn.{fp32,pq}` straight to `cortex.cold.binary` (this is a fast-path of 9a)
- [ ] 4.2 Without `--demote`, only the digest is created; demotion happens on the next normal 9a sweep, which now sees `summarized_by != null` and treats those records as cold-eligible regardless of age

## 5. Budget + idempotence
- [ ] 5.1 Read `cortex.toml [retention.digest]` (`after_days=30`, `min_bucket_size=5`, `max_usd_cents_per_run=500`)
- [ ] 5.2 Track spend per call in `classifier_spend.day`
- [ ] 5.3 Stop cleanly when budget is exceeded, write `retention_sweeps.tier_transitions_json.turn_digest = { buckets_done, buckets_pending, usd_cents }`
- [ ] 5.4 Idempotence guard: a bucket that already has a matching `:Memory` node MUST NOT be re-summarized unless `--rebuild` is passed

## 6. Spec / docs
- [ ] 6.1 Add §"LLM turn digest" to `docs/specs/19-retention.md`
- [ ] 6.2 Reference from `docs/specs/05-classifier.md`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass

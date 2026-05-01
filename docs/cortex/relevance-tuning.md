# Relevance tuning

This page is the operator handbook for the phase11i relevance surfaces:

- the gold-set fixture at [`crates/cortex-api/tests/fixtures/relevance-gold.json`](../../crates/cortex-api/tests/fixtures/relevance-gold.json),
- the IT that drives it ([`crates/cortex-api/tests/relevance_eval_it.rs`](../../crates/cortex-api/tests/relevance_eval_it.rs)),
- and the tuning knobs in [`crates/cortex-api/config/relevance.toml`](../../crates/cortex-api/config/relevance.toml) (boot-loaded + SIGHUP-reloadable).

If the IT goes red, this doc walks you through diagnosing whether the regression is a fixture problem, a corpus problem, or a tuning problem.

---

## The gold set

`relevance-gold.json` carries 30 hand-curated questions paired with 1–3 acceptable result IDs. Recall@10 counts the question as satisfied when **any** entry from `expected_doc_ids` lands in the top-10 fused snippets.

| Intent | Count | Notes |
| --- | --- | --- |
| `pre_change_context` | 10 | Real change-prep prompts that should anchor on the touched file. |
| `decision_lookup` | 5 | "Why did we pick X?" prompts that should land on an ADR or the decision-bearing source. |
| `similar_problems` | 10 | Cross-cutting prompts that exercise fusion (multiple lanes contributing). |
| `law_check` | 3 | Prompts asking what rule applies — should surface the law spec or governance doc. |
| `free_search` | 2 | Open-ended prompts with no overlay — used to spot regressions in the keyword + vector blend. |

**Matchers.** Each `expected_doc_ids` entry is tried in order against every snippet's `(repo, path, content_hash)` composite, then exact `repo` / `path` / `symbol` / `content_hash` / `collection`, then a substring against `path` or `symbol`. The substring fallback is the curator's friend: write `crates/cortex-api/src/fusion.rs` and the matcher accepts every chunk-hash variation of that path.

### Adding a question

1. Pick the intent that matches the prompt (don't add a `pre_change_context` entry for an "explain how X works" prompt — use `free_search` or `explain` instead).
2. Pin a stable `id` (`rel-NNN`, monotonic). Never recycle ids — the report tooling diffs by id.
3. Write the prompt the way an operator would type it. Do **not** include filenames in the prompt unless you are explicitly testing the file-mention path.
4. List 1–3 `expected_doc_ids`. Use partial paths (`crates/cortex-api/src/fusion.rs`) over composite ids — the substring matcher keeps the entry stable through chunk rehashes.
5. Set `scope.repo` to the repo where the answer should land. The IT honours scope verbatim — wrong scope means the lane filter excludes the corpus you wanted to hit.
6. Re-run the IT (see below). The new entry must pass before merge.

### Removing or rewriting a question

Don't. The IT diffs MRR@10 over time; mutating an entry rebases the graph. If a question is genuinely obsolete (e.g. the underlying file was deleted), retire the id and add a new one with a fresh number — never reuse `rel-NNN`.

---

## Running the IT

```bash
# 1. Boot the live stack (Meili + Synap + cortex-api + a populated corpus).
docker-compose up -d cortex-api meili synap

# 2. Index the repos the gold set expects to find.
cargo run -p cortex-cli -- bootstrap --repo cortex --root .

# 3. Run the IT.
CORTEX_RELEVANCE_IT=1 cargo test -p cortex-api --test relevance_eval_it -- --nocapture
```

Override the daemon URL with `CORTEX_API_URL` when the daemon is not on the spec-11 default `127.0.0.1:17000`.

Without `CORTEX_RELEVANCE_IT=1`, the IT prints a one-line skip notice and the suite stays green — vanilla `cargo test` runs do not touch the network.

### Acceptance gate

The IT panics with the rolled-up score when MRR@10 falls below **0.75**. Per-intent MRR is logged for triage but is not asserted independently — a single weak intent can drag the mean below the gate, which is by design.

### Reading the output

```text
relevance gold-set: n=30 recall@10=0.867 mrr@10=0.793 ndcg@10=0.812
  pre_change_context: mrr@10=0.860 (n=10)
  decision_lookup:    mrr@10=0.700 (n=5)
  similar_problems:   mrr@10=0.825 (n=10)
  law_check:          mrr@10=0.667 (n=3)
  free_search:        mrr@10=0.750 (n=2)
misses (4/30):
  - rel-014 (intent=decision_lookup): no match in top-10; query="…" expected=[…]
  - …
```

- **`recall@10`** — share of questions whose top-10 contained any expected id. The cheapest "is the right doc anywhere in the bundle?" check.
- **`mrr@10`** — mean reciprocal rank. `1.0` means every hit was rank 1; `0.5` means rank 2 on average.
- **`ndcg@10`** — discounts hits by `1 / log2(rank+1)`. Penalises pushing the right doc to the bottom of the bundle.
- **misses** — every question that fell out of the top-10. Triage these first; recall regressions usually point at the corpus or scope, not the ranker.

---

## When to re-tune `relevance.toml`

The file at [`crates/cortex-api/config/relevance.toml`](../../crates/cortex-api/config/relevance.toml) carries every multiplier the fusion path reads. The daemon loads it at boot and re-reads on SIGHUP (Unix only — Windows builds log a one-shot WARN and stay at boot-time values).

Re-tune when **and only when**:

1. **Recall is fine, MRR is bad.** The right doc is in the bundle but ranked too low. Candidates:
   - Bump `[fusion].alpha` toward 0 to weight native lane scores more.
   - Bump per-intent recency λ in `[recency]` if the misses are old documents drowning out recent ones.
   - Bump `[session].same_session_boost` if the misses are within the active session.
2. **Decisions are demoted.** `decision_lookup` MRR is the canary — if it slips while everything else holds, raise `[recency].decision_lookup` carefully (decisions are sticky; don't let recency dominate).
3. **Cross-repo misses.** Foreign-repo hits keep getting collapsed. Raise `[cross_repo].boost` from `0.0` (legacy hard-filter) toward something modest like `0.3` — `1.0` lets every cross-repo hit compete on equal footing, which usually over-rotates.
4. **Outcome-tagged turns are noisy.** If `success` turns are not bubbling up, raise `[outcome].success` (default `1.2`) toward `1.5`. Conversely, if `error` turns are surfacing too aggressively, lower `[outcome].error` (default `0.5`) toward `0.3`.

Re-tune **never** when:

- The IT is red because the corpus is missing. Re-index instead — `cortex-bootstrap` over the affected repos.
- The IT is red because the gold-set entry was wrong. Fix the entry. Don't tune away a typo.
- A single intent is the entire failure. Add gold-set entries that cover the failure mode before changing any multiplier.

### Reload workflow

```bash
# Edit the config.
$EDITOR crates/cortex-api/config/relevance.toml

# Signal the running daemon (Unix).
pkill -SIGHUP -f cortex-api
# or, when running under systemd:
systemctl kill --signal=HUP cortex-api

# Watch the daemon log for "relevance config reloaded (SIGHUP)".
journalctl -u cortex-api --since "1 minute ago" | rg relevance
```

The daemon also stamps `alpha`, `k`, `cross_repo_boost`, `same_session_boost`, and `cohort_session_boost` on every audit envelope, so the dashboard's audit timeline shows when a reload took effect and what the active values were per query.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| IT skips with "gate not set" | `CORTEX_RELEVANCE_IT` env var missing | Set `CORTEX_RELEVANCE_IT=1` and re-run. |
| `POST /v1/query …` failures | Daemon not running or firewalled | Confirm `cortex-api` is up at `CORTEX_API_URL`. |
| Recall is `0.0` | Corpus not indexed for the gold set's `scope.repo` | Run `cortex-bootstrap` over the missing repo. |
| MRR drops after a config edit | Tuning regression | `git revert` the `relevance.toml` change, send SIGHUP, re-run the IT. |
| One intent is the entire delta | Coverage gap | Add 2–3 entries that exercise the failure mode before re-tuning. |
| `cargo fmt` reformats your fixture | Don't commit the reformat | The JSON fixture is hand-curated — `cargo fmt` should not touch it; reject the change. |

---

## Related

- Spec 11 — `docs/specs/11-query-api.md` (fusion algorithm + scope shape).
- Spec 12 — `docs/specs/12-pre-thinking-injection.md` (renderer + clipper that consumes the fused output).
- Phase11i task tree — `.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/`.

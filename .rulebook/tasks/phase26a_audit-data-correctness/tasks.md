## §1. Bug #1 — agent_call events dropped (description minLength)

- [ ] §1.1 Locate the SubagentStop envelope builder in `cortex-adapter-claude-code` and identify how `description` is populated
- [ ] §1.2 Apply fix: use `"agent call"` as default when `description` is empty, OR change `minLength: 1 → 0` in the agent_call JSON Schema
- [ ] §1.3 Verify: confirm `envelopes_publish_fail_total.agent_call` drops to 0 after fix; send a test SubagentStop frame and confirm it reaches ingestion

## §2. Bug #3 — Embedder 409 counted as error

- [ ] §2.1 Locate `create_collection` call path in `cortex-workers/src/embedder/` and confirm 409 is bubbled as `Err`
- [ ] §2.2 Handle 409 Conflict response as `Ok(())` — collection already exists is idempotent success
- [ ] §2.3 Add `{error_type}` label to the vectorizer_errors counter so conflict/auth/transport are distinguishable in health output
- [ ] §2.4 Verify: restart embedder, confirm `vectorizer_errors_total` reports only real failures

## §3. Bug #4 — Classifier mode mismatch (.env vs container)

- [ ] §3.1 Find which docker-compose file or env override is setting `CORTEX_CLASSIFIER_MODE=static` (or not passing `disabled`)
- [ ] §3.2 Either (a) fix the override to honor the `.env` value, or (b) update `.env` to declare the actual intended mode
- [ ] §3.3 Restart classifier container and confirm logs show the expected mode

## §4. Bug #5 — Bootstrap emits law rules as law_violation

- [ ] §4.1 Find the bootstrap promoter logic that maps `.claude/rules/*.md` to event envelopes
- [ ] §4.2 Change `kind` from `law_violation` to `law` for files that match the laws promotion pattern
- [ ] §4.3 Run a bootstrap against the Cortex repo; confirm `cortex_laws` index receives `kind: "law"` documents
- [ ] §4.4 Check if existing `law_violation` documents with `detector: null` and `ts: 0` should be removed from Meilisearch; if yes, add a cleanup step to the bootstrap incremental run

## §5. Tail (mandatory)

- [ ] §5.1 Update `docs/analysis/cortex/12-live-audit-2026-06-09.md` — mark bugs #1, #3, #4, #5 as fixed with commit reference
- [ ] §5.2 Write unit tests: agent_call envelope builder with empty description; embedder 409 response handler
- [ ] §5.3 Run `cargo check && cargo test -p cortex-core -p cortex-workers -p cortex-adapter-claude-code` and confirm all pass

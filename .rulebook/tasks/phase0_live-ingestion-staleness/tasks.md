## 1. Cold-archive path (tail watcher)
- [x] 1.1 Root-cause + fix the `files_watched=0` mount/root mismatch — DONE (commit 4706f08): mount host projects at `/data/claude-projects/projects`. Verified files_watched 0→203, 15052 envelopes emitted.
- [ ] 1.2 Confirm whether the restored cold archive re-feeds the live Meili/Nexus indexes (archive_loader / backfill), or only cold storage

## 2. Live indexing path (adapter → ingestion → Synap → classifier)
- [ ] 2.1 Determine why `cortex-ingestion` + `cortex-classifier-worker` are idle: is the host adapter daemon running + posting to `cortex-ingestion:17010 /v1/events`? (adapter-daemon.log stale 2026-06-20 13:13)
- [ ] 2.2 Repair the live path (start/fix the host adapter or its hook config); verify ingestion receives POSTs → Synap raw → classifier enriches → turns index advances past 2026-06-18 and graph gets new confidence-stamped edges (end-to-end phase27a proof on 2.3.4)

## 3. Watchdog (prevent silent recurrence)
- [ ] 3.1 Coverage/health alarm when `cortex-claude-archive` `files_watched==0` while the mounted projects dir is non-empty
- [ ] 3.2 Freshness alarm on `cortex-ingestion` (no POSTs / no Synap publishes in N minutes)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (the two-path ingestion architecture + the alarms; CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (walker path-layout test asserting root/projects discovery; watchdog unit test)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)

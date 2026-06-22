## 1. Cold-archive path (tail watcher)
- [x] 1.1 Root-cause + fix the `files_watched=0` mount/root mismatch — DONE (commit 4706f08): mount host projects at `/data/claude-projects/projects`. Verified files_watched 0→203, 15052 envelopes emitted.
- [ ] 1.2 Confirm whether the restored cold archive re-feeds the live Meili/Nexus indexes (archive_loader / backfill), or only cold storage

## 2. Live indexing path (adapter → ingestion → Synap → classifier)
- [x] 2.1 DONE (2026-06-21): root cause = the host adapter daemon (`cortex-adapter-claude daemon`) was simply NOT running. It listens on the named pipe `\\.\pipe\cortex-adapter-claude`, receives hook frames from `cortex-hook`, builds envelopes, and POSTs them to `cortex-ingestion:17010`. `adapter-daemon.log` showed it healthy (real session `14APAEZ9…` events + pre-thinking bundles flowing) until it stopped Jun 20 13:46 (likely reboot/terminal close). The repeated `ipc handler failed: os error 232` WARNs are benign fire-forget disconnects (cortex-hook writes + exits without awaiting the reply), NOT the cause — they coexisted with healthy ingestion on Jun 20. The earlier "system broken" impression was mostly the false-positive IPC canary spam (fixed + disabled separately, commits 74cea0b/b9314d4).
- [x] 2.2 DONE (2026-06-21): restarted `cortex-adapter-claude daemon` on the host. Verified live via the daemon `/healthz` (:17011) counters after restart: `frames_received_total={PostToolUse:5,PreToolUse:5}`, `frames_parse_error_total=0`, `envelopes_publish_ok_total={tool_call:18,turn:3}`, `envelopes_publish_fail_total={}` — the full path `cortex-hook → pipe → daemon → cortex-ingestion` is flowing again with ZERO publish failures. CAVEAT (durability): the daemon was launched as a background process this session; it is NOT set to autostart, so it will stop again on reboot/logoff. Operator action needed for permanent recurrence-prevention: register `cortex-adapter-claude daemon` as a login/startup task (or run `cortex-adapter-claude install` if that wires autostart). The §3 freshness watchdog also guards against silent recurrence.

## 3. Watchdog (prevent silent recurrence)
- [ ] 3.1 Coverage/health alarm when `cortex-claude-archive` `files_watched==0` while the mounted projects dir is non-empty
- [ ] 3.2 Freshness alarm on `cortex-ingestion` (no POSTs / no Synap publishes in N minutes)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (the two-path ingestion architecture + the alarms; CHANGELOG)
- [ ] 4.2 Write tests covering the new behavior (walker path-layout test asserting root/projects discovery; watchdog unit test)
- [ ] 4.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)

## 1. Walker — file size ceiling
- [ ] 1.1 Add `max_file_bytes: u64` to the per-repo config struct in `crates/cortex-bootstrap/src/config.rs` (default 8 MB)
- [ ] 1.2 Walker stat()'s every accepted file; if `size_bytes > max_file_bytes` emit `WalkEntry::Dropped { reason: format!("oversize:{size}>{limit}") }`
- [ ] 1.3 Unit test: a 12 MB synthetic file is dropped; an 8 MB file is accepted; the boundary is inclusive on the limit

## 2. Metrics + logging
- [ ] 2.1 Increment `cortex_bootstrap_files_dropped{reason="oversize"}` for every drop
- [ ] 2.2 INFO log: `dropped oversize file <rel_path> (<size> bytes > <limit>)` — once per file, never per chunk

## 3. Runner — per-event error tolerance
- [ ] 3.1 Replace the `?` propagation around `publish(...)` in runner.rs with a counter that increments on failure
- [ ] 3.2 If the failure ratio exceeds 5% of attempted publishes, abort the repo with the existing `?` error path (preserves systemic-failure detection)
- [ ] 3.3 Successful publish resets the per-repo failure ratio's denominator increment; per-event counters live in the existing metrics

## 4. End-to-end
- [ ] 4.1 Re-run `cortex-bootstrap` on the 17 Hive repos (same command line that failed on Tml)
- [ ] 4.2 Assert all 17 finish with `outcome="ok"` (Tml will report a non-zero `files_dropped` for `docs/docs.json`)
- [ ] 4.3 Assert post-run that no repo is half-indexed: every repo with `events_published > 0` has all of its accepted files in the lane

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend spec-09 with the `max_file_bytes` config + drop semantics; document the failure-ratio tolerance
- [ ] 5.2 Write tests covering the new behavior — unit tests on walker drops + runner failure counter + integration test simulating one publish failure mid-run
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-bootstrap`, `cargo clippy -p cortex-bootstrap --all-targets -- -D warnings`

## 1. Locate the wholesale-blank
- [ ] 1.1 Trace where tool_call payloads collapse from `{ command, file_path, … }` to `{}` (`cortex-core/src/redact.rs`, `cortex-classifier-worker/src/worker.rs`, or `cortex-api/src/archive_loader.rs::envelope_to_hit`)
- [ ] 1.2 Add a regression test that fails today: capture a `Bash` tool_call with `command = "git status"` → assert the lane-facing text contains the literal `"git status"`

## 2. Field-level redaction
- [ ] 2.1 Replace any `payload.input = json!({})` blanket-blank with a per-field redactor that masks values matching credential / token / secret patterns and leaves the rest verbatim
- [ ] 2.2 Per-tool field whitelist: `Bash.command`, `Edit.{file_path,old_string→hash,new_string→hash}`, `Read.{file_path,offset,limit}`, `Write.{file_path,content→hash}`, `TodoWrite.todos[*].content`, `Grep.{pattern,path,glob}`, `Glob.{pattern,path}`
- [ ] 2.3 Long bodies (`Edit.old_string`, `Write.content`) become a hash + first-line preview, not a full strip
- [ ] 2.4 Redaction trace appended to `envelope.redactions` per spec-04

## 3. Lane-facing text builder
- [ ] 3.1 New `cortex-classifier-worker` helper `tool_call_search_text(env: &Envelope) -> String` that emits e.g. `"[Bash] git commit -m fix(...)"` / `"[Edit] crates/.../sync_paths.rs — replaced PreThinkingRequest"`
- [ ] 3.2 `archive_loader::envelope_to_hit` consumes the new helper

## 4. Backfill
- [ ] 4.1 New subcommand `cortex-ingestion replay --redact-fix` that reads the existing archive parquet, re-runs the new redactor, re-emits the lane-facing text without rewriting raw envelopes
- [ ] 4.2 Re-seed the keyword lane on next boot from the replayed corpus
- [ ] 4.3 Verify on the 2026-04-26 / 2026-04-27 archive (≈1 653 tool_calls) — the same query now returns ≥ 80% non-`[Tool] {}` hits

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation (extend spec-04 redaction section)
- [ ] 5.2 Write tests covering the new behavior (per-tool whitelist, credential masking, hash-preview length)
- [ ] 5.3 Run tests and confirm they pass

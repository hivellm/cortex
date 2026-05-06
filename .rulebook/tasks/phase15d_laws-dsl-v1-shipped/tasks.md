## 1. Laws DSL v1
- [ ] 1.1 New `crates/cortex-laws/` with `Law` struct + serde-YAML deserialise.
- [ ] 1.2 Schema: `{ id, severity: critical|high|medium|low|info, trigger: { tool, action, args_match }, rule: { allow|deny|warn, when }, rationale }`.
- [ ] 1.3 `LawRegistry::load(dir)` reads every `.yml` under the laws dir; rejects duplicates by `id`.
- [ ] 1.4 `LawRegistry::evaluate(action, ctx) -> Vec<Verdict>` returns one verdict per matching law.
- [ ] 1.5 8 unit tests covering load + evaluate paths.

## 2. 6 starter laws
- [ ] 2.1 `.rulebook/laws/cortex-001-task-sequence.yml` — strict task-sequence execution from `AGENTS.override.md`.
- [ ] 2.2 `cortex-007-no-destructive-git.yml` — block destructive git ops without explicit user auth flag.
- [ ] 2.3 `cortex-008-no-verify-bypass.yml` — block `git commit --no-verify`.
- [ ] 2.4 `cortex-009-sequential-editing.yml` — warn on parallel multi-file edits.
- [ ] 2.5 `cortex-010-research-before-implement.yml` — warn on Edit/Write without prior Read on the file.
- [ ] 2.6 `cortex-011-fail-twice-escalate.yml` — warn on 3rd identical fix attempt.

## 3. Governance Engine endpoint
- [ ] 3.1 New `cortex-api /v1/laws/check` POST accepting `{ tool, action, args, ctx }` returning `[ { law_id, severity, verdict, rationale } ]`.
- [ ] 3.2 PreToolUse hook in `cortex-adapter-claude-code` calls the endpoint with 200ms timeout. Critical-severity `deny` verdicts block the tool; lower severities log a structured WARN.
- [ ] 3.3 Fail-open: timeout / network error → tool runs (per spec-12 fail-open contract); structured WARN logged.

## 4. CLI + lint
- [ ] 4.1 `cortex laws lint <dir>` validates every `.yml` against the schema and rejects duplicates.
- [ ] 4.2 `cortex laws list` prints active laws.
- [ ] 4.3 CI gate: `cortex laws lint .rulebook/laws/` in the workflow.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/13-laws-dsl.md` + `docs/specs/14-governance-engine.md` (status `v1`) + `CHANGELOG.md`.
- [ ] 5.2 Tests: §1.5 + per-law fixture test (each starter law denies at least one synthetic action) + §3 PreToolUse smoke.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.

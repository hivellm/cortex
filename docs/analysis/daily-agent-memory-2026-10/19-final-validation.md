# Final validation and review boundaries

Task `phase34_daily-agent-memory-recovery`, section 1.12. Dates: 2026-10-16/17.
The exact selected commit candidate passed the complete Rust quality gates.
Installed binary and authorized model checks are recorded separately in
[analysis22](22-authorized-live-validation.md).

## Independent review

- [Astra](16-astra-final-review.md): durable storage, API authorization, archive
  projection and runtime boundaries. Found archive classification loss and
  repository slug collisions; fixes include projection upgrade and real-key tests.
- [Sol](17-sol-final-review.md): Jev and archive production. Found malformed
  envelope output and acknowledgement/checkpoint loss on emission failure.
- [Integration review](18-integration-final-review.md): pre-thinking, MCP,
  adapter and launcher. Found repository basename ambiguity; daily lookup now
  requires an explicit valid repository identity.

These findings required corrections despite earlier passing suites. New
regressions must pass before updated binaries replace the installed copies.
Independent reviews are complete; individual reports retain their coverage limits.

## Quality gates

| Check | Observed result | Evidence |
| --- | --- | --- |
| Workspace, all targets/features `cargo check -j 4`, before final review fixes | Passed, 51.14 seconds | `%TEMP%/cortex-phase34-step112-check.log` |
| Workspace, all targets/features Clippy with `-D warnings`, before final review fixes | Passed, 55.71 seconds | `%TEMP%/cortex-phase34-step112-clippy.log` |
| Workspace, all targets/features `cargo check -j 4`, first review corrections | Passed, 38.96 seconds | `%TEMP%/cortex-phase34-step112-postreview-check.log` |
| Global `cargo fmt --all -- --check` after formatting reconciliation | Passed | Prior failure retained in `%TEMP%/cortex-phase34-step112-fmt.log` |
| Exact commit candidate `cargo fmt --all -- --check` | Passed, 2.361 seconds | `%TEMP%/cortex-phase34-candidate-v2-fmt.log` |
| Exact candidate workspace, all targets/features `cargo check -j 4` | Passed, 28.936 seconds | `%TEMP%/cortex-phase34-candidate-v2-check.log` |
| Exact candidate workspace, all targets/features Clippy `-D warnings` | Passed, 50.996 seconds | `%TEMP%/cortex-phase34-candidate-v2-clippy.log` |
| Exact candidate complete workspace/all-features suite, four build jobs and serial test threads | Passed: 3,955 tests, zero failures, nine ignored; 776.003 seconds | `%TEMP%/cortex-phase34-candidate-v2-test.log` |

The candidate was extracted from HEAD and overlaid with only the selected task
changes, including five partial files that exclude unrelated work. It uses the
same shared target directory but a separate source directory. The complete test
command used `--no-fail-fast` and `--test-threads=1`; all 231 reported test targets
completed. Ignored examples and the explicitly opt-in live provider test were
not silently counted as passes. Authorized live checks remain separate evidence.

The preceding candidate run retained three failures: a dashboard unit test
probed the installed adapter, an expanded Jev retry test still expected one
reservation instead of two, and the MCP registry specification omitted
`cortex_memory_read`. The dashboard test now supplies an explicit subsystem
snapshot, including a distinct p95-versus-probe-latency regression. The retry
counter and registry documentation match the exercised contracts. No production
deadline or acceptance assertion was relaxed. The failed run remains at
`%TEMP%/cortex-phase34-candidate-final-test.log`.

Existing `ts-rs` derive diagnostics report unsupported serde attribute parsing;
they appeared in successful check/Clippy logs. No warning suppression was added.

The global formatter initially failed in 26 files untouched by this task. Before
formatting them, a guard confirmed every path was clean against Git; no foreign
dirty file was included. Only rustfmt output was applied. Pre-format hashes and
paths are retained in ignored
`.cortex-state/daily-runtime/reviews/preexisting-format-drift.json`. This resolves
the pre-existing formatting gate failure recorded in section 1.9.

## Additional model review authorization history

Claude, Kimi and Grok participated in the earlier design decision (analysis05).
Claude also executed the behavioral acceptance recorded in analyses11/15.
Those are distinct from final source review.

The installed Codex CLI successfully completed a public synthetic availability
check requesting `kimi/kimi-k3` through its existing `9router` provider, returning
`ROUTER_REVIEW_READY` in 26.499 seconds. Native authentication was unchanged;
provider credential stores were not read. The initial attempt failed before
inference because quoted MCP override names produced an invalid transport table;
the corrected invocation used validated bare names. Both attempts are retained
under `%TEMP%/cortex-kimi-native-review-check-*`.

Source packets were then prepared locally for Kimi (storage), Grok (memory API)
and Claude (Jev). Each packet records exact source hashes and line numbers; none
contains `.env`, provider credentials or saved user memories. Automatic approval
review rejected the Kimi/Grok source-review command before process creation,
stating that private code payloads needed explicit authorization for those
destinations. No packet was transmitted by that command. A specific asynchronous
permission request covered the selected Rust files and all three requested
reviewers. The operator granted it on 2026-10-17. Kimi and Grok then completed
native source reviews; [analysis20](20-external-review.md) records source hashes,
findings, triage and the corrected feedback concurrency defect. Claude completed
a separate source review after two unsuccessful attempts;
[analysis21](21-claude-final-review.md) records the exact evidence. No alternate
transport was used to bypass the earlier rejection.

The official [gateway documentation](https://learn.chatgpt.com/docs/enterprise/connect-to-a-gateway)
and [CLI reference](https://learn.chatgpt.com/docs/cli/reference) informed the
native-provider invocation. A configured alias and one completed inference do
not establish support for another model's native CLI hooks or final code-review
participation. Kimi native-model configuration and Grok native CLI availability
retain the separate limits in analysis09/13.

The operator separately granted the real-memory payload permissions described
in analysis13/15. [Analysis22](22-authorized-live-validation.md) records their
precise scope, the delayed-consent retry defect found during live validation,
and subsequent checks. Local-only capture and the passed public synthetic
behavioral tests remain valid independently of those demonstrations.

The release follow-up in [analysis23](23-release-runtime-validation.md) rebuilt
the exact same Rust implementation with optimization. No Rust source or test
changed after these gates. The only subsequent executable-source change sets
the PowerShell launcher's default binary directory to `target/release`;
PowerShell 5.1 and 7 parse/self-tests passed. Candidate comparison confirms all
134 selected implementation files and five prepared partial files match the
final source, with that launcher adjustment explicitly recorded. Documentation
and task closeout are reviewed separately from the Rust gate snapshot.

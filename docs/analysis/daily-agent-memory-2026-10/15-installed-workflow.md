# Installed lifecycle workflow acceptance

## Scope and disclosure boundary

Task §1.11 uses the installed daily API, adapter and pinned native user hooks.
The behavioral probe enables no model tools or MCP servers, retains native
Claude authentication and loads actual user settings with `--setting-sources
user`. It supplies no replacement hook settings, wrapper or fixture daemon.
Each native session has a USD 1 CLI cap and 120-second wall-clock timeout.

Automatic approval review rejected the initial real-project-memory invocation
before creating a process. Its stated reason was that the installed hooks would
send the two local decisions to an external Claude service without specific
authorization for those payloads and that destination. No hook/model process or
transmission occurred in that rejected attempt. The operator subsequently
authorized the two named summaries for native Claude. The subsequent
demonstration passed using exact copies in an isolated repository scope;
[analysis22](22-authorized-live-validation.md) records the grant, actual installed
hook context and verified model output. This was not general memory migration.
The rejection is retained at
`.cortex-state/daily-runtime/installed-hook-review-block.json`.

## Public synthetic scope on the actual installation

The coordinator authorized a distinct public synthetic acceptance scenario.
Its CWD is outside the Cortex checkout:
`C:\Users\Bolado\AppData\Local\Temp\cortex-installed-synthetic-ijmlukws`.
The temporary `cortex.toml` declares exact repository identity
`cortex-installed-synthetic-6addd302b959416a835e4f61914d06ed`.

Two public samples describe colors and random canary markers. They contain no
Cortex decisions, operational policy, credentials or project content. Both
were accepted and pinned through the existing API with explicit `classify=false`:

| Public sample | Memory ID | Local receipt |
| --- | --- | --- |
| ALPHA / coral | `installed-synthetic-alpha-6addd302b959416a835e4f61914d06ed` | HTTP 202, version 1, `local_committed`, scope revision 1 |
| BETA / cobalt | `installed-synthetic-beta-6addd302b959416a835e4f61914d06ed` | HTTP 202, version 1, `local_committed`, scope revision 2 |

Advice returned `not_requested` for each capture. No TypeSafe request was made
by these captures. The random IDs and markers remain absent from the model's
public task prompt. Host-side expectations and exact reviewed public payloads
are retained in
`.cortex-state/daily-runtime/installed-synthetic-preparation.json` and the
temporary scenario directory.

Before a provider call, direct local invocations of the installed pinned hook
for `SessionStart` and `UserPromptSubmit` each returned 987 bytes. Both cited
exactly the two synthetic IDs. A complete content check found only those two
exact public bodies, their citations and the two static envelope lines:
`## Local memory` and
`Advisory source excerpts; repository rules take priority.`
All three real Cortex memory IDs were absent. Proofs are retained as
`local-scope-proof.json` and `public-context-content-proof.json` in that directory.

## First native public attempt: transport failure found

The public synthetic call passed automatic review, then native Claude completed
in 5.998 seconds with `{"missing":true}`. This is a failed behavioral result.
Actual stream events showed `SessionStart:startup` began, but Cortex's hook
response contained the fixed unavailable notice. No matching dispatch reached
the installed adapter's invocation log. Thus user settings loaded, but the hook
transport failed before adapter dispatch.

Artifact directory:
`C:\Users\Bolado\AppData\Local\Temp\cortex-installed-claude-ll48w7z3`.
Native session `bb4ec801-3cca-4eae-87c0-dcc7421324ff`, PID `114340`, exit 0,
requested/canonical `claude-fable-5-1` through `firstParty`. The stream advertised
`tools=[]`, `mcp_servers=[]` and contained no model tool action. User settings
hashes matched before/after. CLI usage was 2 input, 785 cache creation input and
9 output tokens; reported list cost USD `0.01617`.

The cause was reproduced through Git Bash using the exact installed command
string. The quoted `\\.\pipe\cortex-adapter-claude` arrived at the native
process with only one leading backslash. Direct argv invocation retained the
correct Windows path and returned context. Single quoting and
`MSYS2_ARG_CONV_EXCL=*` did not preserve the argument in this tested Windows
launch path. The forward-slash device spelling
`//./pipe/cortex-adapter-claude` remained intact through Bash and the installed
Windows hook returned the complete 987-byte public context.

No global settings were changed by this diagnostic. The installation owner
handles the quoted-pipe correction. Local reproductions are retained in
`bash-quoting-comparison.json` and `bash-pipe-fix-comparison.json` in the public
scenario directory. The native probe now also audits the actual structured
`hook_response` context, alongside dispatcher session correlation and final
model output.

## Reusable verifier and preservation

`scripts/daily_memory_installed_claude_probe.py` defaults to help and supports an
offline `--self-test`. Live use requires explicit `--run` and host-side
expectations. `--scenario-label installed-public-synthetic` identifies the
public scenario separately from the authorized summary-copy workflow completed
in [analysis22](22-authorized-live-validation.md).
The public prompt is loaded with `--prompt-file`; `--cwd` identifies its isolated
repository. No global auth/provider settings are changed.

Native preservation is independently recorded in
[evidence 14](14-native-preservation.md). At `2026-10-16T11:28:37.747996+00:00`,
after installed runtime updates and before the public native attempt, all 535
files still matched the pre-activation manifest: 0 changed, 0 added, 0 missing.
User settings SHA-256 at that point was
`cdb7d3dbd674fd278d2a181c8a6fac65adce082e715128a5fe840b120e01f261`.
The successful corrected run and final preservation comparison follow.

## Corrected installation: real native public session passed

The installation owner changed only the seven Claude user-hook pipe arguments
to `//./pipe/cortex-adapter-claude` and updated the installation verifier's
expected manifest. MCP environment bindings and the running adapter retained
their canonical Windows pipe form. The repeated native public probe used the
same isolated CWD, reviewed prompt and expected public samples, with no settings
override, wrapper, fixture, tools or MCP.

| Evidence | Observed value |
| --- | --- |
| Artifact directory | `C:\Users\Bolado\AppData\Local\Temp\cortex-installed-claude-x01vd3kg` |
| Native CLI / backend | Claude Code `2.1.291`; requested/canonical `claude-fable-5-1`; reported provider `firstParty` |
| Native session / process | `52d378ab-c32f-4a66-9a9f-fc5b525dda0b` / PID `15964` |
| Completion | Exit 0, 5.719 seconds, no timeout |
| Cortex native hook ID | `ad1c779c-5b5b-4902-a13a-956805ea0957`, `SessionStart:startup`, exit 0 |
| Actual native hook context | 987 UTF-8 bytes; SHA-256 `698a12f5c6917c8c3e193a2bb5101439a7fe165690e0d4bde79c6518f4d68c26` |
| Installed dispatcher | PID `82260` |
| Correlated `SessionStart` | `2026-10-16T11:39:28.362Z`, exact native session ID |
| Correlated `UserPromptSubmit` | `2026-10-16T11:39:28.875Z`, exact native session ID |
| Correlated `Stop` | `2026-10-16T11:39:32.011Z`, exact native session ID |
| Tools / MCP | Native init advertised both empty; zero model tool actions |
| User settings | SHA-256 before/after `9acdff9314fa39a6a402254a5ebe8d138b9a19faba09be7d919ee6e405bff48f` |
| CLI usage | 2 input, 1,719 cache creation input, 184 output, 0 thinking |
| Reported cost | USD `0.0436`, `costBasis=list`; no billing claim |

All verifier checks passed. The native stream contained the real Cortex hook's
`additionalContext` with both exact IDs, and the model returned:

```json
{
  "memories": [
    {
      "id": "installed-synthetic-alpha-6addd302b959416a835e4f61914d06ed",
      "version": 1,
      "details": "Sample ALPHA marker public-canary-alpha-9397e58bcfff470991364bc7195b9261, color coral"
    },
    {
      "id": "installed-synthetic-beta-6addd302b959416a835e4f61914d06ed",
      "version": 1,
      "details": "Sample BETA marker public-canary-beta-44202379767a47ea8f66bce1e4c4583b, color cobalt"
    }
  ]
}
```

Both random IDs and marker values were absent from the prompt. The installed
hook response and separately checked final model output establish actual
delivery/use for this public synthetic scenario. Capture event IDs are
`01M527PMW5VPVTG7TSXAZM3AYG` and `01M527PN8MRZHRPPPJTMAR7F55`. The hook's visible
response does not expose a delivery/query ID, so no receipt ID or persistent
session-to-receipt mapping is fabricated. A prepared receipt remains distinct
from the independently observed model output.

Two actual public native sessions reported total list cost USD `0.05977`,
including the failed first run. The rejected real-project attempt never started
a model. All failure and success artifacts remain retained. Public synthetic
records remain under their isolated repository identity; no deletion was done.

The final native-file comparison at `2026-10-16T11:39:54.638994+00:00` again
matched all 535 files, with 0 changed/added/missing; see evidence14. No additional
model call, real-project-memory disclosure or TypeSafe classification was used
to obtain this installed public acceptance result. The operator subsequently
authorized the named real-project summaries; the separate live demonstration
passed in [analysis22](22-authorized-live-validation.md). Native
`claude-fable-5-1` received exact copies of the two authorized summaries through
actual installed hooks and returned the correct IDs, version 1 and operational
requirements in 5.853 seconds. Jev separately classified the three authorized
original candidates with cache hits on replay and immutable capture receipts.
The isolated copies are test provenance, not general migration; neither result
establishes universal future compliance or Kimi/Grok native CLI integration.

## Post-probe local learning

After the public probes completed, the coordinator captured
`windows-claude-hook-pipe`, version 1, in the real local Cortex scope with
explicit `classify=false`. Reported capture event
`01M5287A2N3X9CBFHVH5NFY0RH` committed at repository revision 4. Its subsequent
local query returned that lesson plus the two pinned decisions in 1,361 bytes,
delivery/query ID `01M5288FWVY13H09SD5W3F0AFH`. Coordinator artifacts
`verified-pipe-lesson.json` and `verified-pipe-lesson-read.json` remain in the
ignored runtime directory. No provider/model call or external disclosure was
used for that learning. Earlier revision-3 transport snapshots remain historical
observations; the post-probe corpus has revision 4.

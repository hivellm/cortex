# Cross-session and cross-model behavioral acceptance

## Scope and status

Task §1.10. Live behavioral acceptance passed on 2026-10-16 after explicit GO,
using the rebuilt synthetic fixture and its guarded `--resume` mode. Two native
CLI sessions completed with exit 0, no timeout and no retry. These sessions can
make several provider requests while following the MCP tool loop; they are not
claimed as only two individual provider requests.

`scripts/daily_memory_cross_model_probe.py` uses real installed native clients,
the real MCP binary and the disposable loopback API backed by SQLite. It never
reads credential stores, changes installed configuration, installs software or
executes its generated HTTP request. Native authentication remains with each CLI.

## Acceptance scenario

One Claude session requests `claude-fable-5-1`. It makes exactly four MCP calls:
capture an accepted billing-export policy; read it; explicitly capture an
accepted successor using the observed revision and predecessor ID; read again.
The first read warms the revision-aware path before correction. The probe checks
actual RPC results and local-commit receipts, rather than the final narrative.

The harness seeds three distractors through the real synthetic API: a matching
policy in another repository, a proposed matching policy, and an unrelated
accepted policy. Original policy becomes the fourth distractor after explicit
supersession. Both reads must return the single expected accepted ID/version,
with citation, bounded context and a matching query/delivery ID.

The API process then stops. A separate process reopens the same fixture database
using its fixture ownership marker. One new Codex session requests
`cx/gpt-6.1-sol`, reads current policy and constructs a JSON request containing
method, path, headers and retry settings. It also reports the actual citation
and delivery ID. The full object must match the corrected policy exactly.
Random path, header and memory ID are absent from Codex's prompt and are checked
for presence in returned memory. They cannot be reconstructed from the task.

This distinguishes preparation from observed use: a delivery receipt remains
`prepared`; the separately verified model-produced request proves compliance in
this bounded scenario. It does not establish universal recall or compliance.

## Isolation and limits

The proxy exposes capture/read only; Codex's proxy rejects all captures. All
tool calls must exactly match host-side synthetic scope and byte/record bounds.
No shell, filesystem or outbound HTTP action is requested from either model.
Artifacts include prompts, CLI invocations/versions, native usage where exposed,
MCP request/response IDs and bodies, read receipts, process restart IDs and the
model's final action. Known environment credentials are redacted from artifacts.

Claude uses settings isolation with native authentication; `--bare` is avoided
because installed native login was unavailable with that flag in earlier tests.
Codex retains native configuration for authentication, which can also advertise
other installed MCP servers. Its prompt prohibits other tools; this limitation
is audited against the recorded structured stream. Any observed command, file,
web or other tool action invalidates acceptance; absence of observed native tool
actions also fails. Session/thread IDs and model fields are retained when the
client emits them, alongside separate native client process IDs. The proxy
itself grants no access to any other Cortex operation.

Exactly two live model sessions are attempted, with no automatic retry. Claude
has a $1 CLI budget cap; Codex exposes no equivalent cap here, so wall-clock
timeout and one session bound its run. Usage/cost is reported only when supplied
by the native CLI. Missing cost stays unavailable, never estimated.

Kimi's installed native model/configuration remains blocked from earlier
acceptance; Grok CLI is absent. This probe does not turn either limitation into a
passing cross-model claim. Outage, resource and latency evidence belongs to the
broader §1.10 acceptance and is not implied by this model-action scenario.

## Reproduction

Offline validation makes no provider call:

```powershell
python scripts/daily_memory_cross_model_probe.py --self-test
```

After explicit model-call authorization and the fixture rebuild:

```powershell
python scripts/daily_memory_cross_model_probe.py --run `
  --fixture-binary target/debug/examples/daily_memory_fixture.exe `
  --mcp-binary target/debug/cortex-mcp-server.exe
```

Default invocation only displays help. A live run prints its retained temporary
artifact directory and writes `report.json`, even on acceptance failure. Only
owned fixture/client process trees are stopped; fixture data stays retained.

## Verification performed

Offline checks passed: exact scope allowlist, missing/invalid successor revision
rejection, no capture permission in reader session, global/wrong-repository read
rejection, exact corrected action acceptance, stale action rejection, fabricated
delivery rejection, prompt leakage rejection and extra stale memory rejection.
No Cargo invocation or live model call was made during preparation. Parent
owned the fixture build and Rust checks. Live results follow.

## Observed live run

Retained artifact directory:
`C:\Users\Bolado\AppData\Local\Temp\cortex-cross-model-cu6vjq_0`.
`report.json` passed every writer/reader check. Both native structured streams,
prompts, invocations, client versions and complete RPC journals are retained.

| Evidence | Observed value |
| --- | --- |
| Claude Code | `2.1.291`; requested and canonical reported `claude-fable-5-1`, `firstParty` |
| Claude session / process | `ac2ecb64-d356-48de-9eac-cb564b0598d4` / PID `110556` |
| Codex | `codex-cli 0.162.0`; requested `cx/gpt-6.1-sol`, also present in actual MCP client turn metadata |
| Codex session / process | `01a1445c-1acf-7f92-b938-b5ca3c03626f` / PID `115812` |
| API restart | PID `115212` stopped; PID `47992` reopened the same fixture database |
| Original accepted memory | `original-e429f6a0eadc426e915bdc6cc5d713a9`, version `1` |
| Accepted successor | `corrected-e429f6a0eadc426e915bdc6cc5d713a9`, version `1`, explicit predecessor, observed expected revision `3` |
| Scope after correction / restart | Revision `4` in both reads |

| Read | Actual RPC ID | Delivery/query ID | Context bytes | RPC round trip |
| --- | --- | --- | ---: | ---: |
| Claude original | `3` | `01M525QM4VHTG03VD5DKESA4AA` | 457 | 18.142 ms |
| Claude successor | `5` | `01M525QXMTNZT91ZFW3ZCH3G2Z` | 465 | 13.117 ms |
| Codex after restart | `2` | `01M525RCQPNS22V5WZXTM6Q1BY` | 465 | 24.876 ms |

Capture RPC IDs `2` and `4` returned exact local-commit receipts in 19.047 ms
and 15.821 ms respectively. Round trips use proxy journal timestamps and cover
actual MCP/API tool execution, excluding provider reasoning and CLI startup.
All three reads reported `stage=prepared`, `cache_hit=false`; the old revision
was warmed before correction, but the corrected query correctly missed that
cache entry. Only the active expected memory appeared in each read.

Codex's exact final action was:

```json
{
  "request": {
    "method": "PUT",
    "path": "/synthetic/export/current-dc426e915b",
    "headers": {
      "X-Export-Policy": "current-9a317d5cc6cdb519e624cdae0a6f924e"
    },
    "retry": {"max_attempts": 2, "backoff_ms": 175}
  },
  "memory_id": "corrected-e429f6a0eadc426e915bdc6cc5d713a9",
  "version": 1,
  "delivery_id": "01M525RCQPNS22V5WZXTM6Q1BY"
}
```

The random path, header and memory ID were absent from Codex's prompt, present
in the actual returned memory and matched programmatically. This was a concrete
request plan, not an executed network request. It replaced the original `POST`
policy with retry settings `7`/`900` by the accepted `PUT` policy with `2`/`175`.
Wrong-repository, proposed, unrelated and superseded IDs were all absent.

Claude's stream contained exactly four tool-use actions, all allowed Cortex
calls. Codex's stream contained only the start/completion pair for its single
`cortex_memory_read` and the final model response. No command, file, web or other
tool action occurred in either model stream. Session IDs and client processes
were distinct. Codex's stream omitted a model field; the MCP turn metadata is
client-reported provenance, not independent verification of provider internals.

Native CLI-reported usage:

| Client | Reported usage | Reported cost |
| --- | --- | --- |
| Claude | 130 input, 5,825 cache creation input, 16,388 cache read input, 1,231 output; 0 thinking | USD `0.183447`, CLI `costBasis=list`; not a billing claim |
| Codex | 33,980 input, including 16,512 cached input; 235 output, with `reasoning_output_tokens=18` | Unavailable; CLI reported no dollar cost |

Native Codex also emitted startup warnings: ignored installed `general`/`limits`
settings; a plugin SessionEnd timeout clamped to 3 seconds; model-catalog refresh
decode errors; and failed discovery/authentication for unrelated installed MCP
servers (`context7` and loopback port 8000). Its Windows command wrapper warned
about the extended canonical CWD; Codex's explicit `--cd` and emitted workspace
metadata still identified the fixture repository. These diagnostics are retained
and were not mistaken for Cortex tool execution or successful unrelated tools.
Installed configuration was not changed. They did not prevent the audited
single Cortex read or exact corrected action, and add no integration claim for
those unrelated servers.

Post-run checks found both fixture listeners closed and all four recorded owned
API/client PIDs absent. Temporary fixture databases and evidence remain retained.

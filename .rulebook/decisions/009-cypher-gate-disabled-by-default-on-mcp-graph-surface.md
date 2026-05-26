# 9. Cypher gate disabled by default on the MCP graph surface

**Status**: proposed
**Date**: 2026-05-07
**Related Tasks**: phase11v_mcp-fine-grained-backend-search

## Context

Phase11v shipped three fine-grained backend-search tools on the MCP
surface (`cortex_keyword_search`, `cortex_vector_search`,
`cortex_graph_query`). The graph tool exposes Nexus through two
modes:

- **`neighbors`** — bounded neighborhood walk, depth 1..=5, optional
  `edge_kinds` filter. Read-only by construction; the request shape
  cannot express anything Nexus would not already serve via its
  `/graph/neighbors` endpoint.
- **`cypher`** — raw Cypher passthrough. Arbitrary read OR write
  statements; parameter substitution but no statement-shape guard.

The MCP transport is consumed by descriptor-driven AI clients. Tool
descriptors are not signed and the orchestrator does not attest the
caller's identity. Anything an MCP descriptor can invoke is, in
practice, reachable from any prompt that resolves the tool name —
including prompts the operator did not author and cannot audit
ahead of time.

A raw `cypher` channel through Nexus on that surface lets a prompt
issue `MATCH (n) DETACH DELETE n`, schema mutations, or any other
side-effecting Cypher Nexus accepts. The same channel through a
human-driven path (`cortex-ops`, the Nexus dashboard, a TS/Python
SDK) is fine because the caller is a known operator with a clear
authorization context.

The phase11v proposal flagged this asymmetry and asked whether the
fine-grained graph tool should ship `cypher` at all, ship it
gated, or ship it ungated. We needed a decision before §1.4
landed.

## Decision

Ship `cypher` mode behind a strict environment gate:
`CORTEX_GRAPH_CYPHER_ENABLED=1` (also accepts `true`,
case-insensitive; everything else, including unset, is "off").

When the gate is off, `handle_graph_query` short-circuits before
any Nexus dispatch and returns
[HTTP 403 + reason `cypher_disabled`](../../crates/cortex-api/src/search_proxy.rs#L539)
with a one-line remediation message naming the env var. The gate
covers BOTH the cortex-api proxy handler and the MCP wrapper —
the wrapper does not pre-screen, so any client probing the tool
gets the same `cypher_disabled` envelope regardless of transport.

The `neighbors` mode is **never** gated. It is the default read
path and the only one the MCP surface is expected to use in
ordinary operation.

The gate is a runtime env var, not a build-time feature flag, so
operators can flip it without rebuilding the cortex-api container.
The flag's value is read on every request — there is no
per-process cache — so a SIGHUP-equivalent (just restart the
container, the read is idempotent) is not necessary. Tests pin
the off-by-default contract:
[`cypher_gate_off_by_default`](../../crates/cortex-api/src/search_proxy.rs#L728-L740)
exercises unset / `1` / `true` / `no` / re-unset transitions.

The cortex-mcp-server tool descriptor advertises the two-mode
discriminator without lying — `mode` accepts `"neighbors" |
"cypher"` — and surfaces the `cypher_disabled` reason verbatim
when the gate is off. Documenting the closed door is part of the
contract; pretending the door isn't there would just shift
discovery to runtime probing.

## Alternatives Considered

### Ship cypher mode ungated

Rejected. The MCP surface is the broadest reach Cortex exposes
into Nexus — every Claude Code session, every cortex-mcp-server
client, every descriptor-driven agent can hit it. Defaulting to
"any prompt can run any Cypher" makes the orchestrator the weakest
link in Nexus's authorization story. Even in dev stacks the
default should fail safe; the dev who needs raw Cypher can flip
the env var explicitly and accept the read-write posture for that
container.

### Drop cypher mode entirely

Rejected, but barely. The `neighbors` mode handles the bulk of
the read graph workload and is sufficient for the dashboard,
pre-thinking, and most query intents. Cutting `cypher` would
simplify the surface and remove the gate question entirely.

We kept it for two reasons:

1. The orchestrator's free-search intent has a tail of queries
   that genuinely need a `(predicate)` shape `neighbors` can't
   express — graph-shape lookups across multi-hop conditional
   joins. Without `cypher`, those queries fall back to keyword,
   which loses the structural signal.
2. Operators running `cortex-ops` ad-hoc explorations want one
   path that works through the same proxy the MCP tool uses, so
   the wire response shape and error taxonomy match. Maintaining
   two paths (an internal raw-Cypher path for cortex-ops + a
   gated MCP path for everything else) doubles the surface.

The gate keeps the capability without making it the default
posture. The right tradeoff is "off by default, on when an
operator opts in", not "off forever".

### Allowlist Cypher statement shapes

Considered. A static parser could refuse anything that contains
`CREATE`, `DELETE`, `SET`, `MERGE`, `REMOVE`, etc., and only
permit `MATCH ... RETURN`. This narrows the blast radius of an
ungated default and removes the env-var step.

Rejected for v1. Cypher's grammar is large; an allowlist that
matches every read-shape and rejects every write-shape is itself
a security-critical parser, and getting it wrong both ways
(false-positive — operator can't run a legitimate read; false-
negative — write slips through) is plausible. The env gate is
crude but unambiguous: the bytes `1` enable; anything else
disables. We can revisit a structural allowlist if `cypher` mode
sees enough operator usage to warrant the parser investment.

### Auth-token-bound gate

Considered. Bind the gate to the operator-elevated bearer that
`/v1/dashboard/*` already uses (`CORTEX_DASHBOARD_AUTH=1` flow).
Cypher requests would have to present the dashboard bearer.

Rejected for v1, but earmarked as the migration target. The
dashboard auth posture is itself proposed (the v1 default is no
key) and tying the cypher gate to it would compound two
unsettled decisions. When the dashboard auth posture lands as
ADR-N (post-phase11v), this ADR's "How we keep the door closed"
section gets re-opened: bearer-bound gating is strictly stronger
than env-var gating, because env vars are observable from
anything sharing the container's environment.

## Consequences

**Positive:**

- The MCP graph surface ships read-only by default. A new client
  consuming `cortex_graph_query` cannot reach `MATCH ... DELETE`
  Cypher without an explicit operator action.
- The gate's failure mode is observable: a client probing
  `mode=cypher` without the env var gets a structured
  `cypher_disabled` envelope with the env-var name in the
  remediation message. Operators see "this tool wanted cypher,
  flip the gate if you authorize that" instead of a silent
  fallback to `neighbors` (which would mask the request shape).
- The decision is reversible. Flipping the gate to "on by
  default" is a one-line config change in production; flipping
  back is the same. No code, no migration.
- The gate is enforced at the proxy layer, so any future
  consumer of `/v1/search/graph` (Cursor adapter, OpenCode
  plugin, custom CLI) inherits the posture without re-stating
  the rule.

**Negative / tradeoffs:**

- Operators who legitimately need cypher have to know the env
  var exists. The CHANGELOG entry and spec 22 §`cypher` mode
  document it, but discovery still depends on reading those.
  An operator might miss the gate and assume the tool is
  outright broken. Mitigated by the remediation message in the
  403 envelope.
- Env-var gates are observable from anything sharing the
  process environment. A misbehaving sibling daemon in the same
  container could read the flag and infer the posture. Not a
  confidentiality concern (the flag is binary, not a secret),
  but it does mean "this container has Cypher enabled" leaks to
  process inspection. The bearer-bound migration noted above
  closes this gap.
- The static allowlist alternative is strictly more flexible
  than the env gate — it would let an operator run safe reads
  without authorizing writes. We accept the coarser posture for
  v1 and re-open the question if cypher usage grows.
- The gate adds a third dimension to the response taxonomy
  (alongside `index_not_found`, `bad_input`, etc.). Tests
  enumerate the full set ([search_proxy::tests](../../crates/cortex-api/src/search_proxy.rs#L728-L740)) so a future refactor cannot
  silently merge `cypher_disabled` into a generic `forbidden`.

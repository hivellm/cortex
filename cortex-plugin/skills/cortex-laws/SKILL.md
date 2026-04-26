---
name: cortex-laws
description: Surface active laws in the current scope (repo + topics). Returns law id, severity, title, and detector for each rule that applies. Use this when the user asks "what laws apply here?", "is X allowed?", "any rules I should know?", or before suggesting an action that might trip a critical law.
---

# cortex-laws

Use this skill to enumerate the laws that govern the current scope.
A law is a codified rule with an enforcement tier (`info` /
`notable` / `critical`); critical laws are blocking when invoked
through the spec-10 pre-tool hook.

## When to use

- The user asks "what laws apply?" / "any rules in this repo?".
- The user asks "can I do X?" / "is Y allowed?" — pair with a
  `cortex-lawkeeper` sub-agent invocation if you want the laws
  *plus* a verdict.
- You're about to suggest a `Bash` invocation, a destructive op,
  or a settings change and want to confirm no critical law fires.

Do NOT use this skill to:
- Look up a specific law id — query the dashboard at
  `/laws/<id>` instead.
- Author a new law — that's a `/cortex-laws-author` flow (not in
  this plugin yet; lands with the dashboard authoring surface).

## How to invoke

Call `cortex.query` with `intent=law_check`:

```jsonc
{
  "intent": "law_check",
  "query": "<short description of the action you're considering>",
  "scope": {
    "repo": "<repo from cwd>",
    "topics": ["<optional topic chips>"]
  },
  "include": ["violations"],
  "budget_ms": 300
}
```

`intent=law_check` returns only the `violations` field by default
(spec 11 §Acceptance criteria). The `laws_active` overlay carries
the laws that match scope; surface those when the user asks for
a positive listing.

## How to use the response

For each law in `laws_active`:

- Cite the `id` (`LAW-007`).
- Cite the `severity` and a one-line summary of the `title`.
- If `severity = critical`, surface the law BEFORE proposing the
  action — the spec-10 hook will block the model otherwise, and
  it's better to discuss in plain language than to surprise the
  user with a `permissionDecision: deny`.

If the response also carries `violations` (recent violations of
the same law), mention them as evidence.

## Empty result

No active laws → `laws_active: []`. Tell the user:
"No laws codified for this scope yet." Don't invent rules.

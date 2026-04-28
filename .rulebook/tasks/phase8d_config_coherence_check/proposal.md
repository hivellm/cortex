# Proposal: phase8d_config_coherence_check

## Why

The 2026-04-28 incident's first wrong turn: the adapter was talking to
`http://127.0.0.1:15010` while ingestion was bound to `:17010`. The
config file (`~/.cortex/adapter.toml`) said `endpoint = ":17010"` —
correct — but a stale daemon was still up, holding the old endpoint
in memory. There was no tool that compared "what the config files say"
to "what the running processes are actually using" to "what's actually
listening on the loopback".

When ports moved (commit `d6cd7dc chore: migrate workspace ports
1500x → 1700x`), three things needed to update in lockstep: `.env`,
`adapter.toml`, and any running process. They didn't, because there
was no enforcement — and even after the user updated the config file,
the running daemon kept failing for hours without anyone noticing.

`cortex doctor config` should declaratively check coherence across
every config surface and flag drift before it bites.

## What Changes

1. NEW `crates/cortex-doctor/` — CLI binary (or library + thin bin
   wrapper if cortex-ops already exists) implementing config checks.

2. Surfaces audited:
   - **`.env`** — `CORTEX_ARCHIVE_ROOT`, `CORTEX_FULLTEXT_MEILI_URL`,
     `NEXUS_URL`, `VECTORIZER_URL`, `CORTEX_*_URL` family
   - **`~/.cortex/adapter.toml`** — `[adapter]` section: `endpoint`,
     `api_endpoint`, `timeout_ms`, `queue_bounded`
   - **`cortex-plugin/.mcp.json`** — `mcpServers.cortex.env.CORTEX_API_URL`
   - **`cortex-plugin/hooks/hooks.json`** — present + valid JSON
   - **Workspace** — `crates/cortex-*/Cargo.toml` workspace deps coherent
   - **Live state** — `Get-NetTCPConnection -State Listen` (Windows) /
     `ss -tlnp` (Linux) actual listening ports

3. Cross-checks:
   - For each `CORTEX_*_URL` in env, the host:port MUST be reachable AND
     match a configured-listen port of the corresponding service.
   - `adapter.toml.endpoint` MUST equal `CORTEX_INGESTION_URL` (or
     `http://127.0.0.1:<env CORTEX_INGESTION_PORT>`).
   - `adapter.toml.api_endpoint` MUST equal `CORTEX_API_URL`.
   - `cortex-plugin/.mcp.json CORTEX_API_URL` MUST equal `.env CORTEX_API_URL`.
   - Every URL referenced anywhere MUST resolve to a listener that
     responds to `/healthz` within 1500 ms.

4. Output:
   ```
   $ scripts/doctor-config.bat
   ✓ .env reachable           (12 vars)
   ✓ adapter.toml reachable
   ✗ adapter.toml.endpoint = http://127.0.0.1:15010
     but no process listens on :15010 (expected :17010 per .env)
   ✓ .mcp.json CORTEX_API_URL coherent with .env
   ...
   ```
   Exit 0 if all pass, 1 on warnings, 2 on critical drift.

5. NEW `cortex-api /v1/health/config` returns the same audit as JSON for
   the GUI Health view to consume.

## Impact

- Affected specs: NEW `specs/config_coherence/spec.md`.
- Affected code:
  - NEW `crates/cortex-doctor/` (or extend existing `cortex-ops`)
  - NEW `scripts/doctor-config.bat` + `.sh`
  - `crates/cortex-api/src/health/config.rs` — new module
  - `crates/cortex-api/src/dashboard.rs` — wire route
- Breaking change: NO (read-only audit).
- User benefit: when ports / endpoints drift, the user sees
  "adapter.toml.endpoint = :15010 but nothing listens there" in
  one command instead of after 2 hours of wal/log inspection.

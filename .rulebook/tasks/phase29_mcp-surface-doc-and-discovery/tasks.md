## 1. Runtime discovery — `cortex_capabilities` tool
- [ ] 1.1 Extend the `Tool` trait (or add an adjacent static lookup table) in
      `crates/cortex-mcp-server/src/tools.rs` so `read_or_write` is
      queryable at runtime for every implementor — today this classification
      only exists as prose in spec 20's registry table.
- [ ] 1.2 Implement `CapabilitiesTool` returning `{name, one_line_purpose,
      read_or_write}` for every tool in `ToolRegistry::default_set()`,
      deriving `one_line_purpose` from each tool's existing
      `descriptor()["description"]` (no new prose to author).
- [ ] 1.3 Register `cortex_capabilities` in `ToolRegistry::default_set()`
      (becomes tool #38) and add its own row to spec 20's registry table.

## 2. `cortex-ops doctor-registry-sync` check
- [ ] 2.1 Add `crates/cortex-cli/src/bin/cortex-ops/doctor_registry_sync.rs`
      implementing the check spec 20's "registry drift" requirement already
      specifies: parse the Registry table (row count + tool names) and
      compare against `ToolRegistry::default_set()` (count + names),
      reporting missing/extra tool names on either side.
- [ ] 2.2 Wire exit-code severity matching the existing doctor convention
      (0 clean; nonzero on any drift, escalating to critical at the ≥2-tools
      threshold spec 20 already commits to for blocking PRs).
- [ ] 2.3 Add the `doctor-registry-sync` subcommand to
      `crates/cortex-cli/src/bin/cortex-ops.rs`'s CLI dispatch alongside the
      existing `doctor-config` / `doctor-versions` entries.

## 3. CI wiring — fast path only (do not duplicate phase30's scheduling)
- [ ] 3.1 Add a lightweight, path-scoped GitHub Actions job mirroring
      `.github/workflows/dashboard-grep-gate.yml` (no docker-compose stack,
      <2 minute budget) that runs `cortex-ops doctor-registry-sync` on every
      push/PR touching `crates/cortex-mcp-server/src/tools.rs` or
      `docs/specs/20-mcp-tool-surface.md`.
- [ ] 3.2 Note in the workflow's header comment that the nightly
      long-lived-stack doctor schedule is `phase30_live-e2e-smoke-and-doctor-wiring`'s
      responsibility — this gate is PR-time only, not a second schedule.

## 4. Close the loop on spec 20's existing placeholders
- [ ] 4.1 Update the "tool surface registry stays in sync" scenario
      (currently citing "future work — phase10k doctor entry") to reference
      the implemented `doctor-registry-sync` check by name, and remove the
      stale phase10k cross-reference (phase10k was the retention daemon
      task, unrelated to doctor checks).
- [ ] 4.2 Update the "registry drift is caught before it reaches 30
      undocumented tools" scenario to name `cortex_capabilities` +
      `doctor-registry-sync` as the concrete enforcement mechanism.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation —
      `docs/specs/20-mcp-tool-surface.md`'s registry table with the new
      `cortex_capabilities` row, and CHANGELOG.md.
- [ ] 5.2 Write tests covering the new behavior — unit test for
      `CapabilitiesTool` (38 entries, every entry has a non-empty purpose
      and a valid `read`/`write` value); unit or integration test for
      `doctor-registry-sync` (in-sync case passes; an injected drift fails
      with the correct missing/extra tool names).
- [ ] 5.3 Run tests and confirm they pass — including a local run of the
      new CI gate against a deliberately-drifted doc to confirm it fails
      loudly.

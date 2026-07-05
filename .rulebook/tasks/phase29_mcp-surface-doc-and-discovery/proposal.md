# Proposal: phase29_mcp-surface-doc-and-discovery

## Why

Source: `docs/analysis/cortex-platform-2026-07/` (README.md +
execution-plan.md; `findings.md` is referenced by the README as the
detailed audit but had not yet been written to disk as of this task's
authoring — a concurrent authoring pass in the same working tree). That
analysis supersedes the earlier `docs/analysis/cortex/11-platform-vision-analysis.md`
and is the narrative source behind the phase28–33 Rulebook backlog it
generated; per its execution plan, phase28's three tasks
(`phase28_live-testing-bugfixes`, `phase28_docs-truth-reconciliation`,
`phase28_retrieval-eval-gate-live`) are a prerequisite gate under
LAW-CORTEX-001 — this task does not begin until all three are complete.
Rulebook knowledge base entries `mcp-api-tool-registry-spec-doc-diverges-from-runtime-registry-with-no-automated-check`
and `ship-then-dead-wire-features-land-unit-tested-but-disconnected-from-the-live-path`
(both tagged `analysis:cortex-platform-2026-07`, captured 2026-07-05) record that `docs/specs/20-mcp-tool-surface.md` documented
exactly 7 Cortex MCP tools while the runtime `ToolRegistry::default_set()`
(`crates/cortex-mcp-server/src/tools.rs:260-302`) actually registers 37 —
confirmed by direct read of the source: exactly 37 `Arc::new(...Tool::new())`
entries, one per line, from `QueryTool` through `AclGrantTool`. 30 tools
shipped with zero corresponding documentation for roughly two months before
this was noticed.

The doc itself has already been corrected in that same analysis pass (its
header now reads "37 tools", and the full registry table enumerates every
tool by category) — this task does NOT redo that edit. What remains is the
structural gap the incident exposed: (1) the doc's own "tool surface registry
stays in sync" requirement scenario explicitly defers enforcement to "future
work — phase10k doctor entry" (§ Requirement: tool surface registry stays in
sync), but phase10k (archived `.rulebook/archive/2026-04-30-phase10k_retention_daemon/`)
was the retention daemon, not a doctor check — a stale cross-reference baked
into the doc alongside the real fix; (2) the doc's separately-added
requirement "registry drift is caught before it reaches 30 undocumented
tools" already commits to a CI doc-coherence scan AND a `cortex-ops doctor`
diagnostic that block PRs on a drift of 2+ tools, but neither exists in code
yet; and (3) no connected agent has a runtime way to enumerate what the
37 tools do — `tools/list` returns full JSON-Schema descriptors (verbose,
built for protocol negotiation, not for an agent skimming "what can I do
here"), and nothing surfaces a compact summary.

## What Changes

1. Add a `cortex_capabilities` MCP tool to `ToolRegistry::default_set()`
   (`crates/cortex-mcp-server/src/tools.rs`) returning `{name,
   one_line_purpose, read_or_write}` for every tool currently registered —
   including itself once added (38 tools total post-change). Every tool's
   existing `descriptor()["description"]` already carries prose usable
   as `one_line_purpose`; `read_or_write` requires extending the `Tool`
   trait (or an adjacent static table) since no runtime-queryable R/W
   classification exists today — spec 20's registry table is the only
   place that distinction currently lives, as static prose.
2. Implement the `cortex-ops doctor-registry-sync` check that spec 20's
   "tool surface registry stays in sync" and "registry drift..." requirements
   already specify: parse the Registry table in `docs/specs/20-mcp-tool-surface.md`
   (row count + tool names), compare against `ToolRegistry::default_set()`
   (count + names) at `crates/cortex-cli/src/bin/cortex-ops/doctor.rs` (new
   sibling file `doctor_registry_sync.rs`, following the existing
   `doctor_synap_workers.rs` / `doctor_redaction_coverage.rs` one-file-per-check
   convention), and report any missing/extra tool names with severity scaling
   to the doc's committed "drift ≥2 tools blocks the PR" threshold.
3. Wire the new check into CI as a fast, path-scoped gate — mirroring
   `.github/workflows/dashboard-grep-gate.yml`'s pattern (no docker-compose
   stack, <2 minute budget, triggers only on changes to
   `crates/cortex-mcp-server/src/tools.rs` or the spec doc) rather than the
   heavy `doctor.yml` / `health-smoke.yml` workflows, since counting registry
   rows needs no live services. This is the PR-time gate; the nightly
   long-lived-stack schedule belongs to `phase30_live-e2e-smoke-and-doctor-wiring`
   (referenced, not duplicated, here).
4. Update spec 20's two existing placeholder scenarios to point at the
   concrete mechanism this task ships: replace "future work — phase10k
   doctor entry" with a reference to the implemented `doctor-registry-sync`
   check (and drop the stale phase10k cross-reference in the same edit),
   and update the "registry drift..." scenario to name `cortex_capabilities`
   + `doctor-registry-sync` as the enforcement mechanism instead of
   describing it only in the abstract.

## Impact

- Affected specs: `docs/specs/20-mcp-tool-surface.md` (MODIFIED — the two
  existing requirements noted above get their placeholders resolved; a new
  registry row is added for `cortex_capabilities` itself).
- Affected code: `crates/cortex-mcp-server/src/tools.rs` (new tool +
  `Tool` trait extension), `crates/cortex-cli/src/bin/cortex-ops/doctor.rs`
  and new `doctor_registry_sync.rs`, `crates/cortex-cli/src/bin/cortex-ops.rs`
  (subcommand dispatch wiring), new/extended `.github/workflows/*.yml`.
- Breaking change: NO — additive tool, additive CI gate, no existing
  behavior removed or changed shape.
- User benefit: any agent connected to Cortex can self-discover the full
  tool surface at runtime instead of needing source or doc access; future
  tool additions can no longer silently go undocumented for months the way
  this one did — the doc and the registry are mechanically kept honest.

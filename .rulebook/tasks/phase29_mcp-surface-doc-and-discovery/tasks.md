## 1. Runtime discovery — `cortex_capabilities` tool
- [x] 1.1 `Tool::read_or_write()` added to the trait (default `"read"`; the four mutating tools override: capture_memory, forget, feedback_record, acl_grant — the task predates acl_grant, also a write).
- [x] 1.2 `CapabilitiesTool::from_tools` builds `{name, one_line_purpose, read_or_write}` from the registry itself (purpose = first sentence of each tool's own descriptor description — no new prose); appends its own row so the advertised count matches tools/list.
- [x] 1.3 Registered via `ToolRegistry::push_capabilities()` as the LAST registry step so it can never miss a tool — registry is now **41** (the task text's "#38" predates phase27e path/compare: 38→40→41); spec 20 row added; count assertions updated across tools.rs/server.rs/transport_stdio.rs. Fix-en-route: a sloppy `== 40`→`== 41` batch replace corrupted ForgetTool's HTTP 400 mapping to 410 — caught by the suite, restored, 87/87 lib tests green.

## 2. `cortex-ops doctor-registry-sync` check
- [x] 2.1 `doctor_registry_sync.rs` shipped: pure `doc_tool_names` markdown-table parser (first-cell backticked `cortex_*` rows only) + `diff_names` both-direction diff + comparison against `ToolRegistry::default_set().names()` (new accessor); cortex-cli gained the cortex-mcp-server dep (no cycle).
- [x] 2.2 Exit codes: 0 in-sync, 1 = one-tool drift (warn), 2 = ≥2 drift (critical, spec 20's blocking threshold) or unreadable spec. Verified empirically: live run 41/41 → 0; one-row-removed doc → 1; two-rows-removed → 2.
- [x] 2.3 `DoctorRegistrySync { --spec, --json }` wired into the CLI dispatch with the `#[path]` module convention.

## 3. CI wiring — fast path only (do not duplicate phase30's scheduling)
- [x] 3.1 `.github/workflows/registry-sync-gate.yml` — path-scoped (tools.rs + spec 20), stackless, cargo-cached, runs the doctor on push/PR.
- [x] 3.2 Header comment explicitly defers the nightly stack schedule to phase30 and forbids adding `schedule:` here.

## 4. Close the loop on spec 20's existing placeholders
- [x] 4.1 Scenario now names `doctor-registry-sync` with its exit semantics; stale phase10k reference gone.
- [x] 4.2 Requirement text names both mechanisms (doctor = doc half, capabilities = runtime half) + the CI workflow by filename; cardinality example updated to 41. Stale "only WRITE" prose corrected to the four-write reality.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Spec 20: header count 41, capabilities row, corrected write prose, both scenarios; CHANGELOG entry.
- [x] 5.2 Tests: capabilities (41 entries, non-empty purposes, read|write valid, exact write-set assertion), one_line_purpose units, doc-parser + diff units, AND a live spec-vs-registry sync unit test so plain `cargo test` catches drift without CI.
- [x] 5.3 mcp-server lib 87/87; cortex-cli registry tests 3/3; live doctor run 41/41 exit 0; drift-1 doc exits 1 and drift-2 doc exits 2 with names printed (fails loudly).

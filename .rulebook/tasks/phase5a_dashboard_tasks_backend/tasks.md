## 1. Loader — filesystem walk
- [x] 1.1 Create `crates/cortex-api/src/tasks_loader.rs` with a `TaskLoader` struct holding workspace root + cached `Vec<TaskRow>` behind `RwLock`
- [x] 1.2 Resolve workspace root from the same source `cortex-api` already uses (env var or `cortex.toml`) — do not introduce a new knob
- [x] 1.3 Walk `.rulebook/tasks/*` (active) and `.rulebook/archive/*` (archived); ignore non-directories and the `README.md` index file
- [x] 1.4 For archived dirs, strip the `YYYY-MM-DD-` date prefix to recover the task `id`; capture the prefix as `archived_at`
- [x] 1.5 Parse `phase` from the id prefix using regex `^phase(\d+)([a-z]?)` — store both the canonical key (`phase2g`) and the numeric sort key
- [x] 1.6 Read `.metadata.json` when present (`status`, `createdAt`, `updatedAt`); fall back to dir mtime / archive date when missing
- [x] 1.7 Override `status` to `archived` for anything under `.rulebook/archive/` regardless of metadata content

## 2. Loader — content parsing
- [x] 2.1 Read `proposal.md`; extract first H1 as `title` (fallback: id) and first non-heading paragraph as `summary` (trim to 280 chars, single line)
- [x] 2.2 Read `tasks.md`; count `- [x]` (done) and `- [ ]` (pending) lines as `progress.done` / `progress.total`
- [x] 2.3 Group checklist items by H2 section (`## N. Section name`) → `Vec<{ section, items: Vec<{ text, done }> }>`; preserve file order
- [x] 2.4 List files under `specs/` recursively into `Vec<{ path: relative, name: filename }>`; no body inlined
- [x] 2.5 mtime-based cache: invalidate the cached row for a task when any of `proposal.md`, `tasks.md`, `.metadata.json` mtime changes; full directory listing is re-walked on a 30s TTL

## 3. Routes — list + summary
- [x] 3.1 Add `pub fn tasks_router()` in a new `crates/cortex-api/src/dashboard/tasks.rs` (or inline in `dashboard.rs` if no submodule split exists yet)
- [x] 3.2 `GET /v1/dashboard/tasks` — accept `status`, `phase`, `include_archived` (default true), `limit` (default 200, max 500), `offset` (default 0)
- [x] 3.3 Filter by status (multi-value), phase (multi-value, exact prefix match), and `include_archived` (when false, drop `archived` rows)
- [x] 3.4 Sort default: phase numeric asc → letter asc → updated_at desc; allow `?sort=updated_at|created_at|phase` with `&order=asc|desc`
- [x] 3.5 Response includes `tasks`, `total` (post-filter), `by_phase` (totals per phase across all rows pre-filter), `by_status` (totals across all rows pre-filter)
- [x] 3.6 `GET /v1/dashboard/tasks/summary` — return `{ total, completed, in_progress, pending, archived, completion_pct }` (rounded to 1 decimal)

## 4. Routes — detail
- [x] 4.1 `GET /v1/dashboard/tasks/{id}` — 404 when id not found in either active or archive
- [x] 4.2 Detail body: row fields + `proposal_md` (full file body) + `checklist` (sectioned) + `specs` (list)
- [x] 4.3 When the same id exists both active and archived (rename collisions), prefer the active version and mark `also_archived: true`

## 5. Wiring
- [x] 5.1 Register the new module in `crates/cortex-api/src/lib.rs`
- [x] 5.2 Construct `TaskLoader` once in `service.rs` (or wherever `DashboardState` is built) and inject via `Extension` / `with_state`
- [x] 5.3 Mount `tasks_router()` under `dashboard_router()` so it lives at `/v1/dashboard/tasks*`

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation — extend `docs/specs/16-dashboard.md` with the three new endpoints, full response shapes, and the workspace-root resolution rule
- [x] 6.2 Write tests covering the new behavior — unit tests in `tasks_loader.rs` (id-prefix stripping for archive dirs, checklist counting with mixed `[x]`/`[ ]`/non-checkbox lines, section grouping, phase parsing for `phase0`/`phase2g`/`phase4a`) and an integration test in `crates/cortex-api/tests/` against a fixture `.rulebook/` tree asserting list filters, summary aggregates, and detail body
- [x] 6.3 Run tests and confirm they pass — `cargo test -p cortex-api` is green (137 tests, 0 failures); clippy on touched modules is clean (7 pre-existing warnings in unrelated `dashboard.rs` graph-builder dead code remain — not introduced by this task)

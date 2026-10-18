# Reuse the Existing Dashboard

## 1.1 User correction and integration contract

The user explicitly permits functionality and integration changes but requires
the existing dashboard layout. The reference is the production renderer in
`gui/`, not the parallel static interface in `crates/cortex-local/src/web/`.
Preserve its Header, Sidebar, theme/tweaks, typography, spacing, memory cards,
tables and inspector patterns. Do not introduce a new visual identity.

The renderer already runs in browsers. Standalone mode shares presentational
shell components and existing styles, while a local adapter uses authenticated
POST `/api/*` operations. Legacy Electron/service mode remains available.
The standalone entry must bypass persisted legacy connections and SSE. Consume
the bootstrap token before rendering, remove the URL fragment and retain it only
in memory/sessionStorage. Scope every request and cache by exact repository.

Local views cover paged history, memory/decision inspection and capture/revision,
explicit relationships, classification queue, audit and feedback. Show real store
counts and provenance; do not invent Docker health, independent votes or model
consumption. Unsupported legacy-only operations do not issue background requests.
New local views reuse established visual components/classes and navigation shell.

Compile the existing renderer into the Cargo output directory and generate an
asset manifest for embedding. Source builds require Node and installed GUI
dependencies; installed executable requires neither Node nor Electron. No generated
bundle is committed. Serve only manifest-listed assets with exact MIME types,
same-origin security and no directory traversal. Script CSP remains self-only;
inline style attributes are required by the existing React components.

Independent implementation ownership under item 1.2: shell/bootstrap/local client,
local workspace views, local projections/API, and build/asset packaging. These
parts implement one integration item and do not reorder checklist sections.

Acceptance includes desktop/mobile screenshots compared with the original shell,
functional checks of local flows and adversarial authentication/scope checks.
Existing stored data and native agent memory are not rewritten. New authorization
for external classification is not implied by this interface correction.

## 2.3 Final acceptance

The final Windows release and installed executable share SHA-256
`150bfec3917c58718fa0a5a4b9c82e621912df1c3146d58e7cb45459d81e0207`.
Workspace format, all-target/all-feature check and strict Clippy pass. The full
serial all-feature suite passes 4,329 tests across 276 targets, with zero failures
and nine pre-existing ignored tests. An earlier passing run predates the final
review corrections and is retained separately. GUI renderer/Electron typechecks,
strict ESLint, 89 tests across 17 files, generated API contract comparison and
the legacy renderer build pass. Python runs 171 cases: 170 pass and one Windows
symlink privilege check skips. Remote CI execution is not claimed.

Final release browser checks confirm the original desktop layout at 1360x900 and
responsive mobile layout at 390x844, without document overflow. Full functional
acceptance additionally covers light/dark themes, 320px health, capture/revision,
historical sources, literal markup, graphs, classification enqueue, audit,
feedback and scope isolation. Header search resets pagination; a new startup URL
reloads the mounted session after server restart, including when sessionStorage
is blocked. No legacy API or provider call is made. Host antivirus injection was
blocked separately from application requests. Owned fixture servers stopped;
their temporary tokens are invalid. Screenshots contain fictional data only.

Claude reviewed only the authorized cortex-local source and found no high or
medium severity issue. Final fixes cover HEAD Content-Length parity, rebuild
triggers, URL-safe asset names, strict queue request fields and feedback auth
coverage. The stale-asset concern does not apply because the build explicitly
empties its guarded output directory. Independent frontend/backend/build reviews
approve the corrected result. Node 22.11 built successfully on this machine;
the documented supported build prerequisite remains Node 22.12 or newer.

Fourteen installed smoke checks pass with eighteen MCP tools and schema 3.
All 33 installed plugin files remain identical; this phase needs no plugin-cache
or hook changes. The previous executable is preserved under
`install-01M57HSE9MKHJ70X1WV0YK0P2G` and matches its original phase32 SHA-256.
All 1,662 original canonical files remain unchanged. The separately reverified
historical archive contains 69,634 unchanged files and 319,238,316 bytes.
Real inventory is 106,963 records/revisions across 37 repositories: 106,946 events,
17 memories and 1,812 links. Exact Cortex scope has 29 records/revisions, sixteen
events, thirteen memories and one link. Doctor reports standalone runtime with
no required services; classification is not configured in the validation process.

Fresh offline contract and feedback evaluations pass, but natural-language
readiness remains false and no retrieval/productivity gain is claimed. Active
client reload and model consumption remain unobserved. The 49 evidence files,
including final screenshots and hashes, are preserved with a checksum manifest at
`C:/Users/Bolado/.cortex/backups/phase37-dashboard-150bfec3/evidence`.
Post-gate whitespace cleanup regenerates four byte-identical embedded assets;
both GUI typechecks, strict lint and all 89 tests pass again. Supplemental proof
is stored in the sibling `formatting/` directory without changing sealed evidence.

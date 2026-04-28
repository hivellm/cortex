# GUI multi-connection — store, switcher, and management view

## ADDED Requirements

### Requirement: Persistent connection store
The GUI SHALL persist a list of `Connection` records across sessions
and expose CRUD operations through a single store hook.

#### Scenario: Built-in local connection seeded on first launch
Given a fresh install with no `connections.json`
When the GUI boots
Then a built-in `local` connection (id=`local`, label=`Local`, baseUrl=`http://127.0.0.1:17000`, auth=`none`) is present
And `local` cannot be deleted
And `local` is the active connection

#### Scenario: User-added connection round-trips through persistence
Given the user adds a new connection labelled `Staging`
When the GUI is closed and reopened
Then the new connection appears in the store with the same fields
And its bearer token (if any) is restored via `safeStorage`

### Requirement: Active-connection switcher in the header
The header SHALL show the active connection and let the user switch
between known connections without a page reload.

#### Scenario: Switch via header dropdown
Given two connections `local` and `Staging` exist
And `local` is active
When the user opens the header dropdown and clicks `Staging`
Then `Staging` becomes the active connection
And every visible view re-fetches against the staging base URL
And cached data from `local` is not surfaced

### Requirement: Manage view
The GUI SHALL provide a `/connections` route that lists, edits,
duplicates, removes, and tests connections.

#### Scenario: Test probe round-trips
Given an editable connection form with a base URL filled in
When the user clicks "Test"
Then the GUI sends `GET <baseUrl>/v1/dashboard/status` with the form's auth
And surfaces success (uptime + service id) or failure (status + message) inline

#### Scenario: Cannot delete active connection
Given `Staging` is the active connection
When the user clicks Delete on `Staging`
Then the GUI refuses and explains the user must switch first

### Requirement: Per-connection React Query scoping
Every query key in the renderer MUST be prefixed with the active
`connection.id` so switching connections does not surface cached
data from another backend.

#### Scenario: Cache isolation between connections
Given the user has loaded `/timeline` against `local`
When the user switches to `Staging`
Then the timeline view shows a fresh loading state until `Staging` data arrives
And the `local` cache is preserved (not invalidated) for fast switch-back

### Requirement: Auth header injection
When the active connection's `auth.kind` is `bearer`, every request
MUST carry `Authorization: Bearer <token>`. When `auth.kind` is
`none`, no `Authorization` header is sent.

#### Scenario: Bearer token attached
Given the active connection has `auth.kind = "bearer"` and `auth.token = "abc"`
When any fetcher in `gui/src/lib/api.ts` runs
Then the outgoing request includes header `Authorization: Bearer abc`

#### Scenario: Token never logged
Given a connection has a bearer token
When the GUI logs network errors to the console
Then the token is redacted to `Bearer <REDACTED>` in any log line that quotes the request

### Requirement: Browser fallback warning
When the GUI is loaded in a plain browser (no Electron preload bridge),
the manage view MUST surface a banner saying tokens are stored in
`localStorage` without OS-keychain protection.

### Requirement: Health-probe gauge
The header chip MUST reflect a per-connection health state derived
from a probe against `/v1/dashboard/status` no more frequently than
once every 30 seconds per connection.

#### Scenario: Down backend renders red
Given `Staging` returns HTTP 5xx or refuses connection on `/v1/dashboard/status`
When 30 s have passed since the last probe
Then the header chip for `Staging` (when active) shows a red dot
And the dropdown row for `Staging` (when not active) shows a red dot

## MODIFIED Requirements

### Requirement: api.ts base URL is dynamic
The fetchers in `gui/src/lib/api.ts` MUST resolve the base URL from
the active connection at call time rather than the previous hard-coded
`http://127.0.0.1:17000` constant.

#### Scenario: Fetcher binds to the active connection
Given the active connection's `baseUrl` is `https://cortex.example.com`
When `getDashboardStatus()` runs
Then the request goes to `https://cortex.example.com/v1/dashboard/status`

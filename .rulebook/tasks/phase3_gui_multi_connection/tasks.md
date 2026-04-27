## 1. Connection store + persistence
- [ ] 1.1 Define `Connection` type (id, label, baseUrl, auth: bearer|basic|none, color, createdAt) under `gui/src/lib/connections/types.ts`
- [ ] 1.2 Add schema validation (zod or hand-rolled) for safe load/save round-trip
- [ ] 1.3 Implement `connectionsStore` (Zustand or context) with CRUD + `activeConnectionId`
- [ ] 1.4 Persist non-secret fields to `userData/connections.json` via Electron IPC; tokens via `safeStorage` when available
- [ ] 1.5 Seed a non-removable built-in `local` connection on first launch

## 2. Electron IPC bridge
- [ ] 2.1 Add `connections.read` / `connections.write` IPC channels in electron/main.ts
- [ ] 2.2 Add `secret.read` / `secret.write` channels using `safeStorage`
- [ ] 2.3 Expose via preload as `window.cortex.connections.*`
- [ ] 2.4 Browser-only fallback to `localStorage` with a plaintext-token warning

## 3. API layer rewrite
- [ ] 3.1 Replace static `BASE_URL` in gui/src/lib/api.ts with per-call `Connection`
- [ ] 3.2 `apiFor(connection)` factory returning typed fetchers bound to that backend
- [ ] 3.3 Inject `Authorization: Bearer <token>` when the connection carries one
- [ ] 3.4 Single `useApi()` hook reading the active connection from the store

## 4. React Query scoping
- [ ] 4.1 Prefix every query key with `connection.id`
- [ ] 4.2 No manual invalidate on switch — new key pulls fresh data
- [ ] 4.3 GC cached data older than 5 min for non-active connections

## 5. Header switcher
- [ ] 5.1 Active-connection chip in Header.tsx (label + color dot + status)
- [ ] 5.2 Dropdown with all known connections + "Manage..." entry
- [ ] 5.3 Status dot from a debounced 30s probe per connection

## 6. Manage view
- [ ] 6.1 New route `/connections` (views/Connections.tsx)
- [ ] 6.2 Table with edit / duplicate / remove (local non-removable)
- [ ] 6.3 Form: label, base URL, auth selector, color picker
- [ ] 6.4 Test button against `/v1/dashboard/status`
- [ ] 6.5 Delete confirms; can't delete the active connection without switching first

## 7. cortex-api side
- [ ] 7.1 Audit CORS for `/v1/*` against non-Electron browser origins
- [ ] 7.2 Document bearer-token contract in docs/specs/16-dashboard.md §Connections
- [ ] 7.3 Add `CORTEX_API_ALLOWED_ORIGINS` env if needed; surface in .env.example

## 8. Tests
- [ ] 8.1 Unit: connections store CRUD + persistence round-trip
- [ ] 8.2 Unit: `apiFor(connection)` injects correct base URL + auth
- [ ] 8.3 Unit: query-key scoping prevents cross-connection cache hits
- [ ] 8.4 E2E: add → test → switch → manage delete

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 9.1 Update or create documentation covering the implementation
- [ ] 9.2 Write tests covering the new behavior
- [ ] 9.3 Run tests and confirm they pass

## 1. Stats grid
- [ ] 1.1 Add `useQuery` for `/v1/dashboard/overview` to Timeline
- [ ] 1.2 Maintain a rolling buffer (in-memory, max 20 samples) of `events_total` deltas across consecutive overview polls
- [ ] 1.3 Render 4 stat tiles via the existing `.stats-grid` styles: Events captured / Repos active / Tool calls vs Turns ratio / Sessions
- [ ] 1.4 Each tile uses `Sparkline` for trend where a series exists; tiles without a series omit `.stat__spark`
- [ ] 1.5 No fake numbers — labels reflect what we measure (no "P95", no "events/min" until phase2g lands)

## 2. Stream controls
- [ ] 2.1 Add `live` boolean state (default `true`); `refetchInterval` becomes `live ? 5000 : false`
- [ ] 2.2 `Pause stream` / `Resume` button in the view actions area, using `play` / `pause` Icon names already in the atom set
- [ ] 2.3 Footer status pill reads `● connected` (green) when `live`, `○ paused` (grey) otherwise
- [ ] 2.4 When paused, the buffer count display still shows the last fetched count

## 3. New-row animation
- [ ] 3.1 Track `seenIds: Set<string>` in a ref; on each fetch, diff incoming events vs `seenIds`, mark the diff as `newIds`
- [ ] 3.2 Pass `isNew` to `TimelineRow`; row applies `is-new` class while `isNew && !timedOut`
- [ ] 3.3 Setup a 700 ms timer that drops the id from the active `newIds` set
- [ ] 3.4 First-ever render bypasses the diff (when `seenIds.size === 0`, populate `seenIds` and emit no `newIds`)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation — extend `gui/README.md` Timeline section with the stats tiles + pause control + new-row flash behavior
- [ ] 4.2 Write tests covering the new behavior — Vitest unit on the rolling-buffer logic (push 5 deltas → returns last 5 in order); React Testing Library on `Pause stream` toggling `refetchInterval`; mock `useQuery` returning two snapshots → assert `is-new` class applied to net-new ids and removed after 700 ms
- [ ] 4.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test`, `pnpm lint`

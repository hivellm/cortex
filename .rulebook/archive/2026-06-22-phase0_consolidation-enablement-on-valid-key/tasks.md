## 0. SUPERSEDED (2026-06-21)
- [x] 0.1 Superseded by the CLI-only resolution: the consolidator now summarises through the LOCAL logged-in `claude` CLI (no Anthropic API key), so the original premise (need a valid `ANTHROPIC_API_KEY` + Opus cost authorization) no longer applies. Done in `phase0_recurrent-consolidation-and-retention` §4 (commits 0701630 / 58398b0) + host daemon + autostart (`docs/consolidator-host-daemon.md`). The items below are retained for history only; none are actionable.

## 1. Gate (blocking precondition)
- [x] 1.1 SUPERSEDED — CLI-only resolution removed the API-key requirement (see §0.1).
- [x] 1.2 SUPERSEDED — no Opus API spend; the local claude CLI subscription is used.
- [x] 1.3 SUPERSEDED — live ingestion restored in `phase0_live-ingestion-staleness` §2.

## 2. Enable the trigger producer
- [x] 2.1 DONE via §0.1 resolution — `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=true` set + classifier recreated (commit 58398b0).
- [x] 2.2 DONE — classifier config shows `consolidator_trigger_enabled: true`; triggers feed the host daemon.

## 3. Confirm cadence runs for real
- [x] 3.1 SUPERSEDED — consolidation runs OUTSIDE docker as a host daemon (claude CLI); in-container crons are moot.
- [x] 3.2 DONE — host daemon + CLI requirement documented in `docs/consolidator-host-daemon.md`; no API key needed.

## 4. Initial backfill + verification
- [x] 4.1 DONE — `cortex-consolidator nightly --all` (216 sessions) ran on the host via claude CLI.
- [x] 4.2 DONE — consolidations publish OK to ingestion (232 consolidations in `cortex-cortex-consolidations`).
- [x] 4.3 SUPERSEDED — watchdog `consolidation_*` alarms track recurrence; covered by the host daemon.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — DONE: `docs/consolidator-host-daemon.md` + CHANGELOG (CLI-only consolidator).
- [x] 5.2 Write tests covering the new behavior — N/A: resolution covered by `phase0_recurrent-consolidation-and-retention` §4 tests; no new code here.
- [x] 5.3 Run tests and confirm they pass — DONE: workspace green (3137) at the resolving commits.

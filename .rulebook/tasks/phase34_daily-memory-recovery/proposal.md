# Proposal: phase34_daily-memory-recovery

## Why
Cortex captured and classified nothing for two months: the host adapter daemon that receives every hook had no working autostart (schtasks needs admin), died on 2026-07-06, and fail-open hooks hid the outage. Installed binaries were also 15 commits stale. The operator asked for Cortex to become actually useful day to day: decisions remembered and errors avoided across sessions and agents. Analysis: docs/analysis/daily-agent-memory-2026-10/ (01-05).

## What Changes
Bring capture back with supervision and a loud health channel; stop memory consolidation harm; deliver active lessons/decisions with provenance into the default pre-thinking path with revision-aware caching, durable feedback and delivery evidence; add a bounded Jev backend at capture time for lesson candidates; cut idle footprint; prove the loop with one seeded end-to-end test on the real host install. Joint work: Claude implements, Codex gpt-6-astra reviews each section and owns the Jev section.

## Impact
- Affected specs: none
- Affected code: none
- Breaking change: NO
- User benefit: none

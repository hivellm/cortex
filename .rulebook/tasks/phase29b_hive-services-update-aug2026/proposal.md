# Proposal: phase29b_hive-services-update-aug2026

Source: user request 2026-08-02 ("o nexus e o synap foi atualizando, da
uma olhada e atualize, tanto no container quanto as sdks" + "cria uma
task dessa atualizacao pra manter registro").

## Why

The Hive services move faster than Cortex's pins. This task is the
running record of the August-2026 update round so the decisions (what
was bumped, what was deliberately NOT bumped, and why) survive the
session that made them. It follows the July round that took the stack
to Vectorizer 3.5.0 / Nexus 2.5.0 / Synap 1.0.0 and surfaced three
Nexus 2.5 dialect regressions (`_id` projection null → nexus#29,
undirected patterns returning zero rows, plus the CONTAINS behavior
still under investigation in phase29 §5) and the Synap 1.0
room-not-found ERROR-spam class (fixed by boot-time
`get_or_create_room`, commit 95f32c7).

## Published state at the time of writing (verified 2026-08-02)

| Component | crates.io SDK | Docker Hub | Local sibling repo |
|-----------|---------------|------------|--------------------|
| Synap     | synap-sdk **1.3.0** | hivehub/synap:**1.3.0** | 1.3.0 |
| Nexus     | nexus-graph-sdk **2.5.0** (no 3.x published) | hivehub/nexus:**3.0.0-alpha** only (no stable 3.x) | 3.0.0 |
| Vectorizer| vectorizer-sdk 3.5.0 (unchanged) | 3.5.0-fastembed (unchanged) | 3.5.0 |

## What Changes

- Synap: SDK 1.0 → 1.3 (workspace pin + lockfile) and container
  1.0.0 → 1.3.0. Zero API breakage (workspace `cargo check
  --all-targets` clean on the new pin).
- Nexus: **deliberately kept at 2.5.0/2.5.0** — the only newer image is
  an `-alpha` tag with NO matching SDK on crates.io, and a major (3.0)
  may carry a storage migration; production data does not move onto an
  alpha without an explicit user instruction. Revisit when
  `hivehub/nexus:3.0.0` (stable) + `nexus-graph-sdk 3.x` publish —
  that bump also unblocks the rmcp RUSTSEC-2026-0189 re-audit
  (hivellm/nexus#28).
- Operational finding recorded: Synap stream ROOMS ARE EPHEMERAL — a
  synap-only container restart wipes them, and the boot-time declare
  (95f32c7) only covers workers that boot AFTER synap. Restarting
  synap under long-running workers reproduced ~2400 ERROR lines/min
  until the consumers were restarted. Runtime re-declare on
  `Room not found` is the structural fix (tasks §2).

## Impact

- Affected specs: `docs/specs/03-local-stack.md` (image pin table).
- Affected code: workspace `Cargo.toml` / `Cargo.lock`,
  `docker-compose.yml`, `crates/cortex-workers/src/synap_worker/`
  (§2 runtime re-declare), the four worker bins.
- Breaking change: NO.
- User benefit: stack tracks upstream releases with an auditable
  record; room-spam class eliminated structurally instead of
  per-incident.

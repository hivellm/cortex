## 1. Compose authoring
- [x] 1.1 `docker-compose.yml` with Vectorizer, Nexus, Synap, Meilisearch services
- [x] 1.2 Pinned images / build contexts for each service + healthchecks + named volumes
- [x] 1.3 Host-port remaps exposed via env vars (`VECTORIZER_HOST_PORT`, `NEXUS_HOST_PORT`, etc.) and documented in `.env.example`

## 2. Environment
- [x] 2.1 `.env.example` enumerating every variable (endpoints, master key, mode toggles, retention knobs)
- [x] 2.2 `.env` ignored by git via root `.gitignore` (entry already present as `.env`)

## 3. Seed plumbing
- [x] 3.1 `cortex-ops plan` emits the bootstrap plan (collections / Cypher / indexes / streams / KV) as JSON, sourced from cortex-storage constants
- [x] 3.2 `bin/cortex-init.sh` consumes the plan; applies Meilisearch settings idempotently and records intent for Vectorizer / Nexus / Synap (those workers re-apply on boot under specs 06 / 07 / 04)
- [x] 3.3 Every invocation is idempotent against a warm stack

## 4. Operator ergonomics
- [x] 4.1 `bin/cortex-up`, `bin/cortex-down`, `bin/cortex-reset`, `bin/cortex-logs`, `bin/cortex-doctor` bash wrappers
- [x] 4.2 `bin/cortex-up.ps1`, `bin/cortex-down.ps1`, `bin/cortex-doctor.ps1` for Windows parity
- [x] 4.3 `Makefile` with `up / down / reset / logs / doctor / smoke / build / check / test / clippy / fmt / plan` targets

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation (spec 03 flipped to 🟢 in [docs/specs/00-index.md](../../../docs/specs/00-index.md) and [03-local-stack.md](../../../docs/specs/03-local-stack.md))
- [x] 5.2 Write tests covering the new behavior (`docker compose config --quiet` validates the compose syntax; `cortex-ops plan` exercised; storage constants covered by the 31 cortex-storage tests)
- [x] 5.3 Run tests and confirm they pass (`cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — 60 tests pass, 0 warnings)

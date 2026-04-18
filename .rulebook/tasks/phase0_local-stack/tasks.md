## 1. Compose authoring
- [ ] 1.1 `docker-compose.yml` with Vectorizer, Nexus, Synap, Meilisearch services
- [ ] 1.2 Pinned image tags + healthchecks + named volumes
- [ ] 1.3 Port map + internal network; ports surfaced in `.env.example`

## 2. Environment
- [ ] 2.1 `.env.example` enumerating every variable (endpoints, API keys, namespace prefix)
- [ ] 2.2 `.env.local` ignored by git; documented in README

## 3. Seed scripts
- [ ] 3.1 `scripts/seed-vectorizer.sh` creates the six cortex-* collections (spec 02)
- [ ] 3.2 `scripts/seed-nexus.sh` runs constraint + index Cypher (spec 07)
- [ ] 3.3 `scripts/seed-meili.sh` applies `settings.v1.json` per index (spec 08)
- [ ] 3.4 All seed scripts idempotent on re-run

## 4. Operator ergonomics
- [ ] 4.1 `Makefile` targets: `up`, `down`, `logs`, `reset` (volumes purged), `smoke`
- [ ] 4.2 `make smoke` publishes one event through the full pipeline and checks all four backends
- [ ] 4.3 Cross-platform note for Windows users (WSL2 requirement)

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/03-local-stack.md` status flag to 🟢 + README "Getting started" section
- [ ] 5.2 CI smoke test: GitHub Actions job that runs `make up && make smoke && make down`
- [ ] 5.3 Confirm CI green on main branch

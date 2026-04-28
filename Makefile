.PHONY: help up down reset logs doctor doctor-consistency smoke build check test clippy fmt plan

help:
	@echo "Cortex local stack targets:"
	@echo "  make up        - docker compose up + first-time init"
	@echo "  make down      - stop the stack (volumes preserved)"
	@echo "  make reset     - destroy volumes + restart clean"
	@echo "  make logs S=<svc> - tail logs for one (or all) service"
	@echo "  make doctor    - liveness probe against every backend"
	@echo "  make doctor-consistency - cross-backend coverage doctor (phase4d/h)"
	@echo "  make smoke     - end-to-end smoke check"
	@echo
	@echo "Cortex workspace targets:"
	@echo "  make build / check / test / clippy / fmt"
	@echo "  make plan      - emit bootstrap plan JSON via cortex-ops"

up:
	bin/cortex-up

down:
	bin/cortex-down

reset:
	bin/cortex-reset

logs:
	bin/cortex-logs $(S)

doctor:
	bin/cortex-doctor

# Phase4j — cross-backend consistency doctor used by the CI gate.
# Reads the same env vars the streaming workers honour:
#   CORTEX_ARCHIVE_ROOT             archive root (defaults to ~/.cortex/archive)
#   CORTEX_FULLTEXT_MEILI_URL       Meilisearch base URL (required)
#   CORTEX_FULLTEXT_MEILI_API_KEY   Meilisearch master key (optional)
#   CORTEX_EMBEDDER_VECTORIZER_URL  Vectorizer base URL (optional — Vectorizer
#                                   probe runs only when URL + creds present)
#   CORTEX_EMBEDDER_VECTORIZER_USER admin username (optional)
#   CORTEX_EMBEDDER_VECTORIZER_PASSWORD admin password (optional)
#   CORTEX_NEXUS_URL                Nexus base URL (optional)
# Emits the markdown table on stdout; exits non-zero on any
# `inconsistent` row. The CI gate captures the JSON variant.
doctor-consistency:
	cargo run -q -p cortex-ops -- doctor-consistency

smoke: up doctor plan
	@echo "smoke ok"

build:
	cargo build --workspace

check:
	cargo check --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

plan:
	cargo run -q -p cortex-ops -- plan --pretty

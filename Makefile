.PHONY: help up down reset logs doctor smoke build check test clippy fmt plan

help:
	@echo "Cortex local stack targets:"
	@echo "  make up        - docker compose up + first-time init"
	@echo "  make down      - stop the stack (volumes preserved)"
	@echo "  make reset     - destroy volumes + restart clean"
	@echo "  make logs S=<svc> - tail logs for one (or all) service"
	@echo "  make doctor    - health probe against every backend"
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

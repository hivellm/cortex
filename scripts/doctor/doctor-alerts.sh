#!/usr/bin/env bash
# Phase8e — `cortex-ops doctor-alerts` wrapper. Lists every
# persisted silent-drop alert under ~/.cortex/alerts. Exit codes:
#   0 — no Critical alerts active
#   2 — at least one Critical alert active
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- doctor-alerts "$@"

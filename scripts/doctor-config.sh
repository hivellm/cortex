#!/usr/bin/env bash
# Phase8d — `cortex-ops doctor-config` wrapper. Runs the config-
# coherence audit and forwards the exit code:
#   0 — all findings ok
#   1 — at least one warn
#   2 — at least one critical (e.g. adapter.toml.endpoint != .env)
#
# Usage:
#   scripts/doctor-config.sh             text table
#   scripts/doctor-config.sh --json      machine-readable JSON
#
# Pass extra args (--workspace, --adapter-toml) verbatim through to
# cortex-ops.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- doctor-config "$@"

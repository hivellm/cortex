#!/usr/bin/env bash
# Phase9a — `cortex-ops retention-sweep` wrapper. Runs one tier-
# transition pass (FP32 -> PQ at 30 d, PQ -> Binary at 365 d) and
# exits. Idempotent + concurrency-safe.
#
# Exit codes:
#   0 — sweep completed (records demoted / dropped within ceiling)
#   1 — error-rate ceiling tripped or hard failure
#   2 — another sweep is already in flight
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- retention-sweep "$@"

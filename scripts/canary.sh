#!/usr/bin/env bash
# Phase8f — `cortex-ops canary` wrapper. Fires a synthetic
# PostToolUse frame through the daemon's IPC and waits for it to
# land in the archive. Exit codes:
#   0 — round-trip succeeded
#   1 — transport / connect error
#   2 — deadline elapsed without observing the marker
#
# Usage:
#   scripts/canary.sh                                default
#   scripts/canary.sh --hook UserPromptSubmit        other hook
#   scripts/canary.sh --json                         JSON output
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- canary "$@"

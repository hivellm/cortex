#!/usr/bin/env bash
# Phase9d — `cortex-ops pii-enforce` wrapper. Synthetic cohort
# preview today; production wiring lands with phase9k.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- pii-enforce "$@"

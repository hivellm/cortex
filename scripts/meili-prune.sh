#!/usr/bin/env bash
# Phase9f — `cortex-ops meili-prune` wrapper.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- meili-prune "$@"

#!/usr/bin/env bash
# Phase9b — `cortex-ops rollup` wrapper. Compacts archive
# partitions per spec 19: hourly -> daily at 90 d, daily ->
# monthly at 365 d, three-year drop at 1095 d.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- rollup "$@"

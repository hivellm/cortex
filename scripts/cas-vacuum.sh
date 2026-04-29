#!/usr/bin/env bash
# Phase9c — `cortex-ops cas-vacuum` wrapper. Deletes orphan CAS
# blobs and reclaims disk via SQLite VACUUM. Refuses to drop more
# than 50% of total blobs without --force.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- cas-vacuum "$@"

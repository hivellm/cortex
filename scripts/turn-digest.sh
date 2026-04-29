#!/usr/bin/env bash
# Phase9e — `cortex-ops turn-digest` wrapper. Synthetic preview today;
# production walker lands with phase9k.
set -u
exec cargo run --quiet --release -p cortex-cli --bin cortex-ops -- turn-digest "$@"

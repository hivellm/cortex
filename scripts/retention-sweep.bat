@echo off
REM Phase9a — `cortex-ops retention-sweep` wrapper. Runs one tier-
REM transition pass (FP32 -> PQ at 30 d, PQ -> Binary at 365 d) and
REM exits. Idempotent + concurrency-safe.
REM
REM Exit codes:
REM   0 — sweep completed (records demoted / dropped within ceiling)
REM   1 — error-rate ceiling tripped or hard failure
REM   2 — another sweep is already in flight
REM
REM Usage:
REM   scripts\retention-sweep.bat                       run live
REM   scripts\retention-sweep.bat --dry-run             plan only
REM   scripts\retention-sweep.bat --time-travel 2030-01-01T00:00:00Z
REM   scripts\retention-sweep.bat --json                machine output
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- retention-sweep %*
exit /b %ERRORLEVEL%

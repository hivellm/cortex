@echo off
REM Phase9b — `cortex-ops rollup` wrapper. Compacts archive
REM partitions per spec 19: hourly -> daily at 90 d, daily ->
REM monthly at 365 d, three-year drop at 1095 d (with kind +
REM pii_risk whitelist). Quarantines `*.corrupted*` and orphan
REM `*.tmp` files on entry.
REM
REM Usage:
REM   scripts\rollup.bat                              run all granularities
REM   scripts\rollup.bat --granularity hourly-to-daily
REM   scripts\rollup.bat --dry-run
REM   scripts\rollup.bat --time-travel 2030-01-01T00:00:00Z
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- rollup %*
exit /b %ERRORLEVEL%

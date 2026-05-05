@echo off
REM Phase8e — `cortex-ops doctor-alerts` wrapper. Lists every
REM persisted silent-drop alert under ~\.cortex\alerts. Exit codes:
REM   0 — no Critical alerts active
REM   2 — at least one Critical alert active
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- doctor-alerts %*
exit /b %ERRORLEVEL%

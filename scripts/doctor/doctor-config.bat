@echo off
REM Phase8d — `cortex-ops doctor-config` wrapper. Runs the config-
REM coherence audit and forwards the exit code:
REM   0 — all findings ok
REM   1 — at least one warn
REM   2 — at least one critical (e.g. adapter.toml.endpoint != .env)
REM
REM Usage:
REM   scripts\doctor-config.bat            text table
REM   scripts\doctor-config.bat --json     machine-readable JSON
REM
REM Pass extra args (--workspace, --adapter-toml) verbatim through to
REM cortex-ops.
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- doctor-config %*
exit /b %ERRORLEVEL%

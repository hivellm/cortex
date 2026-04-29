@echo off
REM Phase8f — `cortex-ops canary` wrapper. Fires a synthetic
REM PostToolUse frame through the daemon's IPC and waits for it to
REM land in the archive. Exit codes:
REM   0 — round-trip succeeded
REM   1 — transport / connect error
REM   2 — deadline elapsed without observing the marker
REM
REM Usage:
REM   scripts\canary.bat                                default
REM   scripts\canary.bat --hook UserPromptSubmit        other hook
REM   scripts\canary.bat --json                         JSON output
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- canary %*
exit /b %ERRORLEVEL%

@echo off
REM Phase9d — `cortex-ops pii-enforce` wrapper. Today's surface is
REM a dry-run probe against a synthetic cohort suite so operators
REM can verify the matcher logic. Production backend wiring lands
REM with phase9k's cron scheduler.
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- pii-enforce %*
exit /b %ERRORLEVEL%

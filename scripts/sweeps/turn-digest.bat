@echo off
REM Phase9e — `cortex-ops turn-digest` wrapper. Synthetic cohort
REM preview against the in-memory backend today; production walker
REM (Parquet + classifier + embedder + Nexus + rewriter) lands
REM with phase9k's cron scheduler.
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- turn-digest %*
exit /b %ERRORLEVEL%

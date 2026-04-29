@echo off
REM Phase9f — `cortex-ops meili-prune` wrapper. Synthetic preview
REM today; production walker lands with phase9k.
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- meili-prune %*
exit /b %ERRORLEVEL%

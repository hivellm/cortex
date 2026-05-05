@echo off
REM Phase9c — `cortex-ops cas-vacuum` wrapper. Deletes orphan CAS
REM blobs (refcount=0 AND last_referenced<now-30d) and reclaims
REM disk via SQLite VACUUM. Refuses to drop more than 50%% of
REM total blobs without --force (catastrophic-deletion safeguard).
setlocal EnableDelayedExpansion
cargo run --quiet --release -p cortex-cli --bin cortex-ops -- cas-vacuum %*
exit /b %ERRORLEVEL%

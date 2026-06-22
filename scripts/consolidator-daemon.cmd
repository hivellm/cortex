@echo off
REM ---------------------------------------------------------------------------
REM Cortex consolidator daemon (host) — runs OUTSIDE docker so it can use the
REM local, logged-in `claude` CLI (claude -p), with NO Anthropic API key.
REM
REM Registered to run at logon via Task Scheduler (task name
REM "CortexConsolidatorDaemon") so consolidation resumes automatically after a
REM machine reboot. It reads the same data the containers bind-mount
REM (%USERPROFILE%\.cortex), pulls consolidation triggers from Synap, and
REM publishes the resulting consolidations to cortex-ingestion on the host.
REM
REM Host port map (container -> host): synap 15500 -> 17003,
REM cortex-ingestion 17010 -> 17010.
REM ---------------------------------------------------------------------------
setlocal

set "CORTEX_ARCHIVE_ROOT=%USERPROFILE%\.cortex\archive"
set "CORTEX_METADATA_DB=%USERPROFILE%\.cortex\metadata.sqlite"
set "CORTEX_INGESTION_URL=http://127.0.0.1:17010"
set "SYNAP_BASE_URL=http://127.0.0.1:17003"
set "CLAUDE_CODE_BIN=%USERPROFILE%\.local\bin\claude.exe"

set "CONSOLIDATOR=%USERPROFILE%\.cargo\bin\cortex-consolidator.exe"
set "LOG=%USERPROFILE%\.cortex\consolidator-daemon.log"

echo [%DATE% %TIME%] consolidator-daemon starting (host, claude CLI) >> "%LOG%"
"%CONSOLIDATOR%" daemon >> "%LOG%" 2>&1
echo [%DATE% %TIME%] consolidator-daemon exited code %ERRORLEVEL% >> "%LOG%"

endlocal

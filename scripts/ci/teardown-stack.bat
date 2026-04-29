@echo off
REM Phase8h — Windows companion to teardown-stack.sh. Reads
REM $CORTEX_PIDS_FILE and kills every pid via taskkill. Idempotent.
setlocal EnableDelayedExpansion

set "PIDS_FILE=%CORTEX_PIDS_FILE%"
if "!PIDS_FILE!"=="" (
  if "%CORTEX_HOME%"=="" (
    set "PIDS_FILE=%TEMP%\cortex-pids"
  ) else (
    set "PIDS_FILE=%CORTEX_HOME%\pids"
  )
)

if not exist "!PIDS_FILE!" (
  echo teardown-stack: !PIDS_FILE! missing; nothing to kill
  exit /b 0
)

for /f "usebackq delims=" %%P in ("!PIDS_FILE!") do (
  if not "%%P"=="" (
    echo teardown-stack: taskkill %%P
    taskkill /PID %%P /F /T >nul 2>&1
  )
)

del /Q "!PIDS_FILE!" 2>nul
echo teardown-stack: done
exit /b 0

@echo off
REM Phase8h — Windows companion to boot-stack.sh. Spawns
REM cortex-ingestion + cortex-api + cortex-adapter-claude-code in
REM the background, polls /v1/health, exits 0 once `overall` is
REM `ok` or `degraded`.
REM
REM Required env: CORTEX_HOME (isolated working dir).
REM Optional:     CORTEX_PIDS_FILE, CORTEX_BOOT_TIMEOUT_SECS.
setlocal EnableDelayedExpansion

if "%CORTEX_HOME%"=="" (
  echo CORTEX_HOME must be set 1^>^&2
  exit /b 1
)
if not exist "%CORTEX_HOME%" mkdir "%CORTEX_HOME%"
if not exist "%CORTEX_HOME%\archive" mkdir "%CORTEX_HOME%\archive"
if not exist "%CORTEX_HOME%\logs" mkdir "%CORTEX_HOME%\logs"

set "PIDS_FILE=%CORTEX_PIDS_FILE%"
if "!PIDS_FILE!"=="" set "PIDS_FILE=%CORTEX_HOME%\pids"
type nul > "!PIDS_FILE!"

set "TIMEOUT_SECS=%CORTEX_BOOT_TIMEOUT_SECS%"
if "!TIMEOUT_SECS!"=="" set "TIMEOUT_SECS=60"

set "CORTEX_ARCHIVE_ROOT=%CORTEX_HOME%\archive"

REM Spawn each daemon detached; grab pid from PowerShell helper so
REM teardown-stack.bat can kill it. cmd.exe `start` does not surface
REM pids natively, hence the PS dance.
for %%B in (cortex-ingestion cortex-api cortex-adapter-claude-code) do (
  for /f %%P in ('powershell -NoProfile -Command "(Start-Process -PassThru -FilePath cargo -ArgumentList 'run','--quiet','--release','-p','%%B' -RedirectStandardOutput '%CORTEX_HOME%\logs\%%B.log' -RedirectStandardError '%CORTEX_HOME%\logs\%%B.err.log' -WindowStyle Hidden).Id"') do (
    echo %%P>>"!PIDS_FILE!"
  )
)

echo boot-stack: waiting for /v1/health to come up...
set /a deadline=%TIMEOUT_SECS%
:poll_loop
  for /f %%R in ('curl -fsS --max-time 2 http://127.0.0.1:17000/v1/health 2^>nul') do (
    set "RAW=%%R"
  )
  if defined RAW (
    echo !RAW! | findstr /C:"\"overall\":\"ok\"" >nul && (
      echo boot-stack: ready ^(overall=ok^)
      exit /b 0
    )
    echo !RAW! | findstr /C:"\"overall\":\"degraded\"" >nul && (
      echo boot-stack: ready ^(overall=degraded^)
      exit /b 0
    )
  )
  timeout /t 1 /nobreak >nul
  set /a deadline-=1
  if !deadline! gtr 0 goto poll_loop

echo boot-stack: timeout after %TIMEOUT_SECS%s waiting for /v1/health 1>&2
exit /b 1

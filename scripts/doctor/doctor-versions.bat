@echo off
REM Phase8c — print the cortex-api /v1/health/versions drift report
REM and exit non-zero when any running binary is behind workspace
REM HEAD. Designed for operator use ("did everyone restart after the
REM last cargo build?") + CI smoke jobs.
REM
REM Usage:
REM   scripts\doctor-versions.bat            default endpoint
REM   scripts\doctor-versions.bat -u http://...
REM
REM Exit codes:
REM   0 — all_in_sync = true
REM   1 — at least one binary's git_sha != workspace HEAD
REM   2 — could not reach /v1/health/versions at all
setlocal EnableDelayedExpansion

set "ENDPOINT=%CORTEX_API_URL%"
if "!ENDPOINT!"=="" set "ENDPOINT=http://127.0.0.1:17000"
set "ENDPOINT=!ENDPOINT!/v1/health/versions"

if "%~1"=="-u"    ( set "ENDPOINT=%~2" )
if "%~1"=="--url" ( set "ENDPOINT=%~2" )

where curl >nul 2>&1
if errorlevel 1 (
    echo curl not found on PATH; install curl to use this script 1>&2
    exit /b 2
)

for /f "delims=" %%R in ('curl -fsS --max-time 5 "!ENDPOINT!" 2^>nul') do (
    set "RAW=%%R"
)
if "!RAW!"=="" (
    echo could not reach cortex-api !ENDPOINT! 1>&2
    exit /b 2
)

REM Extract `all_in_sync`.
set "SYNC="
for /f "tokens=2 delims=:" %%T in ('echo !RAW! ^| findstr /R /C:"\"all_in_sync\"[ ]*:"') do (
    set "SYNC_RAW=%%T"
)
if defined SYNC_RAW (
    set "SYNC=!SYNC_RAW: =!"
    set "SYNC=!SYNC:,=!"
    set "SYNC=!SYNC:}=!"
)

echo endpoint:    !ENDPOINT!
echo all_in_sync: !SYNC!
echo.
echo full report:
echo !RAW!

if /i "!SYNC!"=="true"  exit /b 0
if /i "!SYNC!"=="false" exit /b 1
exit /b 2

@echo off
REM Phase8a — pretty-print the cortex-api /v1/health aggregator and
REM exit on the worst observed state. Intended as a quick "is the
REM stack healthy?" probe for operators + CI smoke jobs.
REM
REM Usage:
REM   scripts\health.bat             default endpoint
REM   scripts\health.bat -u http://...   custom endpoint
REM
REM Exit codes (match cortex_health::HealthState::exit_code):
REM   0 — overall=ok
REM   1 — overall=degraded
REM   2 — overall=down
REM   3 — could not reach /v1/health at all
setlocal EnableDelayedExpansion

set "ENDPOINT=%CORTEX_API_URL%"
if "!ENDPOINT!"=="" set "ENDPOINT=http://127.0.0.1:17000"
set "ENDPOINT=!ENDPOINT!/v1/health"

if "%~1"=="-u"     ( set "ENDPOINT=%~2" )
if "%~1"=="--url"  ( set "ENDPOINT=%~2" )

REM curl ships in Windows 10+; fall back with a clear message if not.
where curl >nul 2>&1
if errorlevel 1 (
    echo curl not found on PATH; install curl to use scripts\health.bat 1>&2
    exit /b 3
)

REM Probe the aggregator. -f exits non-zero on 4xx/5xx; --max-time
REM caps the wait at 5s so a stalled aggregator never freezes the
REM script.
for /f "delims=" %%R in ('curl -fsS --max-time 5 "!ENDPOINT!" 2^>nul') do (
    set "RAW=%%R"
)

if "!RAW!"=="" (
    echo could not reach cortex-api !ENDPOINT! 1>&2
    exit /b 3
)

REM Extract the `overall` field — relies on the stable
REM cortex_health::HealthReport JSON shape.
set "OVERALL="
for /f "tokens=2 delims=:" %%T in ('echo !RAW! ^| findstr /R /C:"\"overall\"[ ]*:"') do (
    set "OVERALL_RAW=%%T"
)
if defined OVERALL_RAW (
    REM Strip surrounding whitespace + quotes + the trailing comma/brace.
    set "OVERALL=!OVERALL_RAW: =!"
    set "OVERALL=!OVERALL:"=!"
    set "OVERALL=!OVERALL:,=!"
    set "OVERALL=!OVERALL:}=!"
)

if "!OVERALL!"=="" (
    echo could not parse overall state from response 1>&2
    echo !RAW! 1>&2
    exit /b 3
)

echo endpoint:  !ENDPOINT!
echo overall:   !OVERALL!
echo.
echo full report:
echo !RAW!

if /i "!OVERALL!"=="ok"       exit /b 0
if /i "!OVERALL!"=="degraded" exit /b 1
if /i "!OVERALL!"=="down"     exit /b 2
echo unknown overall state: !OVERALL! 1>&2
exit /b 3

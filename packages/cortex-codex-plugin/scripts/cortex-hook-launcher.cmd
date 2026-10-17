@echo off
rem Cortex Codex plugin hook launcher for Windows. Usage: cortex-hook-launcher.cmd <Event>
rem
rem Codex runs commandWindows as %COMSPEC% /C "<command>". This script hands one
rem Codex lifecycle event to the installed cortex-hook.exe (PATH, then
rem %USERPROFILE%\.cortex\bin) with --client codex. cortex-hook reads the hook
rem JSON from the inherited stdin and owns bounded I/O and on-demand runtime start.
rem Without a binary, or for an unknown event, it drains stdin, prints an empty
rem JSON reply and exits 0. It prints nothing besides cortex-hook's reply.
setlocal EnableExtensions DisableDelayedExpansion
set "EVENT=%~1"
set "FLAG="
if "%EVENT%"=="SessionStart" goto :resolve
if "%EVENT%"=="UserPromptSubmit" goto :resolve
set "FLAG=--fire-forget"
if "%EVENT%"=="PostToolUse" goto :resolve
if "%EVENT%"=="Stop" goto :resolve
if "%EVENT%"=="SubagentStop" goto :resolve
goto :fail_open

:resolve
rem $PATH: limits the search to PATH, never the session's working directory.
set "BIN="
for /f "delims=" %%P in ('where $PATH:cortex-hook.exe 2^>nul') do if not defined BIN set "BIN=%%P"
if not defined BIN if exist "%USERPROFILE%\.cortex\bin\cortex-hook.exe" set "BIN=%USERPROFILE%\.cortex\bin\cortex-hook.exe"
if not defined BIN goto :fail_open
rem --client codex needs cortex-hook from phase36 on (older builds reject it and
rem answer {}). The origin repeats the identity for adapter.hook_origin readers.
set "CORTEX_HOOK_ORIGIN=codex-plugin"
"%BIN%" %EVENT% %FLAG% --client codex
exit /b %ERRORLEVEL%

:fail_open
more >nul 2>nul
echo {}
exit /b 0

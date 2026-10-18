@echo off
rem Standalone Cortex hook transport; stdout contains only its JSON reply.
setlocal EnableExtensions DisableDelayedExpansion
set "EVENT=%~1"
if "%EVENT%"=="SessionStart" goto :resolve
if "%EVENT%"=="UserPromptSubmit" goto :resolve
if "%EVENT%"=="PostToolUse" goto :resolve
if "%EVENT%"=="Stop" goto :resolve
if "%EVENT%"=="SubagentStop" goto :resolve
goto :fail_open
:resolve
set "BIN="
for /f "delims=" %%P in ('where $PATH:cortex.exe 2^>nul') do if not defined BIN set "BIN=%%P"
if not defined BIN if defined CORTEX_HOME if exist "%CORTEX_HOME%\bin\cortex.exe" set "BIN=%CORTEX_HOME%\bin\cortex.exe"
if not defined BIN if defined USERPROFILE if exist "%USERPROFILE%\.cortex\bin\cortex.exe" set "BIN=%USERPROFILE%\.cortex\bin\cortex.exe"
if not defined BIN goto :fail_open
set "REPLY_DIR=%TEMP%\cortex-hook-%RANDOM%-%RANDOM%"
mkdir "%REPLY_DIR%" >nul 2>nul
if errorlevel 1 goto :fail_open
"%BIN%" hook --client codex --event "%EVENT%" >"%REPLY_DIR%\reply.json" 2>nul
if errorlevel 1 goto :failed_reply
for %%F in ("%REPLY_DIR%\reply.json") do if %%~zF==0 goto :failed_reply
type "%REPLY_DIR%\reply.json"
goto :cleanup
:failed_reply
echo {}
:cleanup
del /q "%REPLY_DIR%\reply.json" >nul 2>nul
rmdir "%REPLY_DIR%" >nul 2>nul
exit /b 0
:fail_open
more >nul 2>nul
echo {}
exit /b 0

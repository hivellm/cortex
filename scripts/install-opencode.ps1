# Phase11w §8.2 — Cortex OpenCode integration installer (PowerShell).
#
# PowerShell mirror of `install-opencode.sh`. Verifies `opencode` is on
# PATH, ensures the daemon HTTP listener binding is configured, and
# prints next-step instructions.

param(
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$RepoRoot = if ($env:CORTEX_REPO_ROOT) { $env:CORTEX_REPO_ROOT } else { (git rev-parse --show-toplevel 2>$null) }
if (-not $RepoRoot) { $RepoRoot = (Get-Location).Path }
$DefaultBind = "127.0.0.1:17004"

function Write-Err($msg) { Write-Host "error: $msg" -ForegroundColor Red }
function Write-Info($msg) { Write-Host "ok: $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "warn: $msg" -ForegroundColor Yellow }

# 1. Verify required binaries.
$opencode = Get-Command opencode -ErrorAction SilentlyContinue
if (-not $opencode) {
    Write-Err "opencode CLI not found on PATH. Install from https://opencode.ai/docs/install"
    exit 1
}
Write-Info "opencode CLI: $($opencode.Source)"

$adapter = Get-Command cortex-adapter-claude -ErrorAction SilentlyContinue
if (-not $adapter) {
    Write-Warn "cortex-adapter-claude binary not on PATH; build it with 'cargo build --release -p cortex-adapter-claude-code'"
}

# 2. Confirm the project config exists.
$configPath = Join-Path $RepoRoot "opencode.json"
if (-not (Test-Path $configPath)) {
    Write-Err "opencode.json missing at $RepoRoot. Re-run 'rulebook task apply phase11w_opencode-adapter'."
    exit 1
}
Write-Info "opencode.json present"

# 3. Confirm the agents + commands directories ship.
foreach ($d in @(".opencode/agents", ".opencode/commands")) {
    $p = Join-Path $RepoRoot $d
    if (-not (Test-Path $p)) {
        Write-Err "$d missing under $RepoRoot"
        exit 1
    }
}
Write-Info ".opencode/{agents,commands} present"

# 4. Resolve the HTTP bind the plugin will POST to.
$httpBind = if ($env:CORTEX_ADAPTER_HTTP_BIND) { $env:CORTEX_ADAPTER_HTTP_BIND } else { $DefaultBind }
Write-Info "daemon http bind: $httpBind"

Write-Host @"

Next steps:
  1. Start the Cortex daemon with the HTTP transport:
       `$env:CORTEX_ADAPTER_HTTP_BIND = "$httpBind"
       cortex-adapter-claude daemon
  2. From this repo, launch OpenCode:
       opencode
  3. Inside the OpenCode TUI, verify the cortex tools are listed:
       /mcp list
  4. Smoke-test the plugin: submit a prompt and confirm the assistant
     receives a pre-thinking bundle (look for the "## active work" or
     "## consolidations" sections in the assistant's reasoning context).

Uninstall: scripts/install-opencode.ps1 -Uninstall
"@

if ($Uninstall) {
    Write-Warn "uninstall flag set; this script does not delete .opencode/ or opencode.json on its own"
    Write-Warn "remove them manually: Remove-Item -Recurse .opencode, opencode.json"
}

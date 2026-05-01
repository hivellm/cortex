# Install a Windows scheduled task that starts cortex-adapter-claude
# at user logon and restarts it if it dies. Run this once per host
# (right-click → "Run with PowerShell" or `pwsh -ExecutionPolicy Bypass -File scripts\install-adapter-autostart.ps1`).
#
# Why not docker: the adapter listens on a Windows named pipe
# (`\\.\pipe\cortex-adapter-claude`) that Claude Code's hooks write
# to. Docker Desktop cannot bridge a Windows named pipe into a
# Linux container, so the adapter has to live on the host.

$ErrorActionPreference = "Stop"
$taskName = "CortexAdapterClaude"

$exe = "$env:USERPROFILE\.cargo\bin\cortex-adapter-claude.exe"
if (-not (Test-Path $exe)) {
    Write-Error "Binary not found: $exe`nInstall it with: cargo install --path crates/cortex-adapter-claude-code --bin cortex-adapter-claude"
    exit 1
}

# Drop any previous registration first so re-running this script is
# idempotent.
schtasks /Query /TN $taskName 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "Removing existing task '$taskName'..."
    schtasks /Delete /TN $taskName /F | Out-Null
}

# Register: trigger on every user logon, run hidden in the user's
# context, no admin escalation required, no time limit, restart on
# failure (3 attempts, 1 minute apart).
$action = New-ScheduledTaskAction -Execute $exe
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERDOMAIN\$env:USERNAME
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit ([TimeSpan]::Zero)

Register-ScheduledTask `
    -TaskName $taskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Description "Cortex — Claude Code adapter daemon (host-side, listens on named pipe)." `
    -Force | Out-Null

Write-Host "Registered scheduled task '$taskName'."
Write-Host "Starting it now so the adapter is up immediately..."
Start-ScheduledTask -TaskName $taskName

Start-Sleep -Seconds 2
$proc = Get-Process -Name "cortex-adapter-claude" -ErrorAction SilentlyContinue
if ($proc) {
    Write-Host "Adapter running (pid $($proc.Id)). It will auto-start at every logon and restart on failure."
} else {
    Write-Warning "Task started but cortex-adapter-claude.exe is not visible yet — check Task Scheduler GUI or logs at ~/.cortex/hook-errors.log."
}

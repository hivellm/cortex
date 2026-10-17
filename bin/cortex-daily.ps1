#requires -Version 5.1
<#
.SYNOPSIS
  Start, inspect, or stop only the native Cortex daily-memory processes.
#>
[CmdletBinding()]
param(
    [ValidateSet('start', 'status', 'stop')]
    [string]$Action = 'status',
    [string]$BinaryDirectory,
    [string]$StateDirectory = (Join-Path $env:USERPROFILE '.cortex/daily-runtime'),
    [string]$ConfigFile,
    [ValidateRange(1024, 65535)]
    [int]$ApiPort = 17000,
    [ValidateRange(1024, 65535)]
    [int]$AdapterAdminPort = 17011,
    [string]$AdapterBinding = '\\.\pipe\cortex-adapter-claude',
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$invocationDirectory = (Get-Location).Path
if ([string]::IsNullOrEmpty($BinaryDirectory)) {
    $BinaryDirectory = Join-Path (Split-Path -Parent $PSScriptRoot) 'target/release'
}

function Get-ProcessIdentity([int]$ProcessId) {
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    try {
        $imagePath = $process.Path
        if ([string]::IsNullOrEmpty($imagePath)) {
            # MainModule/Path can be unavailable after the launching shell exits.
            # Query only the image path using Windows' limited-information right.
            if (-not ('CortexDailyLauncher.ProcessImage' -as [type])) {
                Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
namespace CortexDailyLauncher {
    public static class ProcessImage {
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool QueryFullProcessImageName(IntPtr process, uint flags, StringBuilder image, ref int size);
        [DllImport("kernel32.dll")]
        private static extern bool CloseHandle(IntPtr handle);
        public static string Read(int pid) {
            IntPtr handle = OpenProcess(0x1000, false, pid);
            if (handle == IntPtr.Zero) return null;
            try {
                StringBuilder image = new StringBuilder(32768);
                int size = image.Capacity;
                return QueryFullProcessImageName(handle, 0, image, ref size) ? image.ToString() : null;
            } finally { CloseHandle(handle); }
        }
    }
}
'@
            }
            $imagePath = [CortexDailyLauncher.ProcessImage]::Read($ProcessId)
        }
        if ([string]::IsNullOrEmpty($imagePath)) { return $null }
        return [pscustomobject]@{
            pid = $process.Id
            path = [IO.Path]::GetFullPath($imagePath)
            startTimeTicks = $process.StartTime.ToUniversalTime().Ticks.ToString()
        }
    } catch {
        return $null
    }
}

function Test-OwnedIdentity($Expected, $Actual) {
    return $null -ne $Actual -and
        [int]$Expected.pid -eq [int]$Actual.pid -and
        [StringComparer]::OrdinalIgnoreCase.Equals([string]$Expected.path, [string]$Actual.path) -and
        [string]$Expected.startTimeTicks -ceq [string]$Actual.startTimeTicks
}

function Get-OwnedState($Record) {
    $actual = Get-ProcessIdentity ([int]$Record.pid)
    if (Test-OwnedIdentity $Record $actual) { return 'running' }
    if ($null -eq (Get-Process -Id ([int]$Record.pid) -ErrorAction SilentlyContinue)) { return 'stopped' }
    return 'identity_mismatch_or_unreadable'
}

function Stop-OwnedProcess($Record) {
    if ((Get-OwnedState $Record) -eq 'running') {
        Stop-Process -Id ([int]$Record.pid) -ErrorAction Stop
    }
}

function Stop-RecordedServices($Services) {
    foreach ($name in @('adapter', 'api')) {
        foreach ($record in @($Services | Where-Object { $_.name -eq $name })) {
            Stop-OwnedProcess $record
        }
    }
}

function Assert-PortFree([int]$Port) {
    $probe = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $Port)
    try { $probe.Start() }
    catch { throw "Loopback port $Port is occupied or unavailable. No existing service was adopted or stopped." }
    finally { $probe.Stop() }
}

function Test-OwnedListener([int]$Port, [int]$ProcessId) {
    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop |
        Where-Object { $_.LocalAddress -eq '127.0.0.1' -or $_.LocalAddress -eq '0.0.0.0' })
    return $listeners.Count -gt 0 -and
        @($listeners | Where-Object { $_.OwningProcess -ne $ProcessId }).Count -eq 0
}

function Get-HealthUrls([string]$ApiUrl, [int]$AdminPort) {
    return @("$ApiUrl/healthz", "http://127.0.0.1:$AdminPort/healthz")
}

function Quote-Argument([string]$Value) {
    if ($Value.Contains('"') -or $Value.EndsWith('\')) {
        throw 'Unsupported quote or trailing backslash in process argument.'
    }
    return '"' + $Value + '"'
}

function Save-State($State, [string]$Path) {
    $State | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding UTF8
}

if ($SelfTest) {
    $identity = Get-ProcessIdentity $PID
    if ($null -eq $identity) { throw 'Cannot inspect current process identity.' }
    if (-not (Test-OwnedIdentity $identity (Get-ProcessIdentity $PID))) { throw 'Owned process was rejected.' }
    foreach ($field in @('pid', 'path', 'startTimeTicks')) {
        $foreign = [pscustomobject]@{
            pid = $identity.pid
            path = $identity.path
            startTimeTicks = $identity.startTimeTicks
        }
        switch ($field) {
            'pid' { $foreign.pid = 0 }
            'path' { $foreign.path = 'C:\unrelated\cortex-api.exe' }
            'startTimeTicks' { $foreign.startTimeTicks = '0' }
        }
        if (Test-OwnedIdentity $identity $foreign) { throw "Unsafe identity match: $field" }
    }
    if (Test-OwnedIdentity $identity $null) { throw 'Missing process matched.' }
    $healthUrls = @(Get-HealthUrls 'http://127.0.0.1:19000' 19011)
    if ($healthUrls.Count -ne 2 -or $healthUrls[0] -cne 'http://127.0.0.1:19000/healthz' -or
        $healthUrls[1] -cne 'http://127.0.0.1:19011/healthz') { throw 'Separate health URLs were not preserved.' }
    if ((Quote-Argument 'C:\path with spaces\adapter.toml') -cne '"C:\path with spaces\adapter.toml"') {
        throw 'Argument quoting failed.'
    }
    $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $conflictDetected = $false
    try {
        try { Assert-PortFree $listener.LocalEndpoint.Port }
        catch { $conflictDetected = $true }
    } finally { $listener.Stop() }
    if (-not $conflictDetected) { throw 'Occupied port was accepted.' }
    Write-Output 'Daily launcher self-test passed: process ownership, PID reuse, argument quoting, port conflict.'
    return
}

if ($ApiPort -eq $AdapterAdminPort) { throw 'API and adapter admin ports must differ.' }
if (-not $AdapterBinding.StartsWith('\\.\pipe\')) { throw 'Windows adapter binding must be a named pipe.' }
$StateDirectory = [IO.Path]::GetFullPath($StateDirectory)
$statePath = Join-Path $StateDirectory 'state.json'
if ($Action -ne 'start' -and -not (Test-Path -LiteralPath $statePath)) {
    Write-Output 'No owned daily runtime is recorded.'
    return
}
[IO.Directory]::CreateDirectory($StateDirectory) | Out-Null
$lockPath = Join-Path $StateDirectory 'launcher.lock'
try { $lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
catch { throw 'Another daily launcher operation is active, or its state directory is inaccessible.' }
try {
    $state = $null
    if (Test-Path -LiteralPath $statePath) {
        $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        if ($state.version -ne 1) { throw 'Unsupported daily runtime state version.' }
    }
    if ($Action -eq 'status') {
        foreach ($record in $state.services) {
            [pscustomobject]@{ service = $record.name; state = Get-OwnedState $record; pid = $record.pid; path = $record.path }
        }
        return
    }
    if ($Action -eq 'stop') {
        # Adapter first; never stop processes by name, port, or an unchecked PID.
        Stop-RecordedServices $state.services
        $state.phase = 'stopped'
        Save-State $state $statePath
        foreach ($record in $state.services) {
            [pscustomobject]@{ service = $record.name; state = Get-OwnedState $record; pid = $record.pid }
        }
        return
    }
    if ($null -ne $state) {
        foreach ($record in $state.services) {
            if ((Get-OwnedState $record) -eq 'running') { throw 'An owned daily process is already running. Inspect status or stop it first.' }
        }
    }
    Assert-PortFree $ApiPort
    Assert-PortFree $AdapterAdminPort
    # Fail on an existing pipe as well, including an unrelated adapter instance.
    $pipeName = $AdapterBinding.Substring('\\.\pipe\'.Length)
    if (Test-Path -LiteralPath "\\.\pipe\$pipeName") { throw 'Adapter named pipe already exists. No existing adapter was adopted or stopped.' }
    $apiBinary = [IO.Path]::GetFullPath((Join-Path $BinaryDirectory 'cortex-api.exe'))
    $adapterBinary = [IO.Path]::GetFullPath((Join-Path $BinaryDirectory 'cortex-adapter-claude.exe'))
    foreach ($binary in @($apiBinary, $adapterBinary)) {
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Required binary missing: $binary" }
    }
    if ([string]::IsNullOrEmpty($ConfigFile)) {
        $inheritedConfig = [Environment]::GetEnvironmentVariable('CORTEX_CONFIG_FILE', 'Process')
        $localConfig = Join-Path $invocationDirectory 'cortex.toml'
        if (-not [string]::IsNullOrEmpty($inheritedConfig)) {
            $ConfigFile = $inheritedConfig
        } elseif (Test-Path -LiteralPath $localConfig -PathType Leaf) {
            $ConfigFile = $localConfig
        }
    }
    if ([string]::IsNullOrEmpty($ConfigFile)) {
        $ConfigFile = Join-Path $StateDirectory 'runtime.toml'
        [IO.File]::WriteAllText($ConfigFile, '# Generated daily runtime configuration; profile preserves existing Cortex data.', (New-Object Text.UTF8Encoding($false)))
    } else {
        $ConfigFile = [IO.Path]::GetFullPath($ConfigFile)
        if (-not (Test-Path -LiteralPath $ConfigFile -PathType Leaf)) { throw 'Specified Cortex config file does not exist.' }
    }
    $adapterConfig = Join-Path $StateDirectory 'adapter.daily.toml'
    $adapterToml = @"
# Generated by cortex-daily.ps1; archival publishing is disabled by daily profile.
[adapter]
api_endpoint = "http://127.0.0.1:$ApiPort"
"@
    [IO.File]::WriteAllText($adapterConfig, $adapterToml, (New-Object Text.UTF8Encoding($false)))
    $childEnvironment = @{
        CORTEX_RUNTIME_PROFILE = 'daily'
        CORTEX_CONFIG_FILE = $ConfigFile
        CORTEX_API_URL = "http://127.0.0.1:$ApiPort"
        CORTEX_ADAPTER_ADMIN_PORT = $AdapterAdminPort.ToString()
    }
    $previousEnvironment = @{}
    $runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff')
    $state = [pscustomobject]@{ version = 1; phase = 'starting'; apiUrl = "http://127.0.0.1:$ApiPort"; logPrefix = $runId; services = @() }
    try {
        foreach ($name in $childEnvironment.Keys) {
            $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            [Environment]::SetEnvironmentVariable($name, $childEnvironment[$name], 'Process')
        }
        $definitions = @(
            @{ name = 'api'; binary = $apiBinary; arguments = @('--bind', "127.0.0.1:$ApiPort") },
            @{ name = 'adapter'; binary = $adapterBinary; arguments = @('--config', (Quote-Argument $adapterConfig), '--bind', (Quote-Argument $AdapterBinding), 'daemon') }
        )
        foreach ($definition in $definitions) {
            $process = Start-Process -FilePath $definition.binary -ArgumentList $definition.arguments -WindowStyle Hidden -PassThru -WorkingDirectory $invocationDirectory -RedirectStandardOutput (Join-Path $StateDirectory ($runId + '.' + $definition.name + '.stdout.log')) -RedirectStandardError (Join-Path $StateDirectory ($runId + '.' + $definition.name + '.stderr.log'))
            $identity = Get-ProcessIdentity $process.Id
            if ($null -eq $identity -or -not [StringComparer]::OrdinalIgnoreCase.Equals($identity.path, $definition.binary)) {
                # Start-Process retains this exact new child's Windows handle.
                # Clean up that handle, never a newly looked-up unchecked PID.
                if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(3000) }
                throw "Cannot verify newly launched $($definition.name) process. Inspect runtime logs."
            }
            $state.services += [pscustomobject]@{ name = $definition.name; pid = $identity.pid; path = $identity.path; startTimeTicks = $identity.startTimeTicks }
            Save-State $state $statePath
        }
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        $lastReadinessError = 'No health response yet.'
        do {
            foreach ($record in $state.services) {
                if ((Get-OwnedState $record) -ne 'running') { throw "Daily $($record.name) exited before readiness. Inspect runtime logs." }
            }
            $ready = $true
            foreach ($url in (Get-HealthUrls $state.apiUrl $AdapterAdminPort)) {
                try {
                    $response = Invoke-RestMethod -Uri $url -TimeoutSec 1
                    if ($response.extras.runtime_profile -cne 'daily') {
                        throw 'Health endpoint did not confirm daily runtime profile.'
                    }
                }
                catch { $ready = $false; $lastReadinessError = $_.Exception.Message }
            }
            if ($ready) {
                try {
                    $ready = (Test-OwnedListener $ApiPort $state.services[0].pid) -and
                        (Test-OwnedListener $AdapterAdminPort $state.services[1].pid)
                    if (-not $ready) { $lastReadinessError = 'Listening port is not owned by expected child PID.' }
                } catch { $ready = $false; $lastReadinessError = $_.Exception.Message }
            }
            if (-not $ready) { Start-Sleep -Milliseconds 200 }
        } while (-not $ready -and [DateTime]::UtcNow -lt $deadline)
        if (-not $ready) { throw "Daily runtime readiness timed out: $lastReadinessError Inspect runtime logs." }
        $state.phase = 'running'
        Save-State $state $statePath
        Write-Output "Daily runtime ready at $($state.apiUrl). MCP remains client-owned."
    } catch {
        Stop-RecordedServices $state.services
        $state.phase = 'failed'
        Save-State $state $statePath
        throw
    } finally {
        foreach ($name in $previousEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
        }
    }
} finally { $lock.Dispose() }

# cortex-session-start — SessionStart shim (Windows). Spec 10.
# Never break the session: any failure prints `{}` and exits 0.
$ErrorActionPreference = "SilentlyContinue"
# Disabled inside Cortex sub-workers (CORTEX_ADAPTER_DISABLE=1).
if ($env:CORTEX_ADAPTER_DISABLE -eq "1") { Write-Output "{}"; exit 0 }
$pipeName = if ($env:CORTEX_ADAPTER_PIPE) { $env:CORTEX_ADAPTER_PIPE } else { "cortex-adapter-claude" }
$input_text = [Console]::In.ReadToEnd()
if ($input_text) { $input_text = $input_text.Trim() }
if (-not $input_text) { $input_text = "{}" }
$session = if ($env:CLAUDE_SESSION_ID) { $env:CLAUDE_SESSION_ID } else { "" }
$logDir = Join-Path $HOME ".cortex"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }
$logPath = Join-Path $logDir "hook-invocations.log"
$ts = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss.fffZ")
$payloadSession = ""
try { $payloadSession = (ConvertFrom-Json $input_text -ErrorAction SilentlyContinue).session_id } catch {}
Add-Content -Path $logPath -Value "$ts SessionStart env_sid=$session payload_sid=$payloadSession pid=$PID" -ErrorAction SilentlyContinue
$frame = "{`"hook`":`"SessionStart`",`"session_id`":`"$session`",`"cwd`":`"$($PWD.Path -replace '\\','\\\\')`",`"payload`":$input_text}"
$errLog = Join-Path $logDir "hook-errors.log"
try {
    $client = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
    $client.Connect(3000)
    $writer = New-Object System.IO.StreamWriter($client)
    $writer.WriteLine($frame); $writer.Flush()
    $reader = New-Object System.IO.StreamReader($client)
    $response = $reader.ReadLine()
    $client.Dispose()
    Add-Content -Path $logPath -Value "$ts SessionStart  -> ok" -ErrorAction SilentlyContinue
    if ($response) { Write-Output $response } else { Write-Output "{}" }
} catch {
    $errMsg = $_.ToString()
    $category = "other"
    if ($errMsg -match "Pipe is broken|Pipe has been ended") { $category = "pipe_broken" }
    elseif ($errMsg -match "TimeoutException|Connect.*timed out|Wait.*time") { $category = "connect_timeout" }
    elseif ($errMsg -match "Access.*denied|Unauthorized") { $category = "access_denied" }
    Add-Content -Path $logPath -Value "$ts SessionStart  -> err[$category]: $errMsg" -ErrorAction SilentlyContinue
    Add-Content -Path $errLog -Value "$ts SessionStart $category $errMsg" -ErrorAction SilentlyContinue
    Write-Output "{}"
}
exit 0

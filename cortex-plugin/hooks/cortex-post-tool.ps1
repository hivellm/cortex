# cortex-post-tool — PostToolUse shim (Windows). Spec 10.
$ErrorActionPreference = "SilentlyContinue"
$pipeName = if ($env:CORTEX_ADAPTER_PIPE) { $env:CORTEX_ADAPTER_PIPE } else { "cortex-adapter-claude" }
$input_text = [Console]::In.ReadToEnd()
if ($input_text) { $input_text = $input_text.Trim() }
if (-not $input_text) { $input_text = "{}" }
$session = if ($env:CLAUDE_SESSION_ID) { $env:CLAUDE_SESSION_ID } else { "" }
# Forensic log — every invocation lands here so we can prove claude-code
# is firing the hook even when the pipe round-trip silently fails. Path
# is fixed under HOME so it survives across sessions.
$logDir = Join-Path $HOME ".cortex"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }
$logPath = Join-Path $logDir "hook-invocations.log"
$ts = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss.fffZ")
# Pull payload session_id since CLAUDE_SESSION_ID env is empty in
# claude-code's hook context — that's the same fallback the adapter
# dispatcher uses. We only sniff it for logging; the pipe frame still
# forwards the raw payload as-is.
$payloadSession = ""
try { $payloadSession = (ConvertFrom-Json $input_text -ErrorAction SilentlyContinue).session_id } catch {}
$logLine = "$ts PostToolUse env_sid=$session payload_sid=$payloadSession pid=$PID"
Add-Content -Path $logPath -Value $logLine -ErrorAction SilentlyContinue
$frame = "{`"hook`":`"PostToolUse`",`"session_id`":`"$session`",`"cwd`":`"$($PWD.Path -replace '\\','\\\\')`",`"payload`":$input_text}"
try {
    $client = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
    # Bump from 1s → 3s. Hook timeout is 5s (hooks.json); leaving 2s
    # headroom for write+read+exit. Single-instance pipe contention
    # under concurrent sessions was the suspected silent-fail cause.
    $client.Connect(3000)
    $writer = New-Object System.IO.StreamWriter($client)
    $writer.WriteLine($frame); $writer.Flush()
    $reader = New-Object System.IO.StreamReader($client)
    $response = $reader.ReadLine()
    $client.Dispose()
    Add-Content -Path $logPath -Value "$ts PostToolUse  -> ok" -ErrorAction SilentlyContinue
    if ($response) { Write-Output $response } else { Write-Output "{}" }
} catch {
    Add-Content -Path $logPath -Value "$ts PostToolUse  -> err: $_" -ErrorAction SilentlyContinue
    Write-Output "{}"
}
exit 0

# cortex-stop — Stop shim (Windows). Spec 10.
$ErrorActionPreference = "SilentlyContinue"
$pipeName = if ($env:CORTEX_ADAPTER_PIPE) { $env:CORTEX_ADAPTER_PIPE } else { "cortex-adapter-claude" }
$input_text = [Console]::In.ReadToEnd()
if (-not $input_text) { $input_text = "{}" }
$session = if ($env:CLAUDE_SESSION_ID) { $env:CLAUDE_SESSION_ID } else { "" }
$frame = "{`"hook`":`"Stop`",`"session_id`":`"$session`",`"cwd`":`"$($PWD.Path -replace '\\','\\\\')`",`"payload`":$input_text}"
try {
    $client = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
    $client.Connect(1000)
    $writer = New-Object System.IO.StreamWriter($client)
    $writer.WriteLine($frame); $writer.Flush()
    $reader = New-Object System.IO.StreamReader($client)
    $response = $reader.ReadLine()
    $client.Dispose()
    if ($response) { Write-Output $response } else { Write-Output "{}" }
} catch { Write-Output "{}" }
exit 0

param(
    [int]$AgeDays = 2,
    [int]$Batch = 1000,
    [int]$Parallel = 30
)
$token = 'I-UNDERSTAND-FORGET-IS-IRREVERSIBLE'
$api = 'http://127.0.0.1:17000'
$totalGrand = 0
$errorsGrand = 0
$start = Get-Date
$round = 0
while ($true) {
    $round++
    $cutoff = (Get-Date).AddDays(-$AgeDays).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $enc = $cutoff -replace ':', '%3A'
    try {
        $rows = Invoke-RestMethod -Uri "$api/v1/admin/list-events?kind=tool_call&before=$enc&limit=$Batch"
    } catch {
        Write-Host "ENUM_ERR $($_.Exception.Message)"
        break
    }
    if (-not $rows -or $rows.Count -eq 0) {
        Write-Host "DONE total=$totalGrand errors=$errorsGrand elapsed=$((Get-Date) - $start)"
        break
    }
    $batchStart = Get-Date
    $results = $rows | ForEach-Object -Parallel {
        $body = @{ event_id = $_.event_id; confirmation_token = $using:token; dry_run = $false } | ConvertTo-Json -Compress
        try {
            $null = Invoke-RestMethod -Uri "$($using:api)/v1/admin/forget" -Method POST -Body $body -ContentType 'application/json' -TimeoutSec 60
            'ok'
        } catch {
            'err'
        }
    } -ThrottleLimit $Parallel
    $okCount = ($results | Where-Object { $_ -eq 'ok' }).Count
    $errCount = ($results | Where-Object { $_ -eq 'err' }).Count
    $totalGrand += $okCount
    $errorsGrand += $errCount
    $batchElapsed = (Get-Date) - $batchStart
    Write-Host "ROUND $round ok=$okCount err=$errCount batch_elapsed=$batchElapsed total=$totalGrand errors=$errorsGrand grand_elapsed=$((Get-Date) - $start)"
}

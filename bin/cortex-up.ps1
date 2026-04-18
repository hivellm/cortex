#requires -Version 5.1
<#
.SYNOPSIS
  Bring the Cortex local stack up (Windows).
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSCommandPath | Split-Path -Parent
Set-Location $repoRoot

if (-not (Test-Path .env)) {
    Copy-Item .env.example .env
    Write-Host "note: created .env from .env.example — edit secrets before using in anger."
}

Write-Host "[1/3] docker compose up -d"
docker compose up -d --wait

Write-Host "[2/3] health:"
docker compose ps

Write-Host "[3/3] first-time init (idempotent)"
bash .\bin\cortex-init.sh

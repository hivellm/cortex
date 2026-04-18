#requires -Version 5.1
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSCommandPath | Split-Path -Parent
Set-Location $repoRoot
docker compose down @args

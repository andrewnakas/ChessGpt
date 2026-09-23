# Runs chessgpt for https://chessgpt.com behind a Cloudflare Tunnel on this PC.
# Usage (PowerShell):  .\deploy\start-hosted.ps1
# Optional: set $env:ANTHROPIC_API_KEY first to turn on the site's own coach.

$env:CHESSGPT_MODE = "hosted"
$env:CHESSGPT_PUBLIC_URL = "https://chessgpt.com"
$env:CHESSGPT_BIND = "127.0.0.1:8080"
if (-not $env:CHESSGPT_LLM_DAILY_TOKENS) { $env:CHESSGPT_LLM_DAILY_TOKENS = "3000000" }

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
& "$root\target\release\chessgpt.exe"

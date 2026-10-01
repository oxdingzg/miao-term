# windows-verify.ps1 — build, test and smoke-test mtty on Windows.
#
# Run it on a Windows dev/verification machine (see docs/WINDOWS-DEV.md):
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows-verify.ps1
#
# It builds the app and CLI, runs the engine/MTP tests, then starts the app and
# drives it over MTP (ping / file write / file read / view) before stopping it.
# Exits non-zero if any step fails.

[CmdletBinding()]
param(
    [string]$Source = "$env:USERPROFILE\miao-term",
    [string]$Toolchain = "stable-x86_64-pc-windows-msvc",
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$failures = @()

function Step([string]$name, [scriptblock]$body) {
    Write-Host "== $name" -ForegroundColor Cyan
    try {
        & $body
        if ($LASTEXITCODE -ne 0 -and $null -ne $LASTEXITCODE) {
            throw "exit code $LASTEXITCODE"
        }
        Write-Host "   ok" -ForegroundColor Green
    } catch {
        Write-Host "   FAILED: $_" -ForegroundColor Red
        $script:failures += $name
    }
}

if (-not (Test-Path $cargo)) { throw "cargo not found at $cargo" }
if (-not (Test-Path $Source)) { throw "source not found at $Source" }

$toolchainFile = Join-Path $Source "rust-toolchain.toml"
if ((Test-Path $toolchainFile) -and (Select-String -Quiet -Path $toolchainFile -Pattern "windows-gnu")) {
    Write-Warning "rust-toolchain.toml pins a -gnu toolchain; rustc's dlltool step fails on this host. Use $Toolchain."
}

Push-Location $Source
try {
    Step "build mtty" { & $cargo build -p mtty-app --color never }
    Step "build mtty-cli" { & $cargo build -p mtty-cli --color never }
    if (-not $SkipTests) {
        Step "engine + mtp tests" { & $cargo test -p miao-term-core -p miao-term-mtp --color never }
    }

    $exe = Join-Path $Source "target\debug\mtty.exe"
    $cli = Join-Path $Source "target\debug\mtty-cli.exe"
    $tmpFile = Join-Path $env:TEMP "mtty-smoke.txt"
    $app = $null
    try {
        Step "start app" { $script:app = Start-Process $exe -PassThru -WindowStyle Minimized }
        Start-Sleep -Seconds 6
        Step "mtp ping" { & $cli ping }
        Step "mtp file write" { & $cli file write --path $tmpFile --data "hello-from-mtp" }
        Step "mtp file read" { & $cli file read --path $tmpFile }
        Step "mtp view" { & $cli view $tmpFile }
    } finally {
        if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
        Remove-Item $tmpFile -ErrorAction SilentlyContinue
    }
} finally {
    Pop-Location
}

if ($failures.Count -gt 0) {
    Write-Host ""
    Write-Host ("FAILED: " + ($failures -join ", ")) -ForegroundColor Red
    exit 1
}
Write-Host ""
Write-Host "all steps passed" -ForegroundColor Green

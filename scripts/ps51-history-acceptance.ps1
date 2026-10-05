# Verify the mtty PowerShell 5.1 history record on a real Windows desktop
#
# Run this from a normal, logged-on Windows desktop session (not over ssh):
# it launches the mtty app in an isolated state, types two commands into a
# Windows PowerShell 5.1 pane through the shell integration, and reads the
# recorded history back over MTP. It prints PASS/FAIL and where the evidence is.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File ps51-history-acceptance.ps1 -App <path to mtty.exe>
#
# Get the app from the v0.1.6 release zip (mtty.exe, mtty-cli.exe, mtty-ptyhost.exe).

param(
    [Parameter(Mandatory = $true)] [string] $App,
    [string] $WorkDir = "$env:USERPROFILE\mtty-ps51-acceptance"
)

$ErrorActionPreference = 'Stop'
$app = (Resolve-Path $App).Path
$root = Split-Path $app -Parent
$cli = Join-Path $root 'mtty-cli.exe'
if (-not (Test-Path $cli)) { throw "mtty-cli.exe not found next to $app" }

# 0xdtee: this script must run in a logged-on session. Interactive = $true.
if (-not [Environment]::UserInteractive) {
    throw "Not an interactive session. Run this from the Windows desktop (RDP or the console), not over ssh."
}

# Windows MTP uses a fixed named pipe: XDG_RUNTIME_DIR does not isolate it.
# Refuse to touch an existing instance's panes or history.
if (Get-Process -Name mtty,miaotty -ErrorAction SilentlyContinue) {
    throw "Close all existing mtty windows before running this acceptance check. Windows MTP uses a shared named pipe."
}

# Use Windows PowerShell 5.1 as the pane shell, so MTTY's 5.1 history hook is exercised.
$ps51 = "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
if (-not (Test-Path $ps51)) { throw "Windows PowerShell 5.1 not found at $ps51" }
$ver = & $ps51 -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'

$workRoot = [IO.Path]::GetFullPath($WorkDir)
$base = Join-Path $workRoot 'run'
# PTY hosts use AF_UNIX even though the app's MTP endpoint is a named pipe.
# A long socket path makes the host fail after spawning its shell, then the
# app falls back to a local shell: history can pass despite a startup error.
$hostSocket = Join-Path $base ('runtime\mtty-hosts\' + ('0' * 32) + '.sock')
if ([Text.Encoding]::UTF8.GetByteCount($hostSocket) -ge 108) {
    throw "WorkDir is too long for the Windows PTY-host socket. Choose a shorter -WorkDir."
}
New-Item -ItemType Directory -Force -Path $workRoot | Out-Null
if ([IO.Path]::GetFullPath($base) -ne [IO.Path]::GetFullPath((Join-Path $workRoot 'run'))) {
    throw "Refusing to remove a run directory outside WorkDir."
}
Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
foreach ($d in 'home','config','data','runtime','tmp') {
    New-Item -ItemType Directory -Force -Path (Join-Path $base $d) | Out-Null
}

# Isolated state so this never touches the real config; Windows PowerShell 5.1 pane.
$env:HOME = Join-Path $base 'home'
$env:XDG_CONFIG_HOME = Join-Path $base 'config'
$env:XDG_DATA_HOME = Join-Path $base 'data'
$env:XDG_RUNTIME_DIR = Join-Path $base 'runtime'
$env:TMP = Join-Path $base 'tmp'
$env:TEMP = Join-Path $base 'tmp'
# Windows chooses COMSPEC, not SHELL (which is used on Unix).
$env:COMSPEC = $ps51
$env:SHELL = $ps51

$sock = Join-Path $base 'runtime\mtty.sock'
Write-Host "PowerShell 5.1: $ver"
Write-Host "Launching mtty in an isolated state..."

$p = Start-Process -FilePath $app -PassThru
try {
    # `cli` is a built-in alias for Clear-Item in Windows PowerShell 5.1.
    function Invoke-MttyCli {
        param([string[]] $Arguments)
        $text = & $cli --socket $sock --wait 2 @Arguments | Out-String
        if ($LASTEXITCODE -ne 0) { throw "mtty-cli failed ($LASTEXITCODE): $text" }
        $text
    }

    # Wait for the pane to exist.
    $ready = $false
    for ($i = 0; $i -lt 40; $i++) {
        $p.Refresh()
        if ($p.HasExited) { throw "mtty exited before exposing its pane; check for an existing instance." }
        try {
            $panes = Invoke-MttyCli -Arguments @('pane','list') | ConvertFrom-Json
            if (@($panes.panes | Where-Object { $_.id -eq 'pane0' }).Count -eq 1) {
                $ready = $true
                break
            }
        } catch {
            if ($i -eq 39) { throw }
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "mtty never exposed pane0" }

    # Type two commands into the 5.1 pane. `pane run` writes them as keystrokes,
    # so the PSReadLine / PSConsoleHostReadLine hook records each one.
    Invoke-MttyCli -Arguments @('pane','run','--pane','pane0','--data',"echo alpha-$([guid]::NewGuid().ToString('N').Substring(0,8))`r") | Out-Null
    Start-Sleep -Seconds 2
    $mark = "echo beta-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    Invoke-MttyCli -Arguments @('pane','run','--pane','pane0','--data',"$mark`r") | Out-Null
    Start-Sleep -Seconds 2

    $hist = Invoke-MttyCli -Arguments @('history','list','--pane','pane0')
    $hist | Out-File (Join-Path $base 'history.json') -Encoding utf8
    $out = Invoke-MttyCli -Arguments @('pane','output','--pane','pane0')
    $out | Out-File (Join-Path $base 'output.txt') -Encoding utf8

    $history = $hist | ConvertFrom-Json
    $recorded = @($history.entries | Where-Object {
        $_.command -eq $mark -and $_.pane_id -eq 'pane0'
    }).Count -gt 0
    $result = if ($recorded) { 'PASS' } else { 'FAIL' }
    Write-Host ""
    Write-Host "=== $result ===" -ForegroundColor ($(if ($recorded) {'Green'} else {'Red'}))
    Write-Host "Recorded marker '$mark': $recorded"
    Write-Host "history.json: $(Join-Path $base 'history.json')"
    Write-Host "output.txt:   $(Join-Path $base 'output.txt')"
    if (-not $recorded) {
        Write-Host "history list returned:"
        Write-Host $hist
    }
    exit $(if ($recorded) { 0 } else { 1 })
}
finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    # Hosted shells survive app termination by design. End only the hosts
    # installed inside this isolated run, including their shell children.
    $hostDir = Join-Path $base 'runtime\mtty-hosts'
    foreach ($meta in Get-ChildItem -LiteralPath $hostDir -Filter '*.json' -ErrorAction SilentlyContinue) {
        try {
            $hostInfo = Get-Content -LiteralPath $meta.FullName -Raw | ConvertFrom-Json
            $hostProcess = Get-Process -Id $hostInfo.pid -ErrorAction SilentlyContinue
            $expectedHost = Join-Path $base 'data\mtty\ptyhost\0.1.6\mtty-ptyhost.exe'
            if ($hostProcess -and $hostProcess.Path -eq $expectedHost) {
                & "$env:WINDIR\System32\taskkill.exe" /T /F /PID $hostProcess.Id | Out-Null
            }
        } catch {
            Write-Warning "Could not clean up an isolated PTY host: $_"
        }
    }
}

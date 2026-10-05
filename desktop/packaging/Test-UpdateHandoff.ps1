[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PlanPath
)

# GitHub Actions only: exercises the in-app update handoff of an isolated Test-mode installer
# on a disposable runner. Checks that a running (locked) versora.exe is still refused without
# /UPDATE, that /UPDATE waits for the executable to be released, and that /RELAUNCH starts
# the installed app afterwards.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Test-UpdateHandoff.ps1 runs on disposable GitHub Actions runners only.' }
$plan = Get-Content -LiteralPath $PlanPath -Raw | ConvertFrom-Json
if ($plan.mode -ne 'Test' -or -not $plan.testRoot) { throw 'Only an isolated Test-mode build plan is accepted.' }
$installer = $plan.installer
$exe = Join-Path $plan.installTarget 'versora.exe'
function Start-Installer([string[]]$Arguments) { Start-Process -FilePath $installer -ArgumentList $Arguments -PassThru }
function Wait-Installer($Process, [int]$Seconds) {
    if (-not $Process.WaitForExit($Seconds * 1000)) { $Process.Kill(); throw "Installer did not finish within $Seconds s." }
    $Process.ExitCode
}

$code = Wait-Installer (Start-Installer @('/S')) 120
if ($code -ne 0 -or -not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Fresh silent install failed ($code)." }
Write-Host 'PASS fresh silent install'

# A loaded image denies write access; a read-only share reproduces that lock.
$lock = [IO.File]::Open($exe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    $code = Wait-Installer (Start-Installer @('/S')) 60
    if ($code -ne 24) { throw "A locked executable must still be refused without /UPDATE (exit $code)." }
    Write-Host 'PASS locked executable refused without /UPDATE (exit 24)'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $update = Start-Installer @('/S', '/UPDATE')
    Start-Sleep -Seconds 6
    if ($update.HasExited) { throw "/UPDATE did not wait for the executable (exit $($update.ExitCode))." }
} finally { $lock.Dispose() }
$code = Wait-Installer $update 90
if ($code -ne 0) { throw "/UPDATE failed after the executable was released (exit $code)." }
Write-Host ("PASS /UPDATE waited {0:N1} s for the executable, then installed" -f $watch.Elapsed.TotalSeconds)

$code = Wait-Installer (Start-Installer @('/S', '/UPDATE', '/RELAUNCH')) 120
if ($code -ne 0) { throw "/UPDATE /RELAUNCH failed (exit $code)." }
$deadline = (Get-Date).AddSeconds(30)
$started = $null
while (-not $started -and (Get-Date) -lt $deadline) {
    $started = Get-Process -Name versora -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Select-Object -First 1
    if (-not $started) { Start-Sleep -Milliseconds 500 }
}
if (-not $started) { throw '/RELAUNCH did not start the installed versora.exe.' }
Write-Host "PASS /RELAUNCH started $exe (pid $($started.Id))"
Stop-Process -Id $started.Id -Force

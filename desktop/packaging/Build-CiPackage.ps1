[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('Production', 'Test')][string]$Mode,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-z0-9][a-z0-9-]{2,39}$')][string]$BuildId,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{2,39}$')][string]$TestId
)

# GitHub Actions only. Provisions the NSIS 3.11 archive Tauri publishes (SHA-256 pinned below),
# builds the static-CRT custom-protocol release executable, then hands both to the unchanged
# Build-WindowsPackage.ps1 checks. It never signs, publishes or installs anything.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Build-CiPackage.ps1 runs on GitHub Actions runners only; use Build-WindowsPackage.ps1 locally.' }
$desktopRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$nsisRoot = Join-Path $env:LOCALAPPDATA 'tauri\NSIS'
if (-not (Test-Path -LiteralPath (Join-Path $nsisRoot 'makensis.exe') -PathType Leaf)) {
    $zip = Join-Path $env:RUNNER_TEMP 'nsis-3.11.zip'
    Invoke-WebRequest -Uri 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip' -OutFile $zip
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne 'C7D27F780DDB6CFFB4730138CD1591E841F4B7EDB155856901CDF5F214394FA1') { throw 'NSIS 3.11 archive digest differs.' }
    $extract = Join-Path $env:RUNNER_TEMP 'nsis-extract'
    Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
    $null = New-Item -ItemType Directory -Force -Path (Split-Path $nsisRoot -Parent)
    Move-Item -LiteralPath (Join-Path $extract 'nsis-3.11') -Destination $nsisRoot
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$dumpbin = @(& $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe') | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin.exe was not found on this runner.' }
$toolchain = [regex]::Match((Get-Content -LiteralPath (Join-Path $desktopRoot 'rust-toolchain.toml') -Raw), 'channel\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $toolchain) { throw 'rust-toolchain.toml has no channel.' }
# Static CRT (no VC++ runtime import) and no runner profile paths inside the executable.
$env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$($env:USERPROFILE)=/build-host --remap-path-prefix=$desktopRoot=/source"
Push-Location $desktopRoot
try {
    & cargo "+$toolchain" build --locked --release -p versora-desktop --features custom-protocol
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
} finally { Pop-Location }
$exe = Join-Path $desktopRoot 'target\x86_64-pc-windows-msvc\release\versora.exe'
$params = @{
    Mode = $Mode
    SourceExe = $exe
    ExpectedExeSha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    SourceSha = (& git -C $desktopRoot rev-parse HEAD | Out-String).Trim()
    BuildId = $BuildId
    Dumpbin = $dumpbin
}
if ($TestId) { $params.TestId = $TestId }
$result = @(& (Join-Path $PSScriptRoot 'Build-WindowsPackage.ps1') @params) | Select-Object -Last 1 | ConvertFrom-Json
if ($env:GITHUB_OUTPUT) {
    "installer=$($result.Installer)" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "plan=$($result.Plan)" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
}
$result | ConvertTo-Json -Compress

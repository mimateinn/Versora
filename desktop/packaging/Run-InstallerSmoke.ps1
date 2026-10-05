[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('Install', 'Uninstall', 'Verify')][string]$Phase,
    [Parameter(Mandatory = $true)][string]$PlanPath,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedInstallerSha256
)

# Install follows concrete root review. Root owns the separate actual application
# launch/close between Install and Uninstall. This runner never launches/stops the
# app, changes formal entries, writes registry values or downloads prerequisites.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$allowedRoot = Join-Path $PSScriptRoot 'output\internal-smoke'
function Assert-Scoped([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($allowedRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Smoke path is outside the isolated packaging output root.' }
    $walk = $full
    while ($walk) {
        if (Test-Path -LiteralPath $walk) {
            if ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse ancestors are refused.' }
        }
        $parent = Split-Path $walk -Parent
        if ($parent -eq $walk) { break }
        $walk = $parent
    }
    return $full
}
function File-Digest([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false)) }

$planFile = Assert-Scoped $PlanPath
$plan = Get-Content -LiteralPath $planFile -Raw | ConvertFrom-Json
if ($plan.schema -ne 1 -or -not $plan.internalIncomplete -or $plan.publicationAllowed -or $plan.installationExecuted -or $plan.appLaunch) { throw 'Only an internal incomplete build plan is accepted.' }
if ($plan.registryWrites.Count -or $plan.shortcuts.Count -or $plan.associations.Count -or $plan.downloads.Count) { throw 'External integration or download is prohibited.' }
if ($plan.runId -notmatch '^r[0-9]{2,3}$') { throw 'Invalid run ID.' }
$runRoot = Join-Path $allowedRoot $plan.runId
if ($planFile -ne (Join-Path $runRoot 'build-plan.json')) { throw 'Build plan differs from its fixed run path.' }
$target = Assert-Scoped $plan.installTarget
$profile = Assert-Scoped $plan.externalProfileTarget
$installer = Assert-Scoped $plan.installer
if ($target -ne (Join-Path $runRoot 'installed') -or $profile -ne (Join-Path $runRoot 'external-profile')) { throw 'Target/profile differ from the fixed isolated paths.' }
if (-not $installer.StartsWith($runRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Installer does not belong to this run.' }
if ($plan.installerSha256 -ne $ExpectedInstallerSha256.ToLowerInvariant() -or (File-Digest $installer) -ne $ExpectedInstallerSha256.ToLowerInvariant()) { throw 'Installer differs from the explicitly reviewed SHA256.' }
$expectedNames = @('versora.exe', 'icon.ico', 'INTERNAL-NOTICE.txt', 'payload-manifest.json', 'dependency-license-inventory.json', 'cached-license-texts.txt' | Sort-Object)
if (Compare-Object $expectedNames @($plan.payload.name | Sort-Object)) { throw 'Plan payload differs from the fixed allowlist.' }
$statePath = Join-Path $runRoot 'smoke-state.json'
$evidencePath = Join-Path $runRoot 'installer-smoke-result.json'
$credentialPath = Join-Path $profile 'credentials.dpapi'
$plain = [Text.Encoding]::UTF8.GetBytes('{}')
$prefix = [Text.Encoding]::UTF8.GetBytes("VERSORA-DPAPI`0")
$fixtureNames = @('installed/keep-unknown-user-file.txt', 'installed/unknown-owner-subdirectory/keep-nested-user-file.txt', 'external-profile/projects/.sfts-ui.json', 'external-profile/data/outputs/keep-output.txt', 'external-profile/credentials.dpapi')
Add-Type -AssemblyName System.Security
function Assert-Preservation {
    foreach ($relative in $fixtureNames) {
        $path = Assert-Scoped (Join-Path $runRoot $relative)
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (File-Digest $path) -ne $state.fixtureHashes[$relative]) { throw "Preservation fixture changed: $relative" }
    }
    $stored = [IO.File]::ReadAllBytes($credentialPath)
    if ($stored.Length -le $prefix.Length) { throw 'Encrypted profile was truncated.' }
    if ([Convert]::ToBase64String([byte[]]$stored[0..($prefix.Length - 1)]) -ne [Convert]::ToBase64String($prefix)) { throw 'DPAPI framing changed.' }
    $recovered = [Security.Cryptography.ProtectedData]::Unprotect([byte[]]$stored[$prefix.Length..($stored.Length - 1)], $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    if ([Convert]::ToBase64String($recovered) -ne [Convert]::ToBase64String($plain)) { throw 'DPAPI decryptability changed.' }
}

if ($Phase -eq 'Install') {
    if (Test-Path -LiteralPath $statePath) { throw 'Install already started; use a fresh run without overwriting evidence.' }
    if ((Test-Path -LiteralPath $target) -or (Test-Path -LiteralPath $profile)) { throw 'Existing target/profile is never reused or overwritten.' }
    $null = New-Item -ItemType Directory -Path $target, (Join-Path $target 'unknown-owner-subdirectory'), (Join-Path $profile 'projects'), (Join-Path $profile 'data\outputs')
    Write-Utf8 (Join-Path $target 'keep-unknown-user-file.txt') 'Unknown owner-independent preservation fixture.'
    Write-Utf8 (Join-Path $target 'unknown-owner-subdirectory\keep-nested-user-file.txt') 'Nested unknown preservation fixture.'
    Write-Utf8 (Join-Path $profile 'projects\.sfts-ui.json') '{"theme":"dark","lang":"en"}'
    Write-Utf8 (Join-Path $profile 'data\outputs\keep-output.txt') 'External output preservation fixture.'
    # Valid empty native credentials object, actual CurrentUser DPAPI. No key,
    # placeholder credential or selected provider exists in this profile.
    $cipher = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [IO.File]::WriteAllBytes($credentialPath, [byte[]]($prefix + $cipher))
    $hashes = @{}
    foreach ($relative in $fixtureNames) { $hashes[$relative] = File-Digest (Join-Path $runRoot $relative) }
    $state = @{ schema = 1; internalIncomplete = $true; installerSha256 = $plan.installerSha256; installTarget = $target; externalProfileTarget = $profile; appLaunchedByRunner = $false; install = 'NOT_RUN'; uninstall = 'NOT_RUN'; preservation = 'NOT_RUN'; fixtureHashes = $hashes; failures = @() }
} else {
    if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) { throw 'Install has not produced actual smoke state.' }
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json -AsHashtable
    if ($state.schema -ne 1 -or $state.installerSha256 -ne $plan.installerSha256 -or $state.installTarget -ne $target -or $state.externalProfileTarget -ne $profile -or $state.install -ne 'PASS') { throw 'Installed smoke state differs from the reviewed plan.' }
    if (Compare-Object @($fixtureNames | Sort-Object) @($state.fixtureHashes.Keys | Sort-Object)) { throw 'Preservation baseline differs from the fixed fixture list.' }
}

try {
    if ($Phase -eq 'Install') {
        $process = Start-Process -FilePath $installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
        $state.installExitCode = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw "Installer exited $($process.ExitCode)." }
        foreach ($file in $plan.payload) {
            $installed = Join-Path $target $file.name
            if (-not (Test-Path -LiteralPath $installed -PathType Leaf) -or (File-Digest $installed) -ne $file.sha256) { throw "Installed payload differs: $($file.name)" }
        }
        if ([IO.File]::ReadAllText((Join-Path $target '.versora-internal-token')) -ne $plan.runToken) { throw 'Scope marker differs.' }
        $state.uninstallerSha256 = File-Digest (Join-Path $target 'uninstall.exe')
        Assert-Preservation
        $state.install = 'PASS'
        $state.status = 'PASS_INTERNAL_INSTALL_ONLY_AWAITING_ROOT_LAUNCH'
    } elseif ($Phase -eq 'Uninstall') {
        if ($state.uninstall -ne 'NOT_RUN') { throw 'Uninstall already ran; current files/evidence are retained.' }
        $uninstaller = Join-Path $target 'uninstall.exe'
        if ((File-Digest $uninstaller) -ne $state.uninstallerSha256) { throw 'Installed uninstaller changed.' }
        $ownedExe = Join-Path $target 'versora.exe'
        $running = @(Get-CimInstance -ClassName Win32_Process -Filter "Name='versora.exe'" | Where-Object { $_.ExecutablePath -eq $ownedExe })
        if ($running.Count) { throw 'The installed app is still running. Root must finish verification and close its owned window first.' }
        Assert-Preservation
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
        $state.uninstallLauncherExitCode = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw "Uninstaller launcher exited $($process.ExitCode)." }
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        $removedNames = @($expectedNames + @('uninstall.exe', '.versora-internal-token'))
        do {
            $remaining = @($removedNames | Where-Object { Test-Path -LiteralPath (Join-Path $target $_) })
            if ($remaining.Count -eq 0) { break }
            Start-Sleep -Milliseconds 150
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($remaining.Count) { throw "Known payload remains: $($remaining -join ', ')" }
        Assert-Preservation
        $state.uninstall = 'PASS'
        $state.status = 'PASS_INTERNAL_UNINSTALL_AWAITING_FINAL_VERIFY'
    } else {
        if ($state.uninstall -ne 'PASS') { throw 'Verify requires successful actual Uninstall.' }
        foreach ($name in @($expectedNames + @('uninstall.exe', '.versora-internal-token'))) {
            if (Test-Path -LiteralPath (Join-Path $target $name)) { throw "Known payload reappeared: $name" }
        }
        Assert-Preservation
        $state.preservation = 'PASS'
        $state.status = 'PASS_INTERNAL_INSTALL_UNINSTALL_PRESERVATION_ONLY'
    }
    $state.lastPhase = $Phase
} catch {
    $state.failures = @($_.Exception.Message)
    $state.status = 'FAIL'
    throw
} finally {
    $json = $state | ConvertTo-Json -Depth 8
    Write-Utf8 $statePath $json
    Write-Utf8 $evidencePath $json
}
[pscustomobject]@{ Evidence = $evidencePath; Phase = $Phase; Status = $state.status; AppLaunchedByRunner = $false } | ConvertTo-Json -Compress

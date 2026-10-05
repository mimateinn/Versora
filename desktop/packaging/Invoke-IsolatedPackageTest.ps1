[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('Install', 'Upgrade', 'RefuseRunningApp', 'RefuseReparse', 'Uninstall', 'Verify')][string]$Phase,
    [Parameter(Mandatory = $true)][string]$PlanPath,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedInstallerSha256,
    [switch]$FixtureFree
)
# Root reviews the concrete compiled plan before Install/Upgrade. Root owns actual
# native app launch/close between phases. This runner never launches/stops an app.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
function Digest([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false)) }
$outputRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'output'))
function Scoped([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($outputRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Test path is outside ignored packaging/output.' }
    $walk = $full
    while ($walk) {
        if ((Test-Path -LiteralPath $walk) -and ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Reparse path refused.' }
        $parent = Split-Path $walk -Parent
        if ($parent -eq $walk) { break }
        $walk = $parent
    }
    $full
}
$planFile = Scoped $PlanPath
$plan = Get-Content -LiteralPath $planFile -Raw | ConvertFrom-Json
if ($plan.schema -ne 2 -or $plan.mode -ne 'Test' -or $plan.publicationAllowed -or $plan.installationExecuted -or $plan.testId -notmatch '^[a-z0-9][a-z0-9-]{2,39}$') { throw 'Only a reviewed isolated-test plan is accepted.' }
$testRoot = Join-Path $PSScriptRoot "output\production-tests\$($plan.testId)"
$target = Scoped (Join-Path $testRoot 'installed')
$shortcut = Scoped (Join-Path $testRoot 'integration\Versora.lnk')
$privateKey = "Software\mimateinn\Versora\PackagingTests\$($plan.testId)"
if ($plan.testRoot -ne $testRoot -or $plan.installTarget -ne $target -or $plan.shortcut -ne $shortcut -or $plan.registryKey -ne $privateKey -or $plan.registryHive -ne 'HKCU' -or $plan.registryView -ne 64 -or $plan.runtimeDownloads.Count -or $plan.associations.Count -or $plan.standardAppDataWritesByInstaller.Count) { throw 'Test scope differs from fixed reviewed private targets.' }
$installer = Scoped $plan.installer
if ((Digest $installer) -ne $ExpectedInstallerSha256.ToLowerInvariant() -or $plan.installerSha256 -ne $ExpectedInstallerSha256.ToLowerInvariant()) { throw 'Installer digest differs from root review.' }
$names = @('versora.exe', 'icon.ico', 'README-Windows.txt', 'RELEASE-METADATA.json', 'THIRD-PARTY-NOTICES.txt', 'DEPENDENCIES.json', 'SOURCE-LICENSES.txt', 'Rust-COPYRIGHT-library.html', 'font-OFL.txt', 'Libron-LICENSE.txt', 'Libron-COPYRIGHT.txt', 'Libron-SOURCE.json', 'third-party-sources.zip', 'payload-manifest.json')
if (Compare-Object @($names | Sort-Object) @($plan.payload.name | Sort-Object)) { throw 'Unexpected payload allowlist.' }
$statePath = Join-Path $testRoot 'package-test-state.json'
$resultPath = Join-Path $testRoot 'package-test-result.json'
$fixtureNames = @('installed/unknown-user-file.txt', 'installed/unknown-user-directory/keep.txt', 'integration/unknown-shortcut-directory-file.txt', 'external-profile/credentials.dpapi', 'external-profile/projects/.sfts-ui.json', 'external-profile/data/outputs/keep.txt')
$unknownJunction = Join-Path $target 'unknown-user-junction'
$junctionTarget = Scoped (Join-Path $testRoot 'external-profile\data\outputs')
$prefix = [Text.Encoding]::UTF8.GetBytes("VERSORA-DPAPI`0")
$plain = [Text.Encoding]::UTF8.GetBytes('{}')
Add-Type -AssemblyName System.Security
$baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry64)
function Assert-Preserved {
    if ($state.fixtureFree) {
        if ($state.fixtureHashes.Count) { throw 'Fixture-free state unexpectedly contains preservation fixtures.' }
        return
    }
    foreach ($relative in $fixtureNames) {
        $path = Scoped (Join-Path $testRoot $relative)
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Digest $path) -ne $state.fixtureHashes[$relative]) { throw "Preservation failed: $relative" }
    }
    $junction = Get-Item -LiteralPath $unknownJunction -Force
    if (-not ($junction.Attributes -band [IO.FileAttributes]::ReparsePoint) -or @($junction.Target).Count -ne 1 -or [IO.Path]::GetFullPath([string](@($junction.Target)[0])) -ne $junctionTarget) { throw 'Unknown junction/link destination was changed.' }
    $stored = [IO.File]::ReadAllBytes((Join-Path $testRoot 'external-profile\credentials.dpapi'))
    if ($stored.Length -le $prefix.Length -or [Convert]::ToBase64String([byte[]]$stored[0..($prefix.Length-1)]) -ne [Convert]::ToBase64String($prefix)) { throw 'External DPAPI framing changed.' }
    $decoded = [Security.Cryptography.ProtectedData]::Unprotect([byte[]]$stored[$prefix.Length..($stored.Length-1)], $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    if ([Convert]::ToBase64String($decoded) -ne [Convert]::ToBase64String($plain)) { throw 'External DPAPI decryptability changed.' }
}
function Assert-AppClosed {
    $exe = Join-Path $target 'versora.exe'
    if (@(Get-CimInstance -ClassName Win32_Process -Filter "Name='versora.exe'" | Where-Object { $_.ExecutablePath -eq $exe }).Count) { throw 'Root must finish its native launch and close the owned installed app first.' }
}
function Assert-Installed {
    foreach ($file in $plan.payload) {
        $path = Join-Path $target $file.name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Digest $path) -ne $file.sha256) { throw "Installed payload differs: $($file.name)" }
    }
    if ([IO.File]::ReadAllText((Join-Path $target '.versora-installation')) -ne "com.mimateinn.versora|Test:$($plan.testId)") { throw 'Ownership marker differs.' }
    $key = $baseKey.OpenSubKey($privateKey)
    if (-not $key) { throw 'Private test registration is absent.' }
    try {
        if ($key.GetValue('InstallLocation') -ne $target -or $key.GetValue('DisplayVersion') -ne $plan.version -or $key.GetValue('UninstallString') -ne ('"' + (Join-Path $target 'uninstall.exe') + '"')) { throw 'Private test registration differs.' }
    } finally { $key.Dispose() }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($shortcut)
        if ($link.TargetPath -ne (Join-Path $target 'versora.exe') -or $link.IconLocation -ne ((Join-Path $target 'icon.ico') + ',0') -or $link.WorkingDirectory -ne $target) { throw 'Private native shortcut target/icon/working directory differs.' }
    } finally { $null = [Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
    Assert-Preserved
}
if ($Phase -eq 'Install') {
    if (Test-Path -LiteralPath $testRoot) { throw 'Fresh Install requires a new test directory, never an existing installation/profile.' }
    $key = $baseKey.OpenSubKey($privateKey)
    if ($key) { $key.Dispose(); throw 'Private test key already exists; choose a fresh TestId.' }
    $hashes = @{}
    if ($FixtureFree) {
        # Only the evidence parent exists. The actual program and shortcut
        # directories must be absent when the real installer starts.
        $null = New-Item -ItemType Directory -Path $testRoot
        if ((Test-Path -LiteralPath $target) -or (Test-Path -LiteralPath (Split-Path $shortcut -Parent))) { throw 'Fixture-free installation requires absent program/shortcut directories.' }
    } else {
    $null = New-Item -ItemType Directory -Path (Join-Path $target 'unknown-user-directory'), (Join-Path $testRoot 'integration'), (Join-Path $testRoot 'external-profile\projects'), (Join-Path $testRoot 'external-profile\data\outputs')
    foreach ($relative in $fixtureNames | Where-Object { $_ -notlike '*credentials.dpapi' -and $_ -notlike '*/.sfts-ui.json' }) { Write-Utf8 (Join-Path $testRoot $relative) 'Owner-independent unknown-file preservation fixture.' }
    Write-Utf8 (Join-Path $testRoot 'external-profile\projects\.sfts-ui.json') '{"theme":"dark","lang":"en"}'
    $cipher = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [IO.File]::WriteAllBytes((Join-Path $testRoot 'external-profile\credentials.dpapi'), [byte[]]($prefix + $cipher))
    # An unknown nested junction is deliberately preserved and never enumerated
    # by the real installer. Both link and target remain in the reviewed scope.
    $null = New-Item -ItemType Junction -Path $unknownJunction -Target $junctionTarget
    foreach ($relative in $fixtureNames) { $hashes[$relative] = Digest (Join-Path $testRoot $relative) }
    }
    $state = @{ schema = 2; testId = $plan.testId; fixtureFree = [bool]$FixtureFree; installTargetAbsentBeforeInstall = [bool]$FixtureFree; install = 'NOT_RUN'; upgrade = 'NOT_RUN'; runningAppRefusal = 'NOT_RUN'; reparseRefusal = 'NOT_RUN'; uninstall = 'NOT_RUN'; preservation = 'NOT_RUN'; fixtureHashes = $hashes; appLaunchedByRunner = $false; failures = @() }
} else {
    if ($FixtureFree) { throw 'FixtureFree is selected only for Install; later phases use its saved actual state.' }
    if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) { throw 'Actual fresh Install state is required.' }
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json -AsHashtable
    if (-not $state.ContainsKey('fixtureFree')) { $state.fixtureFree = $false }
    $fixtureMismatch = if ($state.fixtureFree) { $state.fixtureHashes.Count -ne 0 } else { [bool](Compare-Object @($fixtureNames | Sort-Object) @($state.fixtureHashes.Keys | Sort-Object)) }
    if ($state.schema -ne 2 -or $state.testId -ne $plan.testId -or $state.install -ne 'PASS' -or $fixtureMismatch) { throw 'Preservation state differs from the actual installed test.' }
}
try {
    if ($Phase -eq 'Install' -or $Phase -eq 'Upgrade') {
        if ($Phase -eq 'Upgrade') {
            if ($state.uninstall -ne 'NOT_RUN' -or $state.currentBuildId -eq $plan.buildId) { throw 'Upgrade requires a new compiled BuildId over an actual owned installation.' }
            Assert-AppClosed
            Assert-Preserved
        }
        $process = Start-Process -FilePath $installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
        $state.lastInstallerExitCode = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw "Installer exited $($process.ExitCode)." }
        Assert-Installed
        $state.currentBuildId = $plan.buildId
        $state.currentInstallerSha256 = $plan.installerSha256
        $state.uninstallerSha256 = Digest (Join-Path $target 'uninstall.exe')
        $state[$Phase.ToLowerInvariant()] = 'PASS'
        $state.status = 'PASS_' + $Phase.ToUpperInvariant() + '_AWAITING_ROOT_NATIVE_LAUNCH'
    } elseif ($Phase -eq 'RefuseRunningApp' -or $Phase -eq 'RefuseReparse') {
        if ($state.currentInstallerSha256 -ne $plan.installerSha256 -or $state.uninstall -ne 'NOT_RUN') { throw 'Refusal checks require the current actual owned installation.' }
        Assert-Installed
        if ($Phase -eq 'RefuseRunningApp') {
            $exe = Join-Path $target 'versora.exe'
            if (-not @(Get-CimInstance -ClassName Win32_Process -Filter "Name='versora.exe'" | Where-Object { $_.ExecutablePath -eq $exe }).Count) { throw 'Root must launch the owned installed app first; runner never launches or stops it.' }
            $process = Start-Process -FilePath $installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
            if ($process.ExitCode -ne 24) { throw "Running-app refusal expected exit 24, observed $($process.ExitCode)." }
            $state.runningAppRefusal = 'PASS'
        } else {
            if ($state.fixtureFree) { throw 'Reparse fixture check requires the separate preservation test mode.' }
            Assert-AppClosed
            $knownPath = Scoped (Join-Path $target 'THIRD-PARTY-NOTICES.txt')
            $savedPath = Scoped (Join-Path $testRoot ('refusal-fixtures\' + $plan.buildId + '-THIRD-PARTY-NOTICES.txt'))
            if (Test-Path -LiteralPath $savedPath) { throw 'Never overwrite a previous refusal fixture.' }
            $null = New-Item -ItemType Directory -Path (Split-Path $savedPath -Parent) -Force
            Move-Item -LiteralPath $knownPath -Destination $savedPath
            try {
                $null = New-Item -ItemType Junction -Path $knownPath -Target $junctionTarget
                $process = Start-Process -FilePath $installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
                if ($process.ExitCode -ne 21) { throw "Known-file junction refusal expected exit 21, observed $($process.ExitCode)." }
                Assert-Preserved
                $state.reparseRefusal = 'PASS'
            } finally {
                if (Test-Path -LiteralPath $knownPath) {
                    $link = Get-Item -LiteralPath $knownPath -Force
                    if (-not ($link.Attributes -band [IO.FileAttributes]::ReparsePoint) -or @($link.Target).Count -ne 1 -or [IO.Path]::GetFullPath([string](@($link.Target)[0])) -ne $junctionTarget) { throw 'Refusal link changed; preserve it and the saved payload for root recovery.' }
                    # Exact validated junction only; no recursion or target removal.
                    Remove-Item -LiteralPath $knownPath -Force
                }
                Move-Item -LiteralPath $savedPath -Destination $knownPath
            }
        }
        Assert-Installed
        if ((Digest (Join-Path $target 'uninstall.exe')) -ne $state.uninstallerSha256) { throw 'Refused installer changed the owned uninstaller.' }
        $state.status = 'PASS_' + $Phase.ToUpperInvariant() + '_NO_PAYLOAD_CHANGES'
    } elseif ($Phase -eq 'Uninstall') {
        if ($state.currentInstallerSha256 -ne $plan.installerSha256 -or $state.uninstall -ne 'NOT_RUN') { throw 'Use the current installed plan and run Uninstall only once.' }
        Assert-AppClosed
        Assert-Installed
        $uninstaller = Join-Path $target 'uninstall.exe'
        if ((Digest $uninstaller) -ne $state.uninstallerSha256) { throw 'Installed uninstaller changed.' }
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
        $state.uninstallLauncherExitCode = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw 'Uninstaller launcher failed.' }
        $deadline = [DateTime]::UtcNow.AddSeconds(45)
        do {
            $remaining = @(@($names + @('uninstall.exe', '.versora-installation')) | Where-Object { Test-Path -LiteralPath (Join-Path $target $_) })
            $key = $baseKey.OpenSubKey($privateKey)
            $registrationRemains = ($null -ne $key)
            if ($key) { $key.Dispose() }
            if (-not $remaining.Count -and -not $registrationRemains -and -not (Test-Path -LiteralPath $shortcut)) { break }
            Start-Sleep -Milliseconds 150
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($remaining.Count -or $registrationRemains -or (Test-Path -LiteralPath $shortcut)) { throw 'Known payload/private integration remains after uninstall.' }
        Assert-Preserved
        $state.uninstall = 'PASS'
        $state.status = 'PASS_UNINSTALL_AWAITING_VERIFY'
    } else {
        if ($state.currentInstallerSha256 -ne $plan.installerSha256 -or $state.uninstall -ne 'PASS') { throw 'Final Verify requires completed actual Uninstall.' }
        Assert-Preserved
        if ($state.fixtureFree) {
            if (Test-Path -LiteralPath $target) { throw 'Fixture-free uninstall left the program directory behind.' }
            $state.preservation = 'NOT_APPLICABLE_NO_FIXTURES'
            $state.status = 'PASS_FIXTURE_FREE_INSTALL_UNINSTALL'
        } else {
            $state.preservation = 'PASS'
            $state.status = if ($state.upgrade -eq 'PASS') { 'PASS_ISOLATED_INSTALL_UPGRADE_UNINSTALL_PRESERVATION' } else { 'PASS_ISOLATED_INSTALL_UNINSTALL_PRESERVATION' }
        }
    }
    $state.lastPhase = $Phase
} catch {
    $state.failures = @($_.Exception.Message)
    $state.status = 'FAIL'
    throw
} finally {
    $json = $state | ConvertTo-Json -Depth 8
    Write-Utf8 $statePath $json
    Write-Utf8 $resultPath $json
    $baseKey.Dispose()
}
[pscustomobject]@{ Evidence = $resultPath; Phase = $Phase; Status = $state.status; AppLaunchedByRunner = $false } | ConvertTo-Json -Compress

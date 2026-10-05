[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SourceExe,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedSourceSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^r[0-9]{2,3}$')][string]$RunId
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$desktopRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$repoRoot = Split-Path $desktopRoot -Parent
$taskRoot = Split-Path $repoRoot -Parent
$runRoot = Join-Path $PSScriptRoot "output\internal-smoke\$RunId"
$stageRoot = Join-Path $runRoot 'stage'
$installTarget = Join-Path $runRoot 'installed'
$profileTarget = Join-Path $runRoot 'external-profile'
$compiler = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
$script = Join-Path $PSScriptRoot 'internal-smoke.nsi'
$icon = Join-Path $desktopRoot 'src-tauri\icons\icon.ico'
$iconSha = '07468CF4A4982765151BD49AF618F54E66288AA58E9006AC7612A2E4F9D60823'

function Assert-WorkspacePath([string]$Path, [switch]$MustExist) {
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = $taskRoot.TrimEnd('\') + '\'
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path is outside the task workspace: $full"
    }
    if ($full -match '["$;\r\n]') { throw 'NSIS paths must not contain interpolation or command delimiters.' }
    if ($MustExist -and -not (Test-Path -LiteralPath $full -PathType Leaf)) { throw "Required file is absent: $full" }
    $walk = $full
    while ($walk) {
        if (Test-Path -LiteralPath $walk) {
            if ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Reparse point refused: $walk"
            }
        }
        $parent = Split-Path $walk -Parent
        if ($parent -eq $walk) { break }
        $walk = $parent
    }
    return $full
}

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

$source = Assert-WorkspacePath $SourceExe -MustExist
$null = Assert-WorkspacePath $runRoot
$null = Assert-WorkspacePath $icon -MustExist
$null = Assert-WorkspacePath $script -MustExist
if ([IO.Path]::GetFileName($source) -ne 'versora.exe') { throw 'The input must be the actual native versora.exe.' }
if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $ExpectedSourceSha256.ToUpperInvariant()) {
    throw 'Source EXE differs from the explicitly reviewed SHA256. Nothing was staged.'
}
if ((Get-FileHash -LiteralPath $icon -Algorithm SHA256).Hash -ne $iconSha) { throw 'Formal icon SHA256 differs.' }
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) { throw 'The audited existing NSIS compiler is missing. No installation is attempted.' }
if (Test-Path -LiteralPath $runRoot) { throw 'This run directory already exists. Choose a new run ID; no existing files are overwritten.' }
$compilerVersion = (& $compiler /VERSION | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $compilerVersion -ne 'v3.11') { throw 'Expected the audited existing NSIS v3.11.' }
$config = Get-Content -LiteralPath (Join-Path $desktopRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = [string]$config.version
if ($version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw 'Application version is not a safe SemVer label.' }

# Offline metadata is inspected before creating the run directory. Missing crates
# fail here instead of causing a download or producing a misleading package.
Push-Location $desktopRoot
try {
    $rawMetadata = & cargo +1.97.1-x86_64-pc-windows-msvc metadata --offline --locked --filter-platform x86_64-pc-windows-msvc --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Locked offline Cargo metadata failed. No dependency download is attempted.' }
    $metadata = ($rawMetadata -join "`n") | ConvertFrom-Json
    $baseline = (& git rev-parse HEAD | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Source baseline lookup failed.' }
} finally { Pop-Location }
$packageMap = @{}
foreach ($item in $metadata.packages) { $packageMap[$item.id] = $item }
$nodeMap = @{}
foreach ($item in $metadata.resolve.nodes) { $nodeMap[$item.id] = $item }
$rootPackage = $metadata.packages | Where-Object name -eq 'versora-desktop'
$visited = [Collections.Generic.HashSet[string]]::new()
$queue = [Collections.Generic.Queue[string]]::new()
$queue.Enqueue($rootPackage.id)
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if (-not $visited.Add($id)) { continue }
    foreach ($dep in $nodeMap[$id].deps) {
        if (@($dep.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) { $queue.Enqueue($dep.pkg) }
    }
}
$inventory = [Collections.Generic.List[object]]::new()
$licenseText = [Text.StringBuilder]::new()
$null = $licenseText.AppendLine('INTERNAL ENGINEERING LICENSE SNAPSHOT - NOT A COMPLETE PUBLIC RELEASE NOTICE')
$null = $licenseText.AppendLine('This snapshot contains unmodified license/notice files already present in the local locked Cargo source cache. Missing texts and vendored SDK notices remain release gates.')
$selected = @($visited | ForEach-Object { $packageMap[$_] } | Sort-Object name, version)
foreach ($package in $selected) {
    $packageRoot = Split-Path $package.manifest_path -Parent
    $isRegistry = [string]$package.source -like 'registry+*'
    $files = @()
    if ($isRegistry) {
        $files = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -File -Force |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([-._]|$)' -and $_.Length -le 1048576 } |
            Sort-Object FullName)
    }
    $inventory.Add([pscustomobject]@{
        name = $package.name
        version = $package.version
        declaredLicense = $package.license
        repository = $package.repository
        registryDependency = $isRegistry
        cachedTexts = @($files | ForEach-Object { [IO.Path]::GetRelativePath($packageRoot, $_.FullName).Replace('\', '/') })
        missingCachedText = ($isRegistry -and $files.Count -eq 0)
    })
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath($packageRoot, $file.FullName).Replace('\', '/')
        $null = $licenseText.AppendLine("`n===== $($package.name) $($package.version) / $relative =====")
        $null = $licenseText.AppendLine([IO.File]::ReadAllText($file.FullName))
    }
}
$runToken = 'versora-internal-' + $RunId + '-' + [Guid]::NewGuid().ToString('N')
$buildKind = if ($source -match '\\release\\') { 'release' } else { 'debug' }
$binaryLimit = if ($buildKind -eq 'release') { 'This is a release-mode native binary; static CRT, production custom protocol and runtime imports require their separate binary-validation evidence.' } else { 'This is a debug native binary and may require an existing Microsoft Visual C++ runtime.' }
$notice = @"
VERSORA INTERNAL INCOMPLETE NATIVE PACKAGE TEST

This is a private engineering artifact, not a release, installer offer, or acceptance of full feature parity.
Version: $version
Source baseline: $baseline (native migration changes are uncommitted)
Frozen executable SHA256: $($ExpectedSourceSha256.ToLowerInvariant())
$binaryLimit
CSV/TSV, YAML and PDF parity is pending dependency/font approval. Paid provider/real CLI translation is NOT_RUN.
The project has no adopted distribution license in this source. Do not infer MIT or another project license from dependency licenses.
The cached license snapshot is deliberately incomplete and must not serve as final public release compliance evidence.
This installer writes only its compiled task-workspace target. It creates no Start Menu shortcuts, associations, registry entries, runtime installations or downloads.
The uninstaller removes enumerated test payload only. Unknown files and external encrypted profiles are preserved.
Do not publish this package or repoint the formal Versora entry to it.
"@

$null = New-Item -ItemType Directory -Path $stageRoot
Copy-Item -LiteralPath $source -Destination (Join-Path $stageRoot 'versora.exe')
Copy-Item -LiteralPath $icon -Destination (Join-Path $stageRoot 'icon.ico')
if ((Get-FileHash -LiteralPath (Join-Path $stageRoot 'versora.exe') -Algorithm SHA256).Hash -ne $ExpectedSourceSha256.ToUpperInvariant()) { throw 'Staged EXE SHA256 differs.' }
if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $ExpectedSourceSha256.ToUpperInvariant()) { throw 'Source EXE changed during staging. Partial evidence is retained; no installer is compiled.' }
Write-Utf8 (Join-Path $stageRoot 'INTERNAL-NOTICE.txt') $notice
Write-Utf8 (Join-Path $stageRoot 'cached-license-texts.txt') $licenseText.ToString()
Write-Utf8 (Join-Path $stageRoot 'dependency-license-inventory.json') ([ordered]@{
    scope = 'Conservative Windows normal/build dependency metadata traversal; not a claim that every listed crate is linked.'
    completePublicReleaseNotice = $false
    packageCount = $inventory.Count
    missingCachedTextCount = @($inventory | Where-Object missingCachedText).Count
    packages = $inventory.ToArray()
} | ConvertTo-Json -Depth 8)
$payloadNames = @('versora.exe', 'icon.ico', 'INTERNAL-NOTICE.txt', 'dependency-license-inventory.json', 'cached-license-texts.txt')
$manifest = @($payloadNames | ForEach-Object {
    $path = Join-Path $stageRoot $_
    [ordered]@{ name = $_; bytes = (Get-Item -LiteralPath $path).Length; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
})
Write-Utf8 (Join-Path $stageRoot 'payload-manifest.json') ([ordered]@{ schema = 1; internalIncomplete = $true; files = $manifest; selfExcludedFromHashList = 'payload-manifest.json' } | ConvertTo-Json -Depth 6)
$expectedNames = @($payloadNames + 'payload-manifest.json' | Sort-Object)
$actualNames = @(Get-ChildItem -LiteralPath $stageRoot -File | Select-Object -ExpandProperty Name | Sort-Object)
if (Compare-Object $expectedNames $actualNames) { throw 'Stage differs from the explicit payload allowlist.' }
$output = Join-Path $runRoot "Versora-$version-INTERNAL-INCOMPLETE-$RunId-setup.exe"
$argsList = @('/V3', '/DINTERNAL_TEST=1', "/DSTAGE_DIR=$stageRoot", "/DEXPECTED_INSTALL_DIR=$installTarget", "/DOUT_FILE=$output", "/DRUN_TOKEN=$runToken", "/DAPP_VERSION=$version", $script)
$compileLog = & $compiler @argsList 2>&1
$compileCode = $LASTEXITCODE
Write-Utf8 (Join-Path $runRoot 'nsis-build.log') ($compileLog -join "`n")
if ($compileCode -ne 0) { throw "NSIS compile failed ($compileCode); see the retained build log." }
$payload = @($expectedNames | ForEach-Object {
    $path = Join-Path $stageRoot $_
    [ordered]@{ name = $_; bytes = (Get-Item -LiteralPath $path).Length; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$plan = [ordered]@{
    schema = 1
    internalIncomplete = $true
    installationExecuted = $false
    publicationAllowed = $false
    version = $version
    runId = $RunId
    runToken = $runToken
    sourceBaselineSha = $baseline
    sourceExeSha256 = $ExpectedSourceSha256.ToLowerInvariant()
    buildKind = $buildKind
    compiler = $compiler
    compilerVersion = $compilerVersion
    compilerSha256 = (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash.ToLowerInvariant()
    nsisScriptSha256 = (Get-FileHash -LiteralPath $script -Algorithm SHA256).Hash.ToLowerInvariant()
    installer = $output
    installerBytes = (Get-Item -LiteralPath $output).Length
    installerSha256 = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    stage = $stageRoot
    installTarget = $installTarget
    externalProfileTarget = $profileTarget
    registryWrites = @()
    shortcuts = @()
    associations = @()
    downloads = @()
    appLaunch = $false
    prerequisite = 'Existing x64 Windows 10/11 and Evergreen WebView2; consult separate frozen binary import evidence.'
    payload = $payload
    plannedChecks = @('Installer and installed payload SHA256 equality', 'Unknown file/subdirectory preservation', 'External real current-user DPAPI decryptability and file hash preservation', 'Enumerated-only uninstaller cleanup', 'No formal entry/registration changes')
}
$planPath = Join-Path $runRoot 'build-plan.json'
Write-Utf8 $planPath ($plan | ConvertTo-Json -Depth 8)
[pscustomobject]@{ Plan = $planPath; Installer = $output; InstallerSHA256 = $plan.installerSha256; InstallTarget = $installTarget; InstallationExecuted = $false } | ConvertTo-Json -Compress

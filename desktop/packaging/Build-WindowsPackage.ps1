[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('Production', 'Test')][string]$Mode,
    [Parameter(Mandatory = $true)][string]$SourceExe,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedExeSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Fa-f0-9]{40}$')][string]$SourceSha,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-z0-9][a-z0-9-]{2,39}$')][string]$BuildId,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{2,39}$')][string]$TestId,
    [string]$SupplementManifest = (Join-Path $PSScriptRoot 'notices\provenance.json'),
    # CI runners have a different MSVC layout; pass the located dumpbin.exe there.
    [string]$Dumpbin = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\dumpbin.exe'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$desktopRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$repoRoot = Split-Path $desktopRoot -Parent
$compiler = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
$dumpbin = $Dumpbin
$nsi = Join-Path $PSScriptRoot 'versora.nsi'
$runRoot = Join-Path $PSScriptRoot "output\builds\$BuildId"
$stage = Join-Path $runRoot 'stage'
$icon = Join-Path $desktopRoot 'src-tauri\icons\icon.ico'
$iconSha = '07468cf4a4982765151bd49af618f54e66288aa58e9006ac7612a2e4f9d60823'
function Digest([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false)) }
function Assert-NoReparse([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if ($full -match '["$;\r\n]') { throw 'NSIS paths must not contain string interpolation/delimiters.' }
    $walk = $full
    while ($walk) {
        if ((Test-Path -LiteralPath $walk) -and ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Reparse path refused: $walk" }
        $parent = Split-Path $walk -Parent
        if ($parent -eq $walk) { break }
        $walk = $parent
    }
    $full
}
$source = Assert-NoReparse $SourceExe
if (-not $source.StartsWith($desktopRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($source) -ne 'versora.exe' -or $source -notmatch '\\release\\') { throw 'Use the explicit isolated native release/versora.exe.' }
if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or (Digest $source) -ne $ExpectedExeSha256.ToLowerInvariant()) { throw 'Release EXE differs from the reviewed digest.' }
$null = Assert-NoReparse $runRoot
$null = Assert-NoReparse $icon
if (Test-Path -LiteralPath $runRoot) { throw 'Build directory already exists; previous artifacts/evidence are never overwritten.' }
if ((Digest $icon) -ne $iconSha) { throw 'Formal icon differs.' }
foreach ($tool in @($compiler, $dumpbin, $nsi)) { if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw 'An audited existing packaging tool/script is missing; no installation is attempted.' } }
if ((& $compiler /VERSION | Out-String).Trim() -ne 'v3.11' -or $LASTEXITCODE -ne 0) { throw 'Expected existing NSIS 3.11.' }
Push-Location $repoRoot
try {
    $head = (& git rev-parse HEAD | Out-String).Trim().ToLowerInvariant()
    if ($LASTEXITCODE -ne 0 -or $head -ne $SourceSha.ToLowerInvariant()) { throw 'SourceSha must equal the actual checkout HEAD.' }
    $dirty = @(& git status --porcelain=v1 --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Git status failed.' }
    if ($Mode -eq 'Production' -and $dirty.Count) { throw 'Production requires a finalized clean source commit. Commit authorized source/docs first; build output stays ignored.' }
} finally { Pop-Location }
if ($Mode -eq 'Test' -and -not $TestId) { throw 'Test mode requires an explicit private TestId.' }
$config = Get-Content -LiteralPath (Join-Path $desktopRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = [string]$config.version
if ($version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw 'Unsafe version label.' }
$imports = @(& $dumpbin /DEPENDENTS $source 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'Release dependency inspection failed.' }
$headers = @(& $dumpbin /HEADERS $source 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'Release PE inspection failed.' }
if (($imports -join "`n") -match '(?i)VCRUNTIME|MSVCP|api-ms-win-crt|python[0-9]*\.dll|WebView2Loader\.dll') { throw 'Release still imports a dynamic CRT/Python/WebView2 loader; inspect static build first.' }
if (($headers -join "`n") -notmatch '8664 machine \(x64\)' -or ($headers -join "`n") -notmatch '2 subsystem \(Windows GUI\)') { throw 'Expected a native x64 Windows GUI executable.' }
$binaryBytes = [IO.File]::ReadAllBytes($source)
$privatePathPattern = [regex]::Escape($env:USERPROFILE)
$privateBuildPathOccurrences = [regex]::Matches([Text.Encoding]::ASCII.GetString($binaryBytes), $privatePathPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase).Count + [regex]::Matches([Text.Encoding]::Unicode.GetString($binaryBytes), $privatePathPattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase).Count
if ($Mode -eq 'Production' -and $privateBuildPathOccurrences) { throw 'Production EXE retains private user build/cache paths. Rebuild Rust with --remap-path-prefix before staging/publication.' }
$binaryBytes = $null

$null = New-Item -ItemType Directory -Path $stage
Copy-Item -LiteralPath $source -Destination (Join-Path $stage 'versora.exe')
Copy-Item -LiteralPath $icon -Destination (Join-Path $stage 'icon.ico')
Write-Utf8 (Join-Path $runRoot 'runtime-imports.txt') ($imports -join "`n")
Write-Utf8 (Join-Path $runRoot 'pe-headers.txt') ($headers -join "`n")
$licenseSummaryRaw = & (Join-Path $PSScriptRoot 'Export-ThirdPartyNotices.ps1') -Destination $stage -SupplementManifest $SupplementManifest
$licenseSummary = ($licenseSummaryRaw | Out-String) | ConvertFrom-Json
$metadata = [ordered]@{
    schema = 1
    version = $version
    sourceSha = $SourceSha.ToLowerInvariant()
    sourceUncommitted = ($dirty.Count -gt 0)
    buildId = $BuildId
    mode = $Mode
    target = 'x86_64-pc-windows-msvc'
    exeSha256 = $ExpectedExeSha256.ToLowerInvariant()
    exeBytes = (Get-Item -LiteralPath $source).Length
    formalIconSha256 = $iconSha
    cargoLockSha256 = $licenseSummary.cargoLockSha256
    nativeRuntime = 'Rust/Tauri static CRT and custom protocol; no Python runtime or app HTTP server bundled'
    webviewPrerequisite = 'Existing Microsoft Evergreen WebView2 Runtime; no automatic runtime download/install'
    signed = $false # Authenticode; the updater .sig is created separately after packaging.
    privateBuildPathOccurrences = $privateBuildPathOccurrences
    userDataPreserved = @('%LOCALAPPDATA%\Versora', '%LOCALAPPDATA%\com.mimateinn.versora', 'user-selected output folders', 'unknown files inside program/shortcut directories')
    livePaidProviderTranslation = 'NOT_RUN'
    realCliTranslationAndToolIsolation = 'NOT_RUN'
    updater = 'Signed self-update: packages run only after minisign verification against the compiled updater key; builds without a key offer the manual release page; no automatic rollback'
    thirdPartySources = $licenseSummary
}
Write-Utf8 (Join-Path $stage 'RELEASE-METADATA.json') ($metadata | ConvertTo-Json -Depth 9)
$readme = @"
Versora $version - Windows x64 native desktop

Source SHA: $($SourceSha.ToLowerInvariant())
Install location: %LOCALAPPDATA%\Programs\Versora (per-user; no administrator installation).
Launch: Start Menu > Versora. The installer uses the formal native icon.
An existing Microsoft Evergreen WebView2 Runtime is required. This package does not download or install a runtime and does not bundle Python or an app server.
API credentials and application data stay outside the program directory. Uninstall preserves the entire app data/cache directories, outputs and unknown files.
This package has no Authenticode signature. Paid provider translation, real installed CLI translation and complete CLI tool isolation have not been verified; no keys or provider CLI executables are bundled.
Provider cancellation stops local waiting/native managed processes; it does not recall requests already accepted or billable by an external provider.
Automatic updates download the signed installer from the official GitHub release, verify its signature against the publisher key built into Versora and install it when you restart or quit. Builds without a publisher key only check for updates and open the official release page. There is no automatic rollback.
Third-party notices and unchanged locked source archives are included. No project license is inferred from dependency licensing.
"@
if ($Mode -eq 'Test') { $readme = "ISOLATED TEST ARTIFACT. DO NOT PUBLISH OR REPOINT THE FORMAL ENTRY.`nSource may be uncommitted; the EXE digest is pinned.`n`n" + $readme }
Write-Utf8 (Join-Path $stage 'README-Windows.txt') $readme
$names = @('versora.exe', 'icon.ico', 'README-Windows.txt', 'RELEASE-METADATA.json', 'THIRD-PARTY-NOTICES.txt', 'DEPENDENCIES.json', 'SOURCE-LICENSES.txt', 'Rust-COPYRIGHT-library.html', 'font-OFL.txt', 'Libron-LICENSE.txt', 'Libron-COPYRIGHT.txt', 'Libron-SOURCE.json', 'third-party-sources.zip')
$fileRecords = @($names | ForEach-Object { $path = Join-Path $stage $_; [ordered]@{ name = $_; bytes = (Get-Item -LiteralPath $path).Length; sha256 = Digest $path } })
Write-Utf8 (Join-Path $stage 'payload-manifest.json') ([ordered]@{ schema = 1; sourceSha = $metadata.sourceSha; files = $fileRecords; selfExcludedFromHashList = 'payload-manifest.json' } | ConvertTo-Json -Depth 6)
$names += 'payload-manifest.json'
if (Compare-Object @($names | Sort-Object) @(Get-ChildItem -LiteralPath $stage -File | Select-Object -ExpandProperty Name | Sort-Object)) { throw 'Staging directory differs from explicit complete payload allowlist.' }
if ((Digest (Join-Path $stage 'versora.exe')) -ne $ExpectedExeSha256.ToLowerInvariant() -or (Digest $source) -ne $ExpectedExeSha256.ToLowerInvariant()) { throw 'EXE changed during staging; no installer is compiled.' }

# Generate literal enumerated NSIS actions. No wildcard File, recursive removal,
# external destination or runtime-parsed deletion manifest enters install logic.
$transactionNames = @($names + @('uninstall.exe', '.versora-installation'))
$include = [Text.StringBuilder]::new()
for ($i = 0; $i -lt $transactionNames.Count; $i++) { $null = $include.AppendLine("Var Committed$i") }
$payloadKiB = [math]::Ceiling((Get-ChildItem -LiteralPath $stage -File | Measure-Object -Property Length -Sum).Sum / 1024)
$null = $include.AppendLine("!define PAYLOAD_KIB $payloadKiB")
$null = $include.AppendLine('!macro ValidatePayloadPaths PREFIX ROOT')
foreach ($name in $transactionNames) { $null = $include.AppendLine('  !insertmacro ValidateFilePath "${PREFIX}" "${ROOT}\' + $name + '"') }
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro RefuseUnownedPayload')
foreach ($name in $transactionNames) { $null = $include.AppendLine('  IfFileExists "$INSTDIR\' + $name + '" unowned_collision') }
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro StagePayload')
$null = $include.AppendLine('  SetOutPath "$StageDir"')
foreach ($name in $names) { $null = $include.AppendLine('  File /oname=' + $name + ' "${STAGE_DIR}\' + $name + '"') }
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro BackupPayload')
for ($i = 0; $i -lt $transactionNames.Count; $i++) {
    $name = $transactionNames[$i]
    $null = $include.AppendLine('  StrCpy $Committed' + $i + ' 0')
    $null = $include.AppendLine('  IfFileExists "$INSTDIR\' + $name + '" 0 backup_next_' + $i)
    $null = $include.AppendLine('  ClearErrors')
    $null = $include.AppendLine('  Rename "$INSTDIR\' + $name + '" "$BackupDir\' + $name + '"')
    $null = $include.AppendLine('  IfErrors rollback')
    $null = $include.AppendLine('backup_next_' + $i + ':')
}
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro CommitPayload')
for ($i = 0; $i -lt $transactionNames.Count; $i++) {
    $name = $transactionNames[$i]
    $null = $include.AppendLine('  ClearErrors')
    $null = $include.AppendLine('  Rename "$StageDir\' + $name + '" "$INSTDIR\' + $name + '"')
    $null = $include.AppendLine('  IfErrors rollback')
    $null = $include.AppendLine('  StrCpy $Committed' + $i + ' 1')
}
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro RollbackPayload')
$null = $include.AppendLine('  !insertmacro ValidatePayloadPaths "" "$INSTDIR"')
$null = $include.AppendLine('  !insertmacro ValidatePayloadPaths "" "$BackupDir"')
$null = $include.AppendLine('  StrCpy $RollbackFailed 0')
for ($i = 0; $i -lt $transactionNames.Count; $i++) {
    $name = $transactionNames[$i]
    $null = $include.AppendLine('  ${If} $Committed' + $i + ' == 1')
    $null = $include.AppendLine('    ClearErrors')
    $null = $include.AppendLine('    Delete "$INSTDIR\' + $name + '"')
    $null = $include.AppendLine('    ${If} ${Errors}')
    $null = $include.AppendLine('      StrCpy $RollbackFailed 1')
    $null = $include.AppendLine('    ${EndIf}')
    $null = $include.AppendLine('  ${EndIf}')
    $null = $include.AppendLine('  IfFileExists "$BackupDir\' + $name + '" 0 restore_next_' + $i)
    $null = $include.AppendLine('  ClearErrors')
    $null = $include.AppendLine('  Rename "$BackupDir\' + $name + '" "$INSTDIR\' + $name + '"')
    $null = $include.AppendLine('  ${If} ${Errors}')
    $null = $include.AppendLine('    StrCpy $RollbackFailed 1')
    $null = $include.AppendLine('  ${EndIf}')
    $null = $include.AppendLine('restore_next_' + $i + ':')
}
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro CleanupStage')
$null = $include.AppendLine('  !insertmacro ValidatePayloadPaths "" "$StageDir"')
foreach ($name in $transactionNames) { $null = $include.AppendLine('  Delete "$StageDir\' + $name + '"') }
$null = $include.AppendLine('  RMDir "$StageDir"')
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro CleanupTransaction')
$null = $include.AppendLine('  !insertmacro CleanupStage')
$null = $include.AppendLine('  !insertmacro ValidatePayloadPaths "" "$BackupDir"')
foreach ($name in $transactionNames) { $null = $include.AppendLine('  Delete "$BackupDir\' + $name + '"') }
$null = $include.AppendLine('  RMDir "$BackupDir"')
$null = $include.AppendLine('!macroend')
$null = $include.AppendLine('!macro RemovePayload')
foreach ($name in $names) {
    $null = $include.AppendLine('  ClearErrors')
    $null = $include.AppendLine('  Delete "$INSTDIR\' + $name + '"')
    $null = $include.AppendLine('  IfErrors remove_failed')
}
$null = $include.AppendLine('!macroend')
$includePath = Join-Path $runRoot 'payload.nsh'
Write-Utf8 $includePath $include.ToString()
$fileName = if ($Mode -eq 'Production') { "Versora-$version-win32-x64-setup.exe" } else { "Versora-$version-ISOLATED-TEST-$BuildId-setup.exe" }
$output = Join-Path $runRoot $fileName
$testRoot = if ($Mode -eq 'Test') { Join-Path $PSScriptRoot "output\production-tests\$TestId" } else { $null }
if ($testRoot) { $null = Assert-NoReparse $testRoot }
$testMode = if ($Mode -eq 'Test') { 1 } else { 0 }
$arguments = @('/V3', "/DTEST_MODE=$testMode", "/DSTAGE_DIR=$stage", "/DPAYLOAD_INCLUDE=$includePath", "/DBUILD_ID=$BuildId", "/DAPP_VERSION=$version", "/DOUT_FILE=$output")
if ($Mode -eq 'Test') { $arguments += @("/DTEST_ROOT=$testRoot", "/DTEST_ID=$TestId") }
$arguments += $nsi
$log = & $compiler @arguments 2>&1
$exitCode = $LASTEXITCODE
Write-Utf8 (Join-Path $runRoot 'nsis-build.log') ($log -join "`n")
if ($exitCode -ne 0) { throw "NSIS compilation failed ($exitCode). Retained evidence: $runRoot" }
$payload = @($names | ForEach-Object { $path = Join-Path $stage $_; [ordered]@{ name = $_; bytes = (Get-Item -LiteralPath $path).Length; sha256 = Digest $path } })
$plan = [ordered]@{ schema = 2; mode = $Mode; version = $version; buildId = $BuildId; testId = $TestId; sourceSha = $metadata.sourceSha; sourceUncommitted = $metadata.sourceUncommitted; exeSha256 = $metadata.exeSha256; installationExecuted = $false; publicationAllowed = ($Mode -eq 'Production'); installer = $output; installerBytes = (Get-Item -LiteralPath $output).Length; installerSha256 = Digest $output; stage = $stage; installTarget = $(if ($testRoot) { Join-Path $testRoot 'installed' } else { '%LOCALAPPDATA%\Programs\Versora' }); shortcut = $(if ($testRoot) { Join-Path $testRoot 'integration\Versora.lnk' } else { '%APPDATA%\Microsoft\Windows\Start Menu\Programs\Versora\Versora.lnk' }); registryHive = 'HKCU'; registryView = 64; registryKey = $(if ($testRoot) { "Software\mimateinn\Versora\PackagingTests\$TestId" } else { 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Versora' }); testRoot = $testRoot; payload = $payload; runtimeDownloads = @(); associations = @(); standardAppDataWritesByInstaller = @(); nsisScriptSha256 = Digest $nsi; notices = $licenseSummary }
Write-Utf8 (Join-Path $runRoot 'build-plan.json') ($plan | ConvertTo-Json -Depth 10)
Write-Utf8 (Join-Path $runRoot 'SHA256SUMS.txt') ((Digest $output) + '  ' + $fileName + "`n")
[pscustomobject]@{ Plan = (Join-Path $runRoot 'build-plan.json'); Installer = $output; SHA256 = $plan.installerSha256; Mode = $Mode; Installed = $false } | ConvertTo-Json -Compress

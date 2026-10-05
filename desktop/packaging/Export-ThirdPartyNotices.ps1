[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$SupplementManifest = (Join-Path $PSScriptRoot 'notices\provenance.json')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$desktopRoot = Split-Path $PSScriptRoot -Parent
$fontRoot = Join-Path $desktopRoot 'assets\fonts'
$font = Join-Path $fontRoot 'NotoSansCJKtc-Regular.otf'
$ofl = Join-Path $fontRoot 'OFL.txt'
$fontSource = Join-Path $fontRoot 'SOURCE.json'
$libronRoot = Join-Path $fontRoot 'libron'
$libronUiRoot = Join-Path $desktopRoot 'ui\assets\fonts\libron'
$libronSourcePath = Join-Path $libronRoot 'SOURCE.json'
$fontSha = 'dce08bd4fd91aa8aa76ed8fea4b694c2dfb8550f67871e326843212ddbeb88b4'
$oflSha = '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2'
$rustRoot = Join-Path $env:USERPROFILE '.rustup\toolchains\1.97.1-x86_64-pc-windows-msvc'
$rustCopyright = Join-Path $rustRoot 'share\doc\rust\COPYRIGHT-library.html'
$nsisCopying = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\COPYING'
function Digest([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false)) }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.Formats.Tar
function Archive-NoticeDigests([string]$Archive, [string]$Prefix) {
    # Read only the checksum-pinned archive; never extract paths into a checkout.
    $digests = @{}
    $sourceStream = [IO.File]::OpenRead($Archive)
    $gzip = [IO.Compression.GZipStream]::new($sourceStream, [IO.Compression.CompressionMode]::Decompress)
    $reader = [System.Formats.Tar.TarReader]::new($gzip)
    try {
        while ($entry = $reader.GetNextEntry($false)) {
            if ($entry.DataStream -and $entry.Length -le 2097152 -and $entry.Name.StartsWith($Prefix, [StringComparison]::Ordinal) -and [IO.Path]::GetFileName($entry.Name) -match '(^|[-_.])(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([-_.]|$)') {
                $relative = $entry.Name.Substring($Prefix.Length)
                if ($digests.ContainsKey($relative)) { throw 'Repeated published notice path refused.' }
                $sha = [Security.Cryptography.SHA256]::Create()
                try { $digests[$relative] = [Convert]::ToHexString($sha.ComputeHash($entry.DataStream)).ToLowerInvariant() } finally { $sha.Dispose() }
            }
        }
    } finally { $reader.Dispose(); $gzip.Dispose(); $sourceStream.Dispose() }
    return $digests
}

# Supplemental texts are exact upstream documents acquired/reviewed by root.
# This exporter is intentionally offline and has no download fallback.
if (-not (Test-Path -LiteralPath $SupplementManifest -PathType Leaf)) { throw 'Reviewed notice provenance.json is required; no notice download is attempted.' }
$supplement = Get-Content -LiteralPath $SupplementManifest -Raw | ConvertFrom-Json
if ($supplement.schema -ne 1) { throw 'Unsupported notice provenance schema.' }
$supplementRoot = Split-Path ([IO.Path]::GetFullPath($SupplementManifest)) -Parent
$extra = @{}
foreach ($entry in $supplement.entries) {
    if ($entry.id -notmatch '^[a-z0-9-]+$' -or $entry.sha256 -notmatch '^[a-fA-F0-9]{64}$' -or $entry.source -notmatch '^https://') { throw 'Invalid supplemental notice provenance.' }
    if ([IO.Path]::GetFileName($entry.file) -ne $entry.file) { throw 'Supplement paths must be flat reviewed filenames.' }
    $path = Join-Path $supplementRoot $entry.file
    if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Digest $path) -ne $entry.sha256.ToLowerInvariant()) { throw "Supplemental text digest differs: $($entry.id)" }
    if ($extra.ContainsKey($entry.id)) { throw 'Duplicate supplemental notice ID.' }
    $extra[$entry.id] = @{ entry = $entry; path = $path }
}
foreach ($required in @('microsoft-webview2-sdk', 'rust-compiler-builtins', 'mit-standard-terms')) {
    if (-not $extra.ContainsKey($required)) { throw "Reviewed notice is required: $required" }
}
foreach ($required in @($font, $ofl, $fontSource, $rustCopyright, $nsisCopying)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required cached notice/resource is absent: $required" }
}
if ((Digest $font) -ne $fontSha -or (Digest $ofl) -ne $oflSha) { throw 'Approved Noto font/OFL digest differs.' }
if ((Digest $libronSourcePath) -ne '587390eb57da726c0629152c6206847e912218b813268251e3b95b7f424df547') { throw 'Approved Libron source provenance differs.' }
$libronSource = Get-Content -LiteralPath $libronSourcePath -Raw | ConvertFrom-Json
if ($libronSource.family -ne 'Libron' -or $libronSource.version -ne '0.25' -or $libronSource.sourceCommit -ne '46cf11c80a3b3a07c5b509b79d14c0e38feb1b35' -or $libronSource.license -ne 'OFL-1.1' -or $libronSource.modifiedFontBytes -or $libronSource.systemInstalled -or $libronSource.upstreamBuildScriptsExecuted) { throw 'Unexpected Libron version/license/admission facts.' }
$libronNames = @('COPYRIGHT', 'LICENSE', 'VERSION', 'ttf/Libron-Regular.ttf', 'ttf/Libron-Italic.ttf', 'ttf/Libron-Bold.ttf', 'ttf/Libron-BoldItalic.ttf', 'woff2/Libron-Regular.woff2', 'woff2/Libron-Italic.woff2', 'woff2/Libron-Bold.woff2', 'woff2/Libron-BoldItalic.woff2')
foreach ($name in $libronNames) {
    $record = @($libronSource.files | Where-Object path -eq $name)
    $path = Join-Path $libronRoot $name
    if ($record.Count -ne 1 -or -not (Test-Path -LiteralPath $path -PathType Leaf) -or (Digest $path) -ne $record[0].sha256 -or (Get-Item -LiteralPath $path).Length -ne $record[0].bytes) { throw "Approved Libron file differs: $name" }
    if ($name -like 'woff2/*') {
        $uiPath = Join-Path $libronUiRoot ([IO.Path]::GetFileName($name))
        if (-not (Test-Path -LiteralPath $uiPath -PathType Leaf) -or (Digest $uiPath) -ne $record[0].sha256) { throw "Embedded UI Libron face differs: $name" }
    }
}

Push-Location $desktopRoot
try {
    $raw = & cargo +1.97.1-x86_64-pc-windows-msvc metadata --offline --locked --filter-platform x86_64-pc-windows-msvc --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Locked offline Cargo metadata failed; no dependencies are downloaded.' }
    $metadata = ($raw -join "`n") | ConvertFrom-Json
} finally { Pop-Location }
$lockPath = Join-Path $desktopRoot 'Cargo.lock'
$checksums = @{}
foreach ($section in ([IO.File]::ReadAllText($lockPath) -split '(?m)^\[\[package\]\]')) {
    $name = [regex]::Match($section, '(?m)^name = "([^"\r\n]+)"\r?$').Groups[1].Value
    $version = [regex]::Match($section, '(?m)^version = "([^"\r\n]+)"\r?$').Groups[1].Value
    $sum = [regex]::Match($section, '(?m)^checksum = "([a-f0-9]{64})"\r?$').Groups[1].Value
    if ($name -and $version -and $sum) { $checksums["$name@$version"] = $sum }
}
$packages = @{}
foreach ($item in $metadata.packages) { $packages[$item.id] = $item }
$nodes = @{}
foreach ($item in $metadata.resolve.nodes) { $nodes[$item.id] = $item }
$rootPackage = $metadata.packages | Where-Object name -eq 'versora-desktop'
$seen = [Collections.Generic.HashSet[string]]::new()
$pending = [Collections.Generic.Queue[string]]::new()
$pending.Enqueue($rootPackage.id)
while ($pending.Count) {
    $id = $pending.Dequeue()
    if (-not $seen.Add($id)) { continue }
    foreach ($dep in $nodes[$id].deps) {
        if (@($dep.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count) { $pending.Enqueue($dep.pkg) }
    }
}
$registry = @($seen | ForEach-Object { $packages[$_] } | Where-Object { [string]$_.source -like 'registry+*' } | Sort-Object name, version)
$notices = [Text.StringBuilder]::new()
$null = $notices.AppendLine('VERSORA THIRD-PARTY NOTICES')
$null = $notices.AppendLine('This document preserves cached upstream notice texts and separately reviewed runtime/font notices. No project license is granted by these third-party licenses.')
$null = $notices.AppendLine('The dependency inventory preserves each declared SPDX expression. Every selected registry package is also supplied as its unmodified, Cargo.lock-checksummed .crate source archive in third-party-sources.zip, including original copyright headers and source license references.')
$null = $notices.AppendLine('For packages omitting a standalone license in their published archive, shared license terms are explicitly identified below. Package authors are reported as Cargo metadata, not invented copyright holders; original source notices remain in the corresponding archive.')
$inventory = [Collections.Generic.List[object]]::new()
$archives = [Collections.Generic.List[object]]::new()
$sharedMpl = $null
foreach ($package in $registry) {
    $root = Split-Path $package.manifest_path -Parent
    $indexRoot = Split-Path $root -Parent
    $registryRoot = Split-Path (Split-Path $indexRoot -Parent) -Parent
    $cache = Join-Path $registryRoot ("cache\" + (Split-Path $indexRoot -Leaf))
    $archive = Join-Path $cache "$($package.name)-$($package.version).crate"
    $key = "$($package.name)@$($package.version)"
    if (-not $checksums.ContainsKey($key) -or -not (Test-Path -LiteralPath $archive -PathType Leaf) -or (Digest $archive) -ne $checksums[$key]) { throw "Unmodified locked source archive is missing or differs: $key" }
    $publishedNotices = Archive-NoticeDigests $archive "$($package.name)-$($package.version)/"
    $files = @(Get-ChildItem -LiteralPath $root -Recurse -File -Force |
        Where-Object { $_.Name -match '(^|[-_.])(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([-_.]|$)' -and $_.Length -le 2097152 } | Sort-Object FullName)
    if ($package.name -eq 'cssparser') { $sharedMpl = Join-Path $root 'LICENSE' }
    $fallback = $null
    if ($files.Count -eq 0) {
        if ($key -eq 'alloc-stdlib@0.2.4' -and $extra.ContainsKey('alloc-stdlib-bsd-3-clause')) { $fallback = 'alloc-stdlib-bsd-3-clause' }
        elseif ($key -eq 'defmt-parser@1.0.0' -and $extra.ContainsKey('defmt-parser-mit')) { $fallback = 'defmt-parser-mit' }
        elseif ($package.name -in @('webview2-com', 'webview2-com-sys', 'webview2-com-macros') -and $extra.ContainsKey('webview2-rust-bindings-mit')) { $fallback = 'webview2-rust-bindings-mit' }
        elseif ([string]$package.license -match '(^|[ ()/])MIT([ ()/]|$)') { $fallback = 'mit-standard-terms' }
        elseif ([string]$package.license -eq 'MPL-2.0') { $fallback = 'mpl-2.0-shared-terms' }
        else { throw "No reviewed shared terms cover a package without cached texts: $key / $($package.license)" }
    }
    $inventory.Add([ordered]@{ name = $package.name; version = $package.version; declaredLicense = $package.license; cargoAuthors = @($package.authors); repository = $package.repository; sourceArchive = "registry/$($package.name)-$($package.version).crate"; sourceArchiveSha256 = $checksums[$key]; cachedNoticeFiles = @($files | ForEach-Object { [IO.Path]::GetRelativePath($root, $_.FullName).Replace('\', '/') }); sharedTermsWhenStandaloneOmitted = $fallback })
    $archives.Add(@{ path = $archive; name = "registry/$($package.name)-$($package.version).crate"; bytes = (Get-Item -LiteralPath $archive).Length; sha256 = $checksums[$key] })
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath($root, $file.FullName).Replace('\', '/')
        if (-not $publishedNotices.ContainsKey($relative) -or (Digest $file.FullName) -ne $publishedNotices[$relative]) { throw "Cached notice is changed or not part of the published package: $key / $relative" }
        $null = $notices.AppendLine("`n===== $($package.name) $($package.version) / $relative =====")
        $null = $notices.AppendLine([IO.File]::ReadAllText($file.FullName))
    }
}
if (-not $sharedMpl -or -not (Test-Path -LiteralPath $sharedMpl)) { throw 'Cached Mozilla MPL-2.0 full shared terms are required.' }
$null = $notices.AppendLine("`n===== Shared MPL-2.0 terms, from cached cssparser LICENSE =====")
$null = $notices.AppendLine([IO.File]::ReadAllText($sharedMpl))
foreach ($key in @($extra.Keys | Sort-Object)) {
    $item = $extra[$key]
    $null = $notices.AppendLine("`n===== Reviewed supplemental notice: $key =====")
    $null = $notices.AppendLine("Source: $($item.entry.source)")
    $null = $notices.AppendLine("SHA256: $($item.entry.sha256.ToLowerInvariant())")
    if ($item.entry.PSObject.Properties.Name -contains 'origin') { $null = $notices.AppendLine("Provenance/coverage: $($item.entry.origin)") }
    $null = $notices.AppendLine([IO.File]::ReadAllText($item.path))
}
$null = $notices.AppendLine("`n===== Noto Sans CJK TC Regular 2.004 =====")
$null = $notices.AppendLine('Actual embedded font name-table copyright: Copyright 2014-2021 Adobe (http://www.adobe.com/). PostScript name: NotoSansCJKtc-Regular. Font is bundled, not installed into the Windows font directory.')
$null = $notices.AppendLine("Font SHA256: $fontSha")
$null = $notices.AppendLine([IO.File]::ReadAllText($ofl))
$null = $notices.AppendLine("`n===== Libron 0.25 / unchanged official static UI faces =====")
$null = $notices.AppendLine("Source: $($libronSource.releaseUrl); source commit: $($libronSource.sourceCommit)")
$null = $notices.AppendLine('The approved WOFF2 faces are embedded as local application UI assets. The four reusable TTF faces remain in the published source checkout. No font is installed into the operating system. Original upstream copyright and OFL terms follow unchanged.')
$null = $notices.AppendLine([IO.File]::ReadAllText((Join-Path $libronRoot 'COPYRIGHT')))
$null = $notices.AppendLine([IO.File]::ReadAllText((Join-Path $libronRoot 'LICENSE')))
$null = $notices.AppendLine("`n===== NSIS 3.11 COPYING; this installer uses zlib compression =====")
$null = $notices.AppendLine([IO.File]::ReadAllText($nsisCopying))

$destinationFull = [IO.Path]::GetFullPath($Destination)
$outputRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'output'))
if (-not $destinationFull.StartsWith($outputRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Notice output must stay under ignored packaging/output.' }
if (-not (Test-Path -LiteralPath $destinationFull -PathType Container)) { throw 'Caller must create the fresh staged destination.' }
$walk = $destinationFull
while ($walk) {
    if ((Test-Path -LiteralPath $walk) -and ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Notice destination has a reparse ancestor.' }
    $parent = Split-Path $walk -Parent
    if ($parent -eq $walk) { break }
    $walk = $parent
}
foreach ($name in @('THIRD-PARTY-NOTICES.txt', 'font-OFL.txt', 'Libron-LICENSE.txt', 'Libron-COPYRIGHT.txt', 'Libron-SOURCE.json', 'Rust-COPYRIGHT-library.html', 'DEPENDENCIES.json', 'third-party-sources.zip', 'SOURCE-LICENSES.txt')) {
    if (Test-Path -LiteralPath (Join-Path $destinationFull $name)) { throw 'Notice destination already contains a known output. No overwrite is attempted.' }
}
Write-Utf8 (Join-Path $destinationFull 'THIRD-PARTY-NOTICES.txt') $notices.ToString()
Copy-Item -LiteralPath $ofl -Destination (Join-Path $destinationFull 'font-OFL.txt')
Copy-Item -LiteralPath (Join-Path $libronRoot 'LICENSE') -Destination (Join-Path $destinationFull 'Libron-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $libronRoot 'COPYRIGHT') -Destination (Join-Path $destinationFull 'Libron-COPYRIGHT.txt')
Copy-Item -LiteralPath $libronSourcePath -Destination (Join-Path $destinationFull 'Libron-SOURCE.json')
Copy-Item -LiteralPath $rustCopyright -Destination (Join-Path $destinationFull 'Rust-COPYRIGHT-library.html')
Write-Utf8 (Join-Path $destinationFull 'DEPENDENCIES.json') ([ordered]@{ schema = 1; scope = 'Conservative locked Windows normal/build metadata traversal; includes build-time packages for source/notice completeness'; cargoLockSha256 = Digest $lockPath; registryPackageCount = $inventory.Count; packages = $inventory.ToArray(); fontSource = (Get-Content -LiteralPath $fontSource -Raw | ConvertFrom-Json); libronSource = $libronSource; supplementalNotices = @($supplement.entries) } | ConvertTo-Json -Depth 12)
$zipPath = Join-Path $destinationFull 'third-party-sources.zip'
$zip = [IO.Compression.ZipFile]::Open($zipPath, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($archive in $archives) {
        $entry = $zip.CreateEntry($archive.name, [IO.Compression.CompressionLevel]::NoCompression)
        $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
        $input = [IO.File]::OpenRead($archive.path)
        $stream = $entry.Open()
        try { $input.CopyTo($stream) } finally { $stream.Dispose(); $input.Dispose() }
    }
} finally { $zip.Dispose() }
$sourceNotice = @"
Corresponding unmodified third-party source is included in third-party-sources.zip.
Each registry/*.crate file is the original crates.io source archive and its SHA256 matches Cargo.lock, as recorded in DEPENDENCIES.json.
This includes all MPL-2.0 packages and all original source copyright/license headers, including packages whose archive omits a standalone LICENSE file.
Archive registry sources can be inspected with a TAR/GZIP reader. No build cache, user data, credentials, local checkout, or developer configuration is included.
Rust standard-library notices are retained in Rust-COPYRIGHT-library.html; supplemental compiler-builtins and WebView2 loader terms are in THIRD-PARTY-NOTICES.txt.
Versora's own source is identified by RELEASE-METADATA.json and its published GitHub source SHA. No additional project license is implied.
"@
Write-Utf8 (Join-Path $destinationFull 'SOURCE-LICENSES.txt') $sourceNotice
[ordered]@{ registryPackages = $inventory.Count; sourceArchives = $archives.Count; sourceArchiveBytes = ($archives | Measure-Object -Property bytes -Sum).Sum; sourcesZipSha256 = Digest $zipPath; fontSha256 = $fontSha; oflSha256 = $oflSha; libronVersion = $libronSource.version; libronSourceSha256 = Digest $libronSourcePath; libronLicenseSha256 = Digest (Join-Path $libronRoot 'LICENSE'); cargoLockSha256 = Digest $lockPath } | ConvertTo-Json -Compress

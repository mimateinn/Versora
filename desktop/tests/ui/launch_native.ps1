param(
  [Parameter(Mandatory=$true)][string]$Executable,
  [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
  [int]$DebugPort = 9234,
  [ValidateSet('text','chunks')][string]$FixtureType = 'text'
)
$ErrorActionPreference = 'Stop'
$testRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$evidenceRoot = [IO.Path]::GetFullPath($EvidenceDirectory)
if (!$evidenceRoot.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Native smoke evidence must stay inside desktop/tests/ui.'
}
$appPath = [IO.Path]::GetFullPath($Executable)
if (!(Test-Path -LiteralPath $appPath -PathType Leaf)) { throw 'Build the native executable first.' }
$uiRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\ui'))
$latestAsset = Get-ChildItem -LiteralPath $uiRoot -File -Recurse | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if ((Get-Item -LiteralPath $appPath).LastWriteTimeUtc -lt $latestAsset.LastWriteTimeUtc) {
  throw 'The embedded frontend is newer than the executable. Rebuild before native regression.'
}
$profilePath = Join-Path $evidenceRoot 'profile'
if (Test-Path -LiteralPath $profilePath) { throw 'A fresh native smoke profile is required. Use a new evidence directory.' }
New-Item -ItemType Directory -Path $profilePath -Force | Out-Null
$fixturePath = Join-Path $evidenceRoot $(if ($FixtureType -eq 'chunks') {'native-input.json'} else {'native-input.txt'})
$fixtureText = if ($FixtureType -eq 'chunks') {
  $fixtureValues = 0..5 | ForEach-Object { "Unique native document part $_`: " + ('paragraph text ' * 130) }
  [ordered]@{paragraphs=@($fixtureValues)} | ConvertTo-Json -Depth 4
} else { "Versora`r`nA desktop document test.`r`n" }
[IO.File]::WriteAllText($fixturePath, $fixtureText, [Text.UTF8Encoding]::new($false))
foreach ($credentialName in @('OPENAI_API_KEY','ANTHROPIC_API_KEY','GEMINI_API_KEY','GOOGLE_API_KEY','XAI_API_KEY','GROK_API_KEY','AZURE_OPENAI_API_KEY','OPENROUTER_API_KEY','DEEPSEEK_API_KEY','GROQ_API_KEY')) {
  [Environment]::SetEnvironmentVariable($credentialName, $null, 'Process')
}
$env:VERSORA_DATA_DIR = $profilePath
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $profilePath 'webview'
$env:SFTS_DEMO = '1'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$DebugPort --remote-debugging-address=127.0.0.1"
$launched = Start-Process -FilePath $appPath -ArgumentList @('"' + $fixturePath + '"') -WindowStyle Normal -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds(30)
do {
  Start-Sleep -Milliseconds 250
  $launched.Refresh()
  if ($launched.HasExited) { throw "Native application exited during startup: $($launched.ExitCode)" }
} while (($launched.MainWindowHandle -eq 0 -or $launched.MainWindowTitle -ne 'Versora') -and [DateTime]::UtcNow -lt $deadline)
if ($launched.MainWindowTitle -ne 'Versora' -or $launched.MainWindowHandle -eq 0) { throw 'A real Versora desktop window did not become ready.' }
$snapshot = Get-CimInstance Win32_Process
$snapshotById = @{}
foreach ($item in $snapshot) { $snapshotById[[int]$item.ProcessId]=$item }
$descendantIds = [Collections.Generic.HashSet[int]]::new()
$descendantIds.Add($launched.Id) | Out-Null
for ($round = 0; $round -lt 6; $round++) {
  foreach ($item in $snapshot) {
    $parentProcess=$snapshotById[[int]$item.ParentProcessId]
    if ($descendantIds.Contains([int]$item.ParentProcessId) -and $parentProcess -and $item.CreationDate -ge $parentProcess.CreationDate) { $descendantIds.Add([int]$item.ProcessId) | Out-Null }
  }
}
$processes = @($snapshot | Where-Object { $descendantIds.Contains([int]$_.ProcessId) } | Select-Object ProcessId,ParentProcessId,Name,CreationDate)
$proof = [ordered]@{
  executable=$appPath; executableSHA256=(Get-FileHash -LiteralPath $appPath -Algorithm SHA256).Hash; processId=$launched.Id; windowTitle=$launched.MainWindowTitle;
  windowHandle=$launched.MainWindowHandle.ToInt64(); debugPort=$DebugPort;
  dataDir=$profilePath; webviewDataDir=$env:WEBVIEW2_USER_DATA_FOLDER; fixture=$fixturePath; demo='EXPLICIT_OFFLINE_TEST_ONLY';
  processes=$processes; pythonIsDevelopmentHarnessOnly=$true;
}
$proof | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'native-process-proof.json') -Encoding UTF8
$proof | ConvertTo-Json -Depth 5

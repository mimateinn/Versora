param(
  [Parameter(Mandatory)][string]$Executable,
  [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$ExpectedExecutableSha256,
  [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{40}$')][string]$SourceSha,
  [Parameter(Mandatory)][ValidatePattern('^(baseline|after)-r[1-9][0-9]*$')][string]$RunId,
  [ValidateRange(9251,9259)][int]$DebugPort=9251
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$desktop=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$exe=[IO.Path]::GetFullPath($Executable)
$evidence=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot ('evidence\progress-segments-'+$RunId)))
if(!$exe.StartsWith($desktop+'\',[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($exe) -ne 'versora.exe'){throw 'Use an exact checkout-owned native executable'}
if((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $ExpectedExecutableSha256){throw 'Native executable hash differs'}
if(Test-Path -LiteralPath $evidence){throw 'Retain the previous native checkpoint; increment RunId'}
if(Get-NetTCPConnection -LocalPort $DebugPort -State Listen -ErrorAction SilentlyContinue){throw 'Private loopback port already in use'}
if($RunId.StartsWith('after-')){
  $head=(& git -C (Split-Path $desktop -Parent) rev-parse HEAD | Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or $head -ne $SourceSha){throw 'After-build source SHA must equal the actual checkout HEAD'}
}elseif($SourceSha -ne '91ba8bf8aef7746183f83406f29fe79fb2a335c9' -or $ExpectedExecutableSha256 -ne '53790965d080b85a63f9027c955ac747b21041c543f26f7209203181dd4e0c4b'){
  throw 'Baseline must use the reviewed original 91ba8bf build'
}
$profile=Join-Path $evidence 'profile'
$null=New-Item -ItemType Directory -Path $profile
$fixture=Join-Path $evidence 'native-ui-chunks.json'
$paragraphs=0..29 | ForEach-Object {"Unique native UI test part $_`: "+('paragraph text '*130)}
[IO.File]::WriteAllText($fixture,([ordered]@{paragraphs=@($paragraphs)} | ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
foreach($name in @('SFTS_DEMO','VERSORA_DEMO','VERSORA_DATA_DIR','WEBVIEW2_USER_DATA_FOLDER','WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS','WEBVIEW2_BROWSER_EXECUTABLE_FOLDER','WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER','OPENAI_API_KEY','ANTHROPIC_API_KEY','GEMINI_API_KEY','GOOGLE_API_KEY','XAI_API_KEY','GROK_API_KEY','AZURE_OPENAI_API_KEY','OPENROUTER_API_KEY','DEEPSEEK_API_KEY','GROQ_API_KEY')){Remove-Item -LiteralPath ('Env:'+$name) -ErrorAction SilentlyContinue}
$env:SFTS_DEMO='1'
$env:VERSORA_DATA_DIR=$profile
$env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $profile 'webview'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=$DebugPort --remote-debugging-address=127.0.0.1"
$app=Start-Process -FilePath $exe -ArgumentList @('"'+$fixture+'"') -WorkingDirectory (Split-Path $exe -Parent) -WindowStyle Normal -PassThru
$deadline=[datetime]::UtcNow.AddSeconds(40)
do{Start-Sleep -Milliseconds 250;$app.Refresh();if($app.HasExited){throw 'Native test exited before readiness'}}while(($app.MainWindowHandle -eq 0 -or $app.MainWindowTitle -ne 'Versora') -and [datetime]::UtcNow -lt $deadline)
if($app.MainWindowHandle -eq 0 -or $app.MainWindowTitle -ne 'Versora'){throw 'Native HWND did not become ready'}
$rootProcess=Get-CimInstance Win32_Process -Filter ('ProcessId='+$app.Id)
$proof=[ordered]@{
  schema=1;sourceSha=$SourceSha;executable=$exe;executableSHA256=$ExpectedExecutableSha256;
  processId=$app.Id;windowHandle=$app.MainWindowHandle.ToInt64();windowTitle=$app.MainWindowTitle;
  debugPort=$DebugPort;dataDir=$profile;fixture=$fixture;demo='EXPLICIT_OFFLINE_TEST_ONLY';
  ownerProfileUsed=$false;processes=@([ordered]@{ProcessId=$app.Id;ParentProcessId=$rootProcess.ParentProcessId;Name=$rootProcess.Name;CreationDate=$app.StartTime.ToUniversalTime().ToString('o')})
}
[IO.File]::WriteAllText((Join-Path $evidence 'launch-proof.json'),($proof|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
$proof|ConvertTo-Json -Depth 6

param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [Parameter(Mandatory=$true)][string]$EvidencePath,
  [string]$SyntheticTarget,
  [switch]$ConfirmSynthetic
)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$testRoot=[IO.Path]::GetFullPath($PSScriptRoot)
$evidence=[IO.Path]::GetFullPath($EvidencePath)
if(!$evidence.StartsWith($testRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $evidence)) { throw 'Evidence outside test root or already exists.' }
$app=Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if($app.ProcessName -ne 'versora' -or !$app.Path.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Not owned native Versora.' }
$pidCondition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$ProcessId)
$parents=@([System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$pidCondition) | Where-Object {$_.Current.ClassName -eq '#32770'})
if($parents.Count -ne 1 -or $parents[0].Current.IsEnabled) { throw 'Owned blocked Save As parent not unique.' }
$warnings=@($parents[0].FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object {$_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Window -and $_.Current.ClassName -eq '#32770' -and $_.Current.IsEnabled})
if($warnings.Count -ne 1) { throw 'Owned nested confirmation not unique.' }
$warning=$warnings[0]
$target=$null
$targetHash=$null
if($ConfirmSynthetic) {
  $target=[IO.Path]::GetFullPath($SyntheticTarget)
  $syntheticRoot=Join-Path $testRoot 'evidence\native-r9'
  if(!$target.StartsWith($syntheticRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or !(Test-Path -LiteralPath $target -PathType Leaf)) { throw 'Confirmation permitted only for existing synthetic r9 fixture.' }
  $fixture=Get-Content -LiteralPath (Join-Path $syntheticRoot 'save-fixtures.json') -Raw | ConvertFrom-Json
  if($target -cne $fixture.preexisting) { throw 'Target is not the explicitly registered synthetic collision fixture.' }
  $targetHash=(Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
  if($targetHash -ne $fixture.preexistingSHA256) { throw 'Synthetic existing file changed before confirmation.' }
  $address=@($parents[0].FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object {$_.Current.AutomationId -eq '1001' -and $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::ToolBar})
  if($address.Count -ne 1 -or !$address[0].Current.Name.EndsWith((Split-Path -Parent $target),[StringComparison]::OrdinalIgnoreCase)) { throw 'Actual folder does not match precise synthetic collision target.' }
  $texts=@($warning.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object {$_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Text})
  if(!($texts | Where-Object {$_.Current.Name.Contains([IO.Path]::GetFileName($target))})) { throw 'Overwrite prompt does not name the precise synthetic target.' }
}
$buttonName=if($ConfirmSynthetic){([string][char]0x662F)+'(Y)'}else{([string][char]0x5426)+'(N)'}
$buttons=@($warning.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object {$_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $_.Current.Name -ceq $buttonName -and $_.Current.IsEnabled})
if($buttons.Count -ne 1) { throw 'Fresh owned confirmation button not unique.' }
$proof=[ordered]@{pid=$ProcessId;exe=$app.Path;creationUTC=$app.StartTime.ToUniversalTime().ToString('o');parentHandle=$parents[0].Current.NativeWindowHandle;warningHandle=$warning.Current.NativeWindowHandle;warningTitle=$warning.Current.Name;buttonName=$buttonName;confirmSynthetic=[bool]$ConfirmSynthetic;syntheticTarget=$target;preexistingSHA256=$targetHash;timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
New-Item -ItemType Directory -Path (Split-Path -Parent $evidence) -Force | Out-Null
$proof | ConvertTo-Json | Set-Content -LiteralPath $evidence -Encoding UTF8
$buttons[0].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
[pscustomobject]@{action=if($ConfirmSynthetic){'Yes, exact authorized synthetic fixture'}else{'No'};evidence=$evidence} | ConvertTo-Json

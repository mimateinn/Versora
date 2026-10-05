param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [Parameter(Mandatory=$true)][string]$EvidencePath
)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public static class VersoraDialogRead {
  [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetWindowText(IntPtr w,StringBuilder s,int n);
  [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr w,out uint p);
}
'@
$testRoot=[IO.Path]::GetFullPath($PSScriptRoot)
$target=[IO.Path]::GetFullPath($EvidencePath)
if (!$target.StartsWith($testRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence escapes native test root.' }
if (Test-Path -LiteralPath $target) { throw 'Evidence already exists.' }
$app=Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ($app.ProcessName -ne 'versora' -or !$app.Path.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Not the owned native test app.' }
$pidCondition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$ProcessId)
$dialogs=@([System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$pidCondition) | Where-Object { $_.Current.ClassName -eq '#32770' })
if ($dialogs.Count -ne 1) { throw 'A unique owned common dialog is required.' }
$dialog=$dialogs[0]
$controls=@($dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
  $item=$_
  $valuePattern=$null
  $value=if($item.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern,[ref]$valuePattern)){$valuePattern.Current.Value}else{$null}
  $nativeText=$null
  $handle=[IntPtr]$item.Current.NativeWindowHandle
  if ($handle -ne [IntPtr]::Zero) {
    $owner=[uint32]0
    [VersoraDialogRead]::GetWindowThreadProcessId($handle,[ref]$owner) | Out-Null
    if ($owner -ne $ProcessId) { throw 'Dialog child handle ownership mismatch.' }
    $buffer=[Text.StringBuilder]::new(32768)
    [VersoraDialogRead]::GetWindowText($handle,$buffer,$buffer.Capacity) | Out-Null
    $nativeText=$buffer.ToString()
  }
  $ancestors=@()
  $parent=[System.Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($item)
  for($depth=0;$parent -and $depth -lt 8;$depth++) {
    $ancestors+=[pscustomobject]@{id=$parent.Current.AutomationId;name=$parent.Current.Name;class=$parent.Current.ClassName;hwnd=$parent.Current.NativeWindowHandle}
    if ($parent.Current.NativeWindowHandle -eq $dialog.Current.NativeWindowHandle) { break }
    $parent=[System.Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($parent)
  }
  [pscustomobject]@{id=$item.Current.AutomationId;name=$item.Current.Name;class=$item.Current.ClassName;type=$item.Current.ControlType.ProgrammaticName;hwnd=$item.Current.NativeWindowHandle;enabled=$item.Current.IsEnabled;value=$value;nativeText=$nativeText;ancestors=$ancestors}
})
$proof=[ordered]@{pid=$ProcessId;exe=$app.Path;creationUTC=$app.StartTime.ToUniversalTime().ToString('o');dialogTitle=$dialog.Current.Name;dialogHandle=$dialog.Current.NativeWindowHandle;enabled=$dialog.Current.IsEnabled;controls=$controls;timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
$proof | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $target -Encoding UTF8
[pscustomobject]@{evidence=$target;pid=$ProcessId;dialogHandle=$dialog.Current.NativeWindowHandle;editors=@($controls | Where-Object {$_.type -eq 'ControlType.Edit' -and $_.class -eq 'Edit'});addressBars=@($controls | Where-Object {$_.id -eq '1001' -and $_.type -eq 'ControlType.ToolBar'})} | ConvertTo-Json -Depth 8

param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [Parameter(Mandatory=$true)][string]$FilePath,
  [Parameter(Mandatory=$true)][string]$EvidencePath,
  [switch]$NavigateFolder,
  [switch]$NativeCharacters
)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public static class VersoraSaveControls {
  [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr w,out uint p);
  [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageW")]
  public static extern IntPtr ReadText(IntPtr w,uint m,IntPtr wp,StringBuilder lp);
  [DllImport("user32.dll",EntryPoint="SendMessageW")]
  public static extern IntPtr Message(IntPtr w,uint m,IntPtr wp,IntPtr lp);
}
'@
$testRoot=[IO.Path]::GetFullPath($PSScriptRoot)
$target=[IO.Path]::GetFullPath($FilePath)
$evidence=[IO.Path]::GetFullPath($EvidencePath)
foreach($path in @($target,$evidence)) {
  if (!$path.StartsWith($testRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Path escapes native test root.' }
}
if (Test-Path -LiteralPath $evidence) { throw 'Preserve existing evidence.' }
if ($NavigateFolder -and !(Test-Path -LiteralPath $target -PathType Container)) { throw 'Synthetic navigation directory missing.' }
if (!$NavigateFolder -and !(Test-Path -LiteralPath (Split-Path -Parent $target) -PathType Container)) { throw 'Synthetic output parent missing.' }
$app=Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if($app.ProcessName -ne 'versora' -or !$app.Path.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Not owned native Versora.' }
$started=$app.StartTime.ToUniversalTime().Ticks
$pidCondition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$ProcessId)
function FreshDialog([string]$phase) {
  $deadline=[DateTime]::UtcNow.AddSeconds(5)
  do {
    $app.Refresh()
    if($app.HasExited -or $app.StartTime.ToUniversalTime().Ticks -ne $started -or !$app.Path.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Owned app identity changed.' }
    $dialogs=@([System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$pidCondition) | Where-Object { $_.Current.ClassName -eq '#32770' })
    if($dialogs.Count -eq 1 -and $dialogs[0].Current.IsEnabled) { return $dialogs[0] }
    Start-Sleep -Milliseconds 100
  } while([DateTime]::UtcNow -lt $deadline)
  $failure=[ordered]@{phase=$phase;pid=$ProcessId;dialogCount=$dialogs.Count;dialogs=@($dialogs | ForEach-Object {[pscustomobject]@{title=$_.Current.Name;hwnd=$_.Current.NativeWindowHandle;enabled=$_.Current.IsEnabled}});status='BLOCKED_BEFORE_ACTION';timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
  $failure | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath ($evidence+'.gate-failure.json') -Encoding UTF8
  throw "Unique enabled owned Save As dialog required at $phase."
}
function FilenameEditor($dialog) {
  $hosts=@($dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object { $_.Current.AutomationId -eq 'FileNameControlHost' -and $_.Current.ClassName -eq 'AppControlHost' })
  if($hosts.Count -ne 1) { throw 'Observed FileNameControlHost is not unique.' }
  $editors=@($hosts[0].FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit -and $_.Current.ClassName -eq 'Edit' -and $_.Current.AutomationId -eq '1001' -and $_.Current.IsEnabled })
  if($editors.Count -ne 1) { throw 'Observed owned filename editor is not unique.' }
  return $editors[0]
}
function RealText($editor) {
  $handle=[IntPtr]$editor.Current.NativeWindowHandle
  $owner=[uint32]0
  [VersoraSaveControls]::GetWindowThreadProcessId($handle,[ref]$owner) | Out-Null
  if($handle -eq [IntPtr]::Zero -or $owner -ne $ProcessId) { throw 'Filename HWND ownership mismatch.' }
  $buffer=[Text.StringBuilder]::new(32768)
  [VersoraSaveControls]::ReadText($handle,0x000D,[IntPtr]$buffer.Capacity,$buffer) | Out-Null
  return $buffer.ToString()
}
$dialog=FreshDialog 'initial'
$handle=$dialog.Current.NativeWindowHandle
$title=$dialog.Current.Name
$editor=FilenameEditor $dialog
$before=RealText $editor
$entry=if($NavigateFolder){$target.TrimEnd('\')+'\'}else{[IO.Path]::GetFileName($target)}
$addresses=@($dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object { $_.Current.AutomationId -eq '1001' -and $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::ToolBar -and $_.Current.ClassName -eq 'ToolbarWindow32' })
if($addresses.Count -ne 1) { throw 'Observed owned current-folder breadcrumb is not unique.' }
$currentAddress=$addresses[0].Current.Name
if(!$NavigateFolder -and !$currentAddress.EndsWith((Split-Path -Parent $target),[StringComparison]::OrdinalIgnoreCase)) { throw 'Actual dialog folder does not match synthetic output directory.' }
$editor.SetFocus()
if($NativeCharacters) {
  # Only the freshly observed owned Edit HWND receives these character messages.
  # No global keyboard event, focus shortcut or other window is used.
  $editorHandle=[IntPtr]$editor.Current.NativeWindowHandle
  [VersoraSaveControls]::Message($editorHandle,0x00B1,[IntPtr]::Zero,[IntPtr](-1)) | Out-Null
  [VersoraSaveControls]::Message($editorHandle,0x0102,[IntPtr]8,[IntPtr]1) | Out-Null
  foreach($character in $entry.ToCharArray()) {
    [VersoraSaveControls]::Message($editorHandle,0x0102,[IntPtr]([int]$character),[IntPtr]1) | Out-Null
  }
} else {
  $editor.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($entry)
}
$dialog=FreshDialog 'after_input'
if($dialog.Current.NativeWindowHandle -ne $handle -or $dialog.Current.Name -ne $title) { throw 'Dialog identity changed after input.' }
$editor=FilenameEditor $dialog
$typed=RealText $editor
if($typed -cne $entry -or $editor.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -cne $entry) { throw 'Displayed native filename does not match scoped input.' }
$buttons=@($dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $_.Current.ClassName -eq 'Button' -and $_.Current.AutomationId -eq '1' -and $_.Current.IsEnabled })
if($buttons.Count -ne 1) { throw 'Owned Save control not unique.' }
$buttons[0].SetFocus()
$dialog=FreshDialog 'after_blur'
$editor=FilenameEditor $dialog
$committed=RealText $editor
if($committed -cne $entry) { throw 'Native filename changed on blur.' }
$proof=[ordered]@{pid=$ProcessId;exe=$app.Path;creationUTC=$app.StartTime.ToUniversalTime().ToString('o');dialogHandle=$handle;dialogTitle=$title;editorHost='FileNameControlHost';editorHandle=$editor.Current.NativeWindowHandle;inputMethod=if($NativeCharacters){'scoped owned HWND WM_CHAR'}else{'UIA ValuePattern'};beforeText=$before;typedText=$typed;committedNativeText=$committed;actualFolderBreadcrumb=$currentAddress;target=$target;navigateFolder=[bool]$NavigateFolder;blurCommit='owned Save control SetFocus';timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
New-Item -ItemType Directory -Path (Split-Path -Parent $evidence) -Force | Out-Null
$proof | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $evidence -Encoding UTF8
$buttons=@($dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,$pidCondition) | Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $_.Current.ClassName -eq 'Button' -and $_.Current.AutomationId -eq '1' -and $_.Current.IsEnabled })
if($buttons.Count -ne 1 -or $dialog.Current.NativeWindowHandle -ne $handle -or $dialog.Current.Name -ne $title) { throw 'Owned Save control changed before invocation.' }
$buttons[0].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
[pscustomobject]@{evidence=$evidence;displayedNativeTextVerified=$true;navigateFolder=[bool]$NavigateFolder;action='owned Save invoked'} | ConvertTo-Json

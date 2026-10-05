param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
  [ValidateSet('inspect','choose','cancel')][string]$Action = 'inspect',
  [string]$FilePath,
  [switch]$UseNativeButton
)
$ErrorActionPreference = 'Stop'
$testRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$evidenceRoot = [IO.Path]::GetFullPath($EvidenceDirectory)
if (!$evidenceRoot.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Dialog evidence must remain inside desktop/tests/ui.' }
$app = Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ($app.ProcessName -ne 'versora' -or !$app.Path.StartsWith($desktopRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Dialog controls must belong to the isolated native Versora test executable.' }
if ($Action -eq 'choose') {
  $targetPath = [IO.Path]::GetFullPath($FilePath)
  if (!$targetPath.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Only native test input/output paths inside desktop/tests/ui may be selected.' }
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$condition = [System.Windows.Automation.AndCondition]::new(
  [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId),
  [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
)
$deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
  $dialog = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $condition)
  if (!$dialog) { Start-Sleep -Milliseconds 200 }
} while (!$dialog -and [DateTime]::UtcNow -lt $deadline)
if (!$dialog) { throw 'No owned native Windows file dialog became available.' }
$items = $dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
$controls = @($items | ForEach-Object { [pscustomobject]@{id=$_.Current.AutomationId;name=$_.Current.Name;type=$_.Current.ControlType.ProgrammaticName;class=$_.Current.ClassName;enabled=$_.Current.IsEnabled} })
$proof = [ordered]@{processId=$ProcessId;windowTitle=$dialog.Current.Name;windowClass=$dialog.Current.ClassName;windowHandle=$dialog.Current.NativeWindowHandle;action=$Action;controls=$controls;timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
New-Item -ItemType Directory -Path $evidenceRoot -Force | Out-Null
$proofPath = Join-Path $evidenceRoot ('native-dialog-' + $dialog.Current.NativeWindowHandle + '-' + $Action + '.json')
$proof | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $proofPath -Encoding UTF8
if ($Action -eq 'cancel') {
  $cancelCondition=[System.Windows.Automation.AndCondition]::new(
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ClassNameProperty,'Button'),
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty,'2')
  )
  $cancel=$dialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$cancelCondition)
  if (!$cancel -or !$cancel.Current.IsEnabled) { throw 'Owned native file-dialog cancellation control unavailable.' }
  $cancel.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}
if ($Action -eq 'choose') {
  $nativeSaveTitle=-join @([char]0x53E6,[char]0x5B58,[char]0x65B0,[char]0x6A94)
  if ($proof.windowTitle -ceq $nativeSaveTitle -or $proof.windowTitle -match '^(?i:Save As)$') {
    # The shell caches filename state separately from a ValuePattern text change.
    # Use the focused native Save helper which verifies FileNameControlHost,
    # actual WM_GETTEXT, owned focus commit and the real folder breadcrumb.
    $parentPath=Split-Path -Parent $targetPath
    $addresses=@($controls | Where-Object {$_.id -eq '1001' -and $_.type -eq 'ControlType.ToolBar'})
    if ($addresses.Count -ne 1 -or !$addresses[0].name.EndsWith($parentPath,[StringComparison]::OrdinalIgnoreCase)) {
      & (Join-Path $PSScriptRoot 'choose_native_save.ps1') -ProcessId $ProcessId -FilePath $parentPath -EvidencePath ($proofPath+'.folder.json') -NavigateFolder -NativeCharacters | Out-Null
      $folderDeadline=[DateTime]::UtcNow.AddSeconds(5)
      do {
        $fresh=[System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children,$condition)
        $folderReady=$false
        if($fresh){
          $address=@($fresh.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.AutomationId -eq '1001' -and $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::ToolBar})
          $folderReady=$fresh.Current.IsEnabled -and $address.Count -eq 1 -and $address[0].Current.Name.EndsWith($parentPath,[StringComparison]::OrdinalIgnoreCase)
        }
        if(!$folderReady){Start-Sleep -Milliseconds 100}
      } while(!$folderReady -and [DateTime]::UtcNow -lt $folderDeadline)
      if(!$folderReady){throw 'Native Save As did not navigate to the verified synthetic folder.'}
    }
    & (Join-Path $PSScriptRoot 'choose_native_save.ps1') -ProcessId $ProcessId -FilePath $targetPath -EvidencePath ($proofPath+'.filename.json') -NativeCharacters | Out-Null
    [pscustomobject]@{proof=$proofPath;title=$proof.windowTitle;handle=$proof.windowHandle;action=$Action;filenameMethod='verified FileNameControlHost scoped WM_CHAR'} | ConvertTo-Json
    return
  }
  $editCondition = [System.Windows.Automation.AndCondition]::new(
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit),
    [System.Windows.Automation.OrCondition]::new(
      [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1001'),
      [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1148')
    )
  )
  $filename = $dialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
  if (!$filename) { throw 'The owned file dialog has no identified native filename control.' }
  $valuePattern = $filename.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
  $valuePattern.SetValue($targetPath)
  # Refresh ownership/tree after typing; no stale element is used for confirmation.
  $app.Refresh()
  if ($app.ProcessName -ne 'versora' -or !$app.Path.StartsWith($desktopRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Native app identity changed during dialog input.' }
  $freshDialog = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $condition)
  if (!$freshDialog -or $freshDialog.Current.NativeWindowHandle -ne $proof.windowHandle -or $freshDialog.Current.Name -ne $proof.windowTitle) { throw 'Native file dialog changed after input.' }
  $filename = $freshDialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
  if (!$filename -or $filename.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -ne $targetPath) { throw 'Native filename control did not accept the scoped path.' }
  $buttonCondition = [System.Windows.Automation.AndCondition]::new(
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'Button'),
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1')
  )
  $confirm = $freshDialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
  if (!$confirm -or !$confirm.Current.IsEnabled) { throw 'The native file confirmation control is not ready.' }
  $invokePattern=$null
  if (!$UseNativeButton -and $confirm.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$invokePattern)) {
    $invokePattern.Invoke()
  } else {
    # Some Windows common-dialog split buttons expose no UIA InvokePattern.
    # BM_CLICK targets the freshly observed native child button, never global input.
    Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public static class VersoraNativeDialogButton {
  [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr w,out uint processId);
  [DllImport("user32.dll")]public static extern IntPtr SendMessage(IntPtr w,uint message,IntPtr wp,IntPtr lp);
}
'@
    $buttonHandle=[IntPtr]$confirm.Current.NativeWindowHandle
    $buttonProcess=[uint32]0
    [VersoraNativeDialogButton]::GetWindowThreadProcessId($buttonHandle,[ref]$buttonProcess) | Out-Null
    if ($buttonHandle -eq [IntPtr]::Zero -or $buttonProcess -ne $ProcessId) { throw 'Native Open/Save button ownership is not proven.' }
    [VersoraNativeDialogButton]::SendMessage($buttonHandle,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
  }
}
[pscustomobject]@{proof=$proofPath;title=$proof.windowTitle;handle=$proof.windowHandle;action=$Action;controls=$controls.Count} | ConvertTo-Json

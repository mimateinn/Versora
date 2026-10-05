param([Parameter(Mandatory=$true)][int]$ProcessId,[string]$ExpectedExecutable,[Alias('ExpectedSHA256')][string]$ExpectedExecutableSHA256)
$ErrorActionPreference='Stop'
$app=Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$approvedPath=$app.Path.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase)
if($ExpectedExecutable -or $ExpectedExecutableSHA256){
  if(!$ExpectedExecutable -or !$ExpectedExecutableSHA256){throw 'Explicit installed executable proof requires both path and SHA256.'}
  $expectedPath=[IO.Path]::GetFullPath($ExpectedExecutable)
  $installedPath=[IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\Versora\versora.exe'))
  if(!$expectedPath.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase) -and $expectedPath -ne $installedPath){throw 'Explicit path is outside the checkout and exact approved production install.'}
  $approvedPath=$app.Path -eq $expectedPath -and (Get-FileHash -LiteralPath $app.Path -Algorithm SHA256).Hash -eq $ExpectedExecutableSHA256
}
if ($app.ProcessName -ne 'versora' -or !$approvedPath -or $app.MainWindowTitle -ne 'Versora') { throw 'Owned native window identity mismatch.' }
Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public static class VersoraMinimumWindow {
  [StructLayout(LayoutKind.Sequential)]public struct RECT {public int Left,Top,Right,Bottom;}
  [DllImport("user32.dll")]public static extern uint GetDpiForWindow(IntPtr w);
  [DllImport("user32.dll")]public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr w,out RECT rect);
  [DllImport("user32.dll")]public static extern bool GetClientRect(IntPtr w,out RECT rect);
  [DllImport("user32.dll")]public static extern bool SetWindowPos(IntPtr w,IntPtr z,int x,int y,int width,int height,uint flags);
}
'@
$scale=[VersoraMinimumWindow]::GetDpiForWindow($app.MainWindowHandle)/96.0
[VersoraMinimumWindow]::SetProcessDPIAware() | Out-Null
$windowRect=[VersoraMinimumWindow+RECT]::new();$clientRect=[VersoraMinimumWindow+RECT]::new()
if (![VersoraMinimumWindow]::GetWindowRect($app.MainWindowHandle,[ref]$windowRect) -or ![VersoraMinimumWindow]::GetClientRect($app.MainWindowHandle,[ref]$clientRect)) { throw 'Owned native geometry lookup failed.' }
$frameWidth=($windowRect.Right-$windowRect.Left)-($clientRect.Right-$clientRect.Left)
$frameHeight=($windowRect.Bottom-$windowRect.Top)-($clientRect.Bottom-$clientRect.Top)
if (![VersoraMinimumWindow]::SetWindowPos($app.MainWindowHandle,[IntPtr]::Zero,0,0,[int](800*$scale+$frameWidth),[int](640*$scale+$frameHeight),0x0016)) { throw 'Owned native resize failed.' }
[pscustomobject]@{pid=$ProcessId;logicalTargetWidth=800;logicalTargetHeight=640;dpiScale=$scale;resize='Actual native HWND; no browser emulation'} | ConvertTo-Json

param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [Parameter(Mandatory=$true)][string]$OutputPath,
  [string]$ExpectedExecutable,
  [Alias('ExpectedSHA256')][string]$ExpectedExecutableSHA256,
  [switch]$BringOwnedWindowForeground,
  [ValidateSet('screen','window')][string]$Mode = 'window'
)
$ErrorActionPreference = 'Stop'
$testRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$imagePath = [IO.Path]::GetFullPath($OutputPath)
if (!$imagePath.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Native screenshot must remain in desktop/tests/ui.' }
$app = Get-Process -Id $ProcessId -ErrorAction Stop
$desktopRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$approvedPath=$app.Path.StartsWith($desktopRoot + '\', [StringComparison]::OrdinalIgnoreCase)
if($ExpectedExecutable -or $ExpectedExecutableSHA256){
  if(!$ExpectedExecutable -or !$ExpectedExecutableSHA256){throw 'Explicit installed executable proof requires both path and SHA256.'}
  $expectedPath=[IO.Path]::GetFullPath($ExpectedExecutable)
  $installedPath=[IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\Versora\versora.exe'))
  if(!$expectedPath.StartsWith($desktopRoot+'\',[StringComparison]::OrdinalIgnoreCase) -and $expectedPath -ne $installedPath){throw 'Explicit path is outside the checkout and exact approved production install.'}
  $approvedPath=$app.Path -eq $expectedPath -and (Get-FileHash -LiteralPath $app.Path -Algorithm SHA256).Hash -eq $ExpectedExecutableSHA256
}
if ($app.ProcessName -ne 'versora' -or !$approvedPath -or $app.MainWindowTitle -ne 'Versora') { throw 'Owned native Versora executable/window identity mismatch.' }
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VersoraWindowCapture {
  [StructLayout(LayoutKind.Sequential)] public struct RECT {public int Left,Top,Right,Bottom;}
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd,out uint processId);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd,out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window,IntPtr hdc,uint flags);
}
'@
[VersoraWindowCapture]::SetProcessDPIAware() | Out-Null
$handle = $app.MainWindowHandle
if ($BringOwnedWindowForeground) {
  if (![VersoraWindowCapture]::SetForegroundWindow($handle)) { throw 'Windows did not grant foreground to the verified owned window.' }
  Start-Sleep -Milliseconds 200
  $app.Refresh()
  if ($app.MainWindowHandle -ne $handle -or $app.MainWindowTitle -ne 'Versora') { throw 'Owned native window changed during foreground selection.' }
}
$foreground = [VersoraWindowCapture]::GetForegroundWindow()
$foregroundProcess = [uint32]0
[VersoraWindowCapture]::GetWindowThreadProcessId($foreground,[ref]$foregroundProcess) | Out-Null
if ($Mode -eq 'screen' -and ($foreground -ne $handle -or $foregroundProcess -ne $ProcessId)) { throw 'The owned Versora window is not foreground; no unrelated window pixels were captured.' }
$rectangle = [VersoraWindowCapture+RECT]::new()
if (![VersoraWindowCapture]::GetWindowRect($handle,[ref]$rectangle)) { throw 'Native window rectangle lookup failed.' }
$width=$rectangle.Right-$rectangle.Left
$height=$rectangle.Bottom-$rectangle.Top
if ($width -lt 100 -or $height -lt 100) { throw 'Native window is minimized or has no usable pixels.' }
$bitmap = [Drawing.Bitmap]::new($width,$height)
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try {
  if ($Mode -eq 'window') {
    $hdc=$graphics.GetHdc()
    try { if (![VersoraWindowCapture]::PrintWindow($handle,$hdc,2)) { throw 'PrintWindow did not capture the owned native HWND.' } } finally {$graphics.ReleaseHdc($hdc)}
  } else {
    $graphics.CopyFromScreen($rectangle.Left,$rectangle.Top,0,0,[Drawing.Size]::new($width,$height),[Drawing.CopyPixelOperation]::SourceCopy)
    if ([VersoraWindowCapture]::GetForegroundWindow() -ne $handle) { throw 'Foreground changed during native capture; screenshot was not saved.' }
  }
  $bitmap.Save($imagePath,[Drawing.Imaging.ImageFormat]::Png)
} finally {$graphics.Dispose();$bitmap.Dispose()}
$proof=[ordered]@{pid=$ProcessId;executable=$app.Path;title=$app.MainWindowTitle;windowHandle=$handle.ToInt64();windowDpi=[VersoraWindowCapture]::GetDpiForWindow($handle);windowsScalePercent=([VersoraWindowCapture]::GetDpiForWindow($handle)/96.0*100);foregroundHandle=$foreground.ToInt64();foregroundProcess=$foregroundProcess;rectangle=@{left=$rectangle.Left;top=$rectangle.Top;width=$width;height=$height};captureMode=$Mode;capture='Actual owned native HWND pixels including titlebar and frame; no compositing or editing';image=$imagePath;sha256=(Get-FileHash -LiteralPath $imagePath -Algorithm SHA256).Hash;timestampUTC=(Get-Date).ToUniversalTime().ToString('o')}
$proof | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath ($imagePath + '.json') -Encoding UTF8
$proof | ConvertTo-Json -Depth 4

param([Parameter(Mandatory=$true)][string[]]$CargoArgs)
$ErrorActionPreference = 'Stop'
$toolset = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207'
$sdkRoot = 'C:\Program Files (x86)\Windows Kits\10'
$sdkVersion = '10.0.26100.0'
$compilerBin = Join-Path $toolset 'bin\Hostx64\x64'
$requiredHeader = Join-Path $toolset 'include\vcruntime.h'
if (-not (Test-Path -LiteralPath $requiredHeader)) { throw 'The audited existing C++ headers are missing. Do not install another toolchain automatically.' }
$env:LIB = (Join-Path $toolset 'lib\x64') + ';' + (Join-Path $sdkRoot "Lib\$sdkVersion\ucrt\x64") + ';' + (Join-Path $sdkRoot "Lib\$sdkVersion\um\x64")
$env:INCLUDE = (Join-Path $toolset 'include') + ';' + (Join-Path $sdkRoot "Include\$sdkVersion\ucrt") + ';' + (Join-Path $sdkRoot "Include\$sdkVersion\shared") + ';' + (Join-Path $sdkRoot "Include\$sdkVersion\um") + ';' + (Join-Path $sdkRoot "Include\$sdkVersion\winrt") + ';' + (Join-Path $sdkRoot "Include\$sdkVersion\cppwinrt")
$env:CC = Join-Path $compilerBin 'cl.exe'
$env:CXX = $env:CC
$env:AR = Join-Path $compilerBin 'lib.exe'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $compilerBin 'link.exe'
$env:PATH = $compilerBin + ';' + (Join-Path $sdkRoot "bin\$sdkVersion\x64") + ';' + $env:PATH
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    & cargo +1.97.1-x86_64-pc-windows-msvc @CargoArgs
    $resultCode = $LASTEXITCODE
} finally { Pop-Location }
exit $resultCode

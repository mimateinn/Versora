; Dedicated internal package pipeline. This script cannot build a public installer.
!pragma warning error all
!ifndef INTERNAL_TEST
  !error "INTERNAL_TEST=1 is required. This script is not a release installer."
!endif
!if ${INTERNAL_TEST} != 1
  !error "Only isolated internal test mode is implemented."
!endif
!ifndef STAGE_DIR
  !error "STAGE_DIR is required."
!endif
!ifndef EXPECTED_INSTALL_DIR
  !error "EXPECTED_INSTALL_DIR is required."
!endif
!ifndef OUT_FILE
  !error "OUT_FILE is required."
!endif
!ifndef RUN_TOKEN
  !error "RUN_TOKEN is required."
!endif
!ifndef APP_VERSION
  !error "APP_VERSION is required."
!endif

Unicode true
RequestExecutionLevel user
ManifestDPIAware true
SetCompressor zlib
CRCCheck on
Name "Versora internal incomplete package test"
OutFile "${OUT_FILE}"
InstallDir "${EXPECTED_INSTALL_DIR}"
Icon "${STAGE_DIR}\icon.ico"
UninstallIcon "${STAGE_DIR}\icon.ico"
ShowInstDetails show
ShowUninstDetails show
AutoCloseWindow true

!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WinVer.nsh"
!include "x64.nsh"

Page instfiles
UninstPage instfiles

; Validate every existing ancestor, including the target itself. Junctions and
; symbolic links are refused so a workspace test cannot traverse into other data.
!macro CHECK_ISOLATED_PATH PREFIX
Function ${PREFIX}CheckIsolatedPath
  GetFullPathName $0 "$INSTDIR"
  GetFullPathName $1 "${EXPECTED_INSTALL_DIR}"
  ${If} $0 != $1
    MessageBox MB_OK|MB_ICONSTOP "This internal test accepts only its compiled workspace target. /D redirection is refused." /SD IDOK
    SetErrorLevel 20
    Abort
  ${EndIf}
  StrCpy $INSTDIR $1
  StrCpy $0 $INSTDIR
  ${Do}
    System::Call 'kernel32::GetFileAttributesW(w r0) i.r1'
    ${If} $1 != -1
      IntOp $2 $1 & 0x400
      ${If} $2 != 0
        MessageBox MB_OK|MB_ICONSTOP "A reparse point occurs in the internal test target. No files were changed." /SD IDOK
        SetErrorLevel 21
        Abort
      ${EndIf}
    ${EndIf}
    StrCpy $2 $0
    ${GetParent} "$0" $0
  ${LoopUntil} $0 == ""
FunctionEnd
!macroend

!insertmacro CHECK_ISOLATED_PATH ""
!insertmacro CHECK_ISOLATED_PATH "un."

Function .onInit
  SetShellVarContext current
  Call CheckIsolatedPath
  ${IfNot} ${IsNativeAMD64}
    MessageBox MB_OK|MB_ICONSTOP "This internal payload is tested only on Windows x64." /SD IDOK
    SetErrorLevel 22
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "This internal test requires Windows 10 or later." /SD IDOK
    SetErrorLevel 23
    Abort
  ${EndIf}
  ; Read-only prerequisite detection. No browser/runtime download or install.
  SetRegView 32
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    MessageBox MB_OK|MB_ICONSTOP "An existing Microsoft Evergreen WebView2 Runtime is required. This internal test does not download or install it. Official prerequisite: https://developer.microsoft.com/microsoft-edge/webview2/" /SD IDOK
    SetErrorLevel 24
    Abort
  ${EndIf}
  ; Never overwrite an existing installation or even a previous test payload.
  IfFileExists "$INSTDIR\versora.exe" occupied
  IfFileExists "$INSTDIR\uninstall.exe" occupied
  IfFileExists "$INSTDIR\.versora-internal-token" occupied
  IfFileExists "$INSTDIR\icon.ico" occupied
  IfFileExists "$INSTDIR\INTERNAL-NOTICE.txt" occupied
  IfFileExists "$INSTDIR\payload-manifest.json" occupied
  IfFileExists "$INSTDIR\dependency-license-inventory.json" occupied
  IfFileExists "$INSTDIR\cached-license-texts.txt" occupied
  Return
occupied:
  MessageBox MB_OK|MB_ICONSTOP "An existing known payload occupies this isolated test directory. It will not be overwritten." /SD IDOK
  SetErrorLevel 25
  Abort
FunctionEnd

Section "Internal test payload"
  SetOutPath "$INSTDIR"
  ClearErrors
  File "${STAGE_DIR}\versora.exe"
  File "${STAGE_DIR}\icon.ico"
  File "${STAGE_DIR}\INTERNAL-NOTICE.txt"
  File "${STAGE_DIR}\payload-manifest.json"
  File "${STAGE_DIR}\dependency-license-inventory.json"
  File "${STAGE_DIR}\cached-license-texts.txt"
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\uninstall.exe"
  IfErrors install_failed
  FileOpen $0 "$INSTDIR\.versora-internal-token" w
  IfErrors install_failed
  FileWrite $0 "${RUN_TOKEN}"
  FileClose $0
  IfErrors install_failed
  SetErrorLevel 0
  Goto install_done
install_failed:
  ; Retain partial files for diagnosis. No recursive cleanup or user-data writes.
  SetErrorLevel 30
  Abort "The internal install failed; partial test payload has been retained."
install_done:
SectionEnd

Function un.onInit
  SetShellVarContext current
  Call un.CheckIsolatedPath
  ClearErrors
  FileOpen $0 "$INSTDIR\.versora-internal-token" r
  IfErrors invalid_token
  FileRead $0 $1
  FileClose $0
  ${If} $1 != "${RUN_TOKEN}"
    Goto invalid_token
  ${EndIf}
  Return
invalid_token:
  MessageBox MB_OK|MB_ICONSTOP "The internal installation marker does not match. No files will be removed." /SD IDOK
  SetErrorLevel 31
  Abort
FunctionEnd

Section "Uninstall"
  ; Enumerated files only. Unknown files/subdirectories, all external profiles,
  ; shortcuts, registrations and shared WebView2 are outside this operation.
  ClearErrors
  Delete "$INSTDIR\versora.exe"
  IfErrors remove_failed
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\INTERNAL-NOTICE.txt"
  Delete "$INSTDIR\payload-manifest.json"
  Delete "$INSTDIR\dependency-license-inventory.json"
  Delete "$INSTDIR\cached-license-texts.txt"
  IfErrors remove_failed
  Delete "$INSTDIR\.versora-internal-token"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ; A nonempty directory containing unknown files is intentionally retained.
  SetErrorLevel 0
  Goto remove_done
remove_failed:
  SetErrorLevel 32
  Abort "A known payload could not be removed; remaining files are preserved."
remove_done:
SectionEnd

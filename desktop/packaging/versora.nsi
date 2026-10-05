; Production and isolated-test installers use the same transaction/removal code.
!pragma warning error all
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WinVer.nsh"
!include "x64.nsh"
!include "MUI2.nsh"

!ifndef TEST_MODE
  !error "TEST_MODE must be explicitly 0 or 1."
!endif
!if ${TEST_MODE} != 0
!if ${TEST_MODE} != 1
  !error "Unsupported install mode."
!endif
!endif
!ifndef STAGE_DIR
  !error "STAGE_DIR is required."
!endif
!ifndef PAYLOAD_INCLUDE
  !error "The reviewed enumerated payload include is required."
!endif
!ifndef BUILD_ID
  !error "BUILD_ID is required."
!endif
!ifndef APP_VERSION
  !error "APP_VERSION is required."
!endif
!ifndef OUT_FILE
  !error "OUT_FILE is required."
!endif

!define APP_ID "com.mimateinn.versora"
!if ${TEST_MODE} == 1
  !ifndef TEST_ROOT
    !error "A fixed isolated TEST_ROOT is required."
  !endif
  !ifndef TEST_ID
    !error "TEST_ID is required."
  !endif
  !define INSTALL_MARKER "${APP_ID}|Test:${TEST_ID}"
  !define UNINSTALL_KEY "Software\mimateinn\Versora\PackagingTests\${TEST_ID}"
  Name "Versora ${APP_VERSION} isolated package test"
  InstallDir "${TEST_ROOT}\installed"
!else
  !define INSTALL_MARKER "${APP_ID}|Production"
  !define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Versora"
  Name "Versora ${APP_VERSION}"
  InstallDir "$LOCALAPPDATA\Programs\Versora"
!endif

Unicode true
RequestExecutionLevel user
ManifestDPIAware true
SetCompressor zlib
CRCCheck on
OutFile "${OUT_FILE}"
Icon "${STAGE_DIR}\icon.ico"
UninstallIcon "${STAGE_DIR}\icon.ico"
ShowInstDetails show
ShowUninstDetails show
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "TradChinese"

Var ExpectedDir
Var StageDir
Var BackupDir
Var ShortcutDir
Var ShortcutPath
Var InstallerMutex
Var ExistingOwned
Var RollbackFailed

; Every enumerated file is checked as a leaf and through all its ancestors.
; Unknown directories are never traversed, including unknown junctions.
!macro ValidateFilePath PREFIX PATH
  Push "${PATH}"
  Call ${PREFIX}ValidateSafePath
  System::Call 'kernel32::GetFileAttributesW(w "${PATH}") i.r1'
  ${If} $1 != -1
    IntOp $2 $1 & 0x10
    ${If} $2 != 0
      MessageBox MB_OK|MB_ICONSTOP "A known program/shortcut file is occupied by a directory. Its contents are preserved." /SD IDOK
      SetErrorLevel 36
      Abort
    ${EndIf}
  ${EndIf}
!macroend

!include "${PAYLOAD_INCLUDE}"

!macro COMMON_INIT PREFIX
Function ${PREFIX}ValidateSafePath
  Pop $0
  ${Do}
    System::Call 'kernel32::GetFileAttributesW(w r0) i.r1'
    ${If} $1 != -1
      IntOp $2 $1 & 0x400
      ${If} $2 != 0
        MessageBox MB_OK|MB_ICONSTOP "A reparse point occurs in a known program/shortcut path. Files outside the fixed target are preserved." /SD IDOK
        SetErrorLevel 21
        Abort
      ${EndIf}
    ${EndIf}
    ${GetParent} "$0" $0
  ${LoopUntil} $0 == ""
FunctionEnd

Function ${PREFIX}ValidatePayloadPaths
  !insertmacro ValidatePayloadPaths "${PREFIX}" "$INSTDIR"
  !insertmacro ValidateFilePath "${PREFIX}" "$ShortcutPath"
FunctionEnd

Function ${PREFIX}ValidateLocation
  SetShellVarContext current
  !if ${TEST_MODE} == 1
    StrCpy $ExpectedDir "${TEST_ROOT}\installed"
    StrCpy $ShortcutDir "${TEST_ROOT}\integration"
  !else
    StrCpy $ExpectedDir "$LOCALAPPDATA\Programs\Versora"
    StrCpy $ShortcutDir "$SMPROGRAMS\Versora"
  !endif
  StrCpy $ShortcutPath "$ShortcutDir\Versora.lnk"
  GetFullPathName $0 "$INSTDIR"
  GetFullPathName $ExpectedDir "$ExpectedDir"
  ${If} $0 != $ExpectedDir
    MessageBox MB_OK|MB_ICONSTOP "The installation target differs from the fixed per-user location. /D redirection is refused." /SD IDOK
    SetErrorLevel 20
    Abort
  ${EndIf}
  StrCpy $INSTDIR $ExpectedDir
  Push "$INSTDIR"
  Call ${PREFIX}ValidateSafePath
  Push "$ShortcutDir"
  Call ${PREFIX}ValidateSafePath
  Call ${PREFIX}ValidatePayloadPaths
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\${INSTALL_MARKER}") p.r0 ?e'
  Pop $1
  StrCpy $InstallerMutex $0
  ${If} $0 == 0
  ${OrIf} $1 == 183
    MessageBox MB_OK|MB_ICONSTOP "Another Versora installer/uninstaller is using this location." /SD IDOK
    SetErrorLevel 22
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function ${PREFIX}ValidateOwnership
  StrCpy $ExistingOwned 0
  IfFileExists "$INSTDIR\.versora-installation" 0 ownership_done_${PREFIX}
  ClearErrors
  FileOpen $0 "$INSTDIR\.versora-installation" r
  IfErrors ownership_bad_${PREFIX}
  FileRead $0 $1
  FileClose $0
  ${If} $1 != "${INSTALL_MARKER}"
    Goto ownership_bad_${PREFIX}
  ${EndIf}
  StrCpy $ExistingOwned 1
  Goto ownership_done_${PREFIX}
ownership_bad_${PREFIX}:
  MessageBox MB_OK|MB_ICONSTOP "An existing installation marker belongs to another target. It will not be overwritten or removed." /SD IDOK
  SetErrorLevel 23
  Abort
ownership_done_${PREFIX}:
FunctionEnd

Function ${PREFIX}CheckExecutableNotInUse
  IfFileExists "$INSTDIR\versora.exe" 0 executable_free_${PREFIX}
  ; Opening a write handle does not modify bytes. Loaded image sections and
  ; read-only files reject it; never kill the app or schedule reboot deletion.
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\versora.exe", i 0x40000000, i 7, p 0, i 3, i 0, p 0) p.r0'
  ${If} $0 == -1
    MessageBox MB_OK|MB_ICONSTOP "Close Versora before installing/removing it. The existing executable is preserved." /SD IDOK
    SetErrorLevel 24
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
executable_free_${PREFIX}:
FunctionEnd
!macroend

!insertmacro COMMON_INIT ""
!insertmacro COMMON_INIT "un."

Function .onInit
  Call ValidateLocation
  ${IfNot} ${IsNativeAMD64}
    MessageBox MB_OK|MB_ICONSTOP "This package requires Windows x64." /SD IDOK
    SetErrorLevel 25
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "This package requires Windows 10 or later." /SD IDOK
    SetErrorLevel 26
    Abort
  ${EndIf}
  SetRegView 32
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    MessageBox MB_OK|MB_ICONSTOP "Microsoft Evergreen WebView2 Runtime is required. This installer does not download or install another runtime. Official prerequisite: https://developer.microsoft.com/microsoft-edge/webview2/" /SD IDOK
    SetErrorLevel 27
    Abort
  ${EndIf}
  SetRegView 64
  Call ValidateOwnership
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} $0 != $INSTDIR
    Goto unowned_collision
  ${EndIf}
  ${If} $ExistingOwned == 0
    !insertmacro RefuseUnownedPayload
    IfFileExists "$ShortcutPath" unowned_collision
    ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
    ${If} $0 != ""
      Goto unowned_collision
    ${EndIf}
  ${EndIf}
  Call CheckExecutableNotInUse
  StrCpy $StageDir "$INSTDIR\.versora-stage-${BUILD_ID}"
  StrCpy $BackupDir "$INSTDIR\.versora-backup-${BUILD_ID}"
  IfFileExists "$StageDir\*.*" transaction_exists
  IfFileExists "$BackupDir\*.*" transaction_exists
  System::Call 'kernel32::GetFileAttributesW(w "$StageDir") i.r0'
  ${If} $0 != -1
    Goto transaction_exists
  ${EndIf}
  System::Call 'kernel32::GetFileAttributesW(w "$BackupDir") i.r0'
  ${If} $0 != -1
    Goto transaction_exists
  ${EndIf}
  Return
unowned_collision:
  MessageBox MB_OK|MB_ICONSTOP "An existing unowned program, shortcut or registration occupies this location. No files were changed." /SD IDOK
  SetErrorLevel 28
  Abort
transaction_exists:
  MessageBox MB_OK|MB_ICONSTOP "A previous transaction directory exists. Its files are preserved for recovery; no overwrite was attempted." /SD IDOK
  SetErrorLevel 29
  Abort
FunctionEnd

Section "Versora"
  Call ValidatePayloadPaths
  ClearErrors
  CreateDirectory "$StageDir"
  IfErrors stage_failed
  CreateDirectory "$BackupDir"
  IfErrors stage_failed
  ClearErrors
  !insertmacro StagePayload
  IfErrors stage_failed
  WriteUninstaller "$StageDir\uninstall.exe"
  IfErrors stage_failed
  FileOpen $0 "$StageDir\.versora-installation" w
  IfErrors stage_failed
  FileWrite $0 "${INSTALL_MARKER}"
  FileClose $0
  IfErrors stage_failed
  !insertmacro ValidatePayloadPaths "" "$StageDir"
  !insertmacro ValidatePayloadPaths "" "$BackupDir"
  Call ValidatePayloadPaths
  !insertmacro BackupPayload
  Call ValidatePayloadPaths
  !insertmacro ValidatePayloadPaths "" "$StageDir"
  !insertmacro ValidatePayloadPaths "" "$BackupDir"
  !insertmacro CommitPayload
  ; The program transaction has completed before integration is written.
  ClearErrors
  CreateDirectory "$ShortcutDir"
  CreateShortCut "$ShortcutPath" "$INSTDIR\versora.exe" "" "$INSTDIR\icon.ico" 0 SW_SHOWNORMAL "" "Versora native desktop"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Versora"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "mimateinn"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" '$\"$INSTDIR\icon.ico$\"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/mimateinn/Versora"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" ${PAYLOAD_KIB}
  IfErrors integration_failed
  !insertmacro CleanupTransaction
  SetErrorLevel 0
  Goto install_done
stage_failed:
  !insertmacro CleanupStage
  RMDir "$BackupDir"
  SetErrorLevel 30
  Abort "Staging failed. The previous program and all user data were preserved."
rollback:
  !insertmacro RollbackPayload
  ${If} $RollbackFailed != 0
    SetErrorLevel 32
    Abort "Replacement failed and a backup could not be restored. Backup files are retained in the installation directory."
  ${EndIf}
  !insertmacro CleanupTransaction
  SetErrorLevel 31
  Abort "Replacement failed. The previous program was restored; user data was preserved."
integration_failed:
  ; Leave completed program and backups intact for recovery/reinstallation.
  SetErrorLevel 33
  Abort "Program files were installed, but shortcut/registration could not be completed. Existing backups and user data were retained."
install_done:
SectionEnd

Function un.onInit
  Call un.ValidateLocation
  Call un.ValidateOwnership
  ${If} $ExistingOwned != 1
    MessageBox MB_OK|MB_ICONSTOP "A matching Versora installation marker is required before removal." /SD IDOK
    SetErrorLevel 34
    Abort
  ${EndIf}
  Call un.CheckExecutableNotInUse
FunctionEnd

Section "Uninstall"
  Call un.ValidatePayloadPaths
  ClearErrors
  !insertmacro RemovePayload
  IfErrors remove_failed
  ; Remove only this installer-owned shortcut and matching private/public entry.
  Delete "$ShortcutPath"
  RMDir "$ShortcutDir"
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 == $INSTDIR
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ${EndIf}
  Delete "$INSTDIR\.versora-installation"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ; Nonempty directories containing unknown files are deliberately retained.
  SetErrorLevel 0
  Goto remove_done
remove_failed:
  SetErrorLevel 35
  Abort "A known program file could not be removed. Remaining program files and user data were preserved."
remove_done:
SectionEnd

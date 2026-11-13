Unicode True
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "Sections.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"
Name "StickTalk"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\StickTalk"
InstallDirRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define MUI_ICON "..\..\crates\client\assets\sticktalk.ico"
!define MUI_UNICON "..\..\crates\client\assets\sticktalk.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
UninstPage custom un.DataPage un.DataLeave
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!ifdef TEST_DATA_DIR
  !ifndef TESTFIXTURE
    !error "TEST_DATA_DIR is only permitted in explicit test fixtures"
  !endif
  !define DATA_ROOT "${TEST_DATA_DIR}"
!else
  !define DATA_ROOT "$LOCALAPPDATA\StickTalk"
!endif
Var KeepData
Var KeepDataControl
Var Options
Section "StickTalk (required)" Main
  SectionIn RO
  SetRegView 64
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ClearErrors
  File /oname=sticktalk.exe "${CLIENT}"
  File /oname=sticktalk-maintenance.exe "${MAINTENANCE}"
!ifdef CRT
  ; App-local redistributables: no separate VC redist installer or administrator.
  File "${CRT}\*.dll"
!endif
  File /oname=sticktalk.ico "..\..\crates\client\assets\sticktalk.ico"
  File /oname=Inter-OFL.txt "..\..\crates\client\assets\fonts\Inter-OFL.txt"
  File /oname=JetBrainsMono-OFL.txt "..\..\crates\client\assets\fonts\JetBrainsMono-OFL.txt"
  File /oname=Lucide-LICENSE.txt "..\..\crates\client\assets\icons\LICENSE"
  File /oname=Twemoji-NOTICE.txt "..\..\crates\client\assets\emoji\README.md"
  File /oname=Twemoji-LICENSE.txt "..\..\crates\client\assets\emoji\LICENSE-GRAPHICS"
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\StickTalk.lnk" "$INSTDIR\sticktalk.exe" "" "$INSTDIR\sticktalk.ico"
  nsExec::ExecToStack '"$INSTDIR\sticktalk-maintenance.exe" shortcut "$SMPROGRAMS\StickTalk.lnk"'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_failed
  WriteRegStr HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop" "DisplayName" "StickTalk"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop" "IconUri" "$INSTDIR\sticktalk.ico"
  WriteRegStr HKCU "Software\Classes\sticktalk" "" "URL:StickTalk Protocol"
  WriteRegStr HKCU "Software\Classes\sticktalk" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\sticktalk\DefaultIcon" "" '$\"$INSTDIR\sticktalk.ico$\",0'
  WriteRegStr HKCU "Software\Classes\sticktalk\shell\open\command" "" '$\"$INSTDIR\sticktalk.exe$\" $\"%1$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DisplayName" "StickTalk"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "Publisher" "StickTalk"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DisplayIcon" "$INSTDIR\sticktalk.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "EstimatedSize" $0
  Goto install_done
install_failed:
  SetErrorLevel 1
  Abort "Close StickTalk, then run setup again."
install_done:
SectionEnd
Section /o "Desktop shortcut" Desktop
  CreateShortcut "$DESKTOP\StickTalk.lnk" "$INSTDIR\sticktalk.exe" "" "$INSTDIR\sticktalk.ico"
  nsExec::ExecToStack '"$INSTDIR\sticktalk-maintenance.exe" shortcut "$DESKTOP\StickTalk.lnk"'
  Pop $0
  Pop $1
  StrCmp $0 "0" +3
  SetErrorLevel 1
  Abort
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DesktopShortcut" 1
SectionEnd
Function .onInit
  SetRegView 64
  SetShellVarContext current
  ReadRegDWORD $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DesktopShortcut"
  ${If} $0 == 1
    !insertmacro SelectSection ${Desktop}
  ${EndIf}
  ${GetParameters} $Options
  ${GetOptions} $Options "/DESKTOP=" $0
  ${If} $0 == "1"
    !insertmacro SelectSection ${Desktop}
  ${EndIf}
FunctionEnd
Function un.onInit
  SetRegView 64
  SetShellVarContext current
  StrCpy $KeepData ${BST_CHECKED}
  ${GetParameters} $Options
  ${GetOptions} $Options "/PURGE=" $0
  ${If} $0 == "1"
    StrCpy $KeepData ${BST_UNCHECKED}
  ${EndIf}
FunctionEnd
Function un.DataPage
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateCheckbox} 0 16u 100% 28u "Keep my settings, downloads and local logs"
  Pop $KeepDataControl
  ${NSD_SetState} $KeepDataControl $KeepData
  nsDialogs::Show
FunctionEnd
Function un.DataLeave
  ${NSD_GetState} $KeepDataControl $KeepData
FunctionEnd
Section "Uninstall"
  SetRegView 64
  SetShellVarContext current
  ClearErrors
  Delete "$INSTDIR\sticktalk.exe"
  Delete "$INSTDIR\sticktalk-maintenance.exe"
  IfErrors 0 +3
  SetErrorLevel 1
  Abort "Close StickTalk before uninstalling."
  Delete "$SMPROGRAMS\StickTalk.lnk"
  Delete "$DESKTOP\StickTalk.lnk"
  Delete "$INSTDIR\concrt140.dll"
  Delete "$INSTDIR\msvcp140_1.dll"
  Delete "$INSTDIR\msvcp140_2.dll"
  Delete "$INSTDIR\msvcp140_atomic_wait.dll"
  Delete "$INSTDIR\msvcp140_codecvt_ids.dll"
  Delete "$INSTDIR\msvcp140.dll"
  Delete "$INSTDIR\vccorlib140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  Delete "$INSTDIR\vcruntime140_threads.dll"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\sticktalk.ico"
  Delete "$INSTDIR\Inter-OFL.txt"
  Delete "$INSTDIR\JetBrainsMono-OFL.txt"
  Delete "$INSTDIR\Lucide-LICENSE.txt"
  Delete "$INSTDIR\Twemoji-NOTICE.txt"
  Delete "$INSTDIR\Twemoji-LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  ${If} $KeepData == ${BST_UNCHECKED}
    ; Fixed application-owned data root, never an argument-derived deletion target.
    RMDir /r "${DATA_ROOT}"
  ${EndIf}
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "StickTalk"
  DeleteRegKey HKCU "Software\Classes\sticktalk"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk"
SectionEnd

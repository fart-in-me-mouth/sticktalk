Unicode True
!include "MUI2.nsh"
Name "StickTalk"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\StickTalk"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "StickTalk"
  SetRegView 64
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ClearErrors
  File /oname=sticktalk.exe "${CLIENT}"
  File /oname=sticktalk-maintenance.exe "${MAINTENANCE}"
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\StickTalk.lnk" "$INSTDIR\sticktalk.exe"
  CreateShortcut "$DESKTOP\StickTalk.lnk" "$INSTDIR\sticktalk.exe"
  ; Native helper writes System.AppUserModel.ID on both .lnk files. Hidden,
  ; no powershell, external NSIS plug-ins, browser, app launch or elevation.
  nsExec::ExecToStack '"$INSTDIR\sticktalk-maintenance.exe" shortcut "$SMPROGRAMS\StickTalk.lnk"'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_failed
  nsExec::ExecToStack '"$INSTDIR\sticktalk-maintenance.exe" shortcut "$DESKTOP\StickTalk.lnk"'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_failed
  WriteRegStr HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop" "DisplayName" "StickTalk"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop" "IconUri" "$INSTDIR\sticktalk.exe"
  WriteRegStr HKCU "Software\Classes\sticktalk" "" "URL:StickTalk Protocol"
  WriteRegStr HKCU "Software\Classes\sticktalk" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\sticktalk\DefaultIcon" "" '$\"$INSTDIR\sticktalk.exe$\",0'
  WriteRegStr HKCU "Software\Classes\sticktalk\shell\open\command" "" '$\"$INSTDIR\sticktalk.exe$\" $\"%1$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DisplayName" "StickTalk"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk" "NoRepair" 1
  ; Autostart is off by default. Never overwrite an existing preference on upgrade.
  Goto install_done
install_failed:
  SetErrorLevel 1
  Abort
install_done:
SectionEnd
Section "Uninstall"
  SetRegView 64
  SetShellVarContext current
  Delete "$SMPROGRAMS\StickTalk.lnk"
  Delete "$DESKTOP\StickTalk.lnk"
  Delete "$INSTDIR\sticktalk.exe"
  Delete "$INSTDIR\sticktalk-maintenance.exe"
  Delete "$INSTDIR\Uninstall.exe"
  ; Nonrecursive: retain user preferences, cache and unknown files.
  RMDir "$INSTDIR"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "StickTalk"
  DeleteRegKey HKCU "Software\Classes\sticktalk"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\StickTalk.Desktop"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk"
SectionEnd

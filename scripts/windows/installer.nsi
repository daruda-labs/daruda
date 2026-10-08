Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

Name "daruda"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\daruda"
InstallDirRegKey HKCU "Software\daruda" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show

!ifdef UNINSTALL_SIGN_COMMAND
  !uninstfinalize '${UNINSTALL_SIGN_COMMAND}' = 0
!endif

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${BUNDLE}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "daruda requires 64-bit Windows."
    Abort
  ${EndIf}
  SetShellVarContext current
FunctionEnd

!macro CheckExecutable PREFIX
  ${If} ${FileExists} "$INSTDIR\daruda.exe"
    ${PREFIX}retry:
    System::Call 'kernel32::CreateFileW(w "$INSTDIR\daruda.exe", i 0x40000000, i 1, p 0, i 3, i 0, p 0) p.r0'
    ${If} $0 == -1
      IfSilent 0 +3
      SetErrorLevel 2
      Abort
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Close daruda and its running tasks before continuing." IDRETRY ${PREFIX}retry
      Abort
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r0)'
  ${EndIf}
!macroend

Section "daruda"
  !insertmacro CheckExecutable install_
  SetOutPath "$INSTDIR"
  File /r "${BUNDLE}\*"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$SMPROGRAMS\daruda"
  CreateShortCut "$SMPROGRAMS\daruda\daruda.lnk" "$INSTDIR\daruda.exe"
  CreateShortCut "$DESKTOP\daruda.lnk" "$INSTDIR\daruda.exe"
  WriteRegStr HKCU "Software\daruda" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "DisplayName" "daruda"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "DisplayIcon" "$INSTDIR\daruda.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro CheckExecutable uninstall_
  ClearErrors
  ExecWait '"$INSTDIR\daruda.exe" --unregister-desktop' $0
  ${If} ${Errors}
  ${OrIf} $0 != 0
    IfSilent +2
    MessageBox MB_ICONEXCLAMATION "Could not remove daruda notification shortcuts. Close daruda and retry."
    SetErrorLevel 3
    Abort
  ${EndIf}
  ; Delete only the shipped manifest, preserving user-created files and data.
  !include "${UNINSTALL_MANIFEST}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\daruda\daruda.lnk"
  RMDir "$SMPROGRAMS\daruda"
  Delete "$DESKTOP\daruda.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda"
  DeleteRegKey HKCU "Software\daruda"
SectionEnd

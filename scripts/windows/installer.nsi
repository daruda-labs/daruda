Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

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
  ; The updater starts this installer before quitting, so hold a process handle.
  ${GetOptions} $CMDLINE "/WAITPID=" $0
  IfErrors wait_done
  System::Call 'kernel32::OpenProcess(i 0x100000, i 0, i r0) p.r1'
  ${If} $1 != 0
    System::Call 'kernel32::WaitForSingleObject(p r1, i 120000) i.r0'
    System::Call 'kernel32::CloseHandle(p r1)'
    ${If} $0 != 0
      MessageBox MB_ICONSTOP "daruda did not exit. Close it and run this installer again."
      SetErrorLevel 5
      Abort
    ${EndIf}
  ${EndIf}
  wait_done:
FunctionEnd

!macro CheckOwnedFile PATH PREFIX
  ${If} ${FileExists} "${PATH}"
    ${PREFIX}retry:
    System::Call 'kernel32::CreateFileW(w "${PATH}", i 0x40010000, i 7, p 0, i 3, i 0, p 0) p.r0'
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

!macro DeleteOwnedFile PATH
  ${If} ${FileExists} "${PATH}"
  ClearErrors
  Delete "${PATH}"
  ${If} ${Errors}
    IfSilent +2
    MessageBox MB_ICONSTOP "Could not remove ${PATH}. Close applications using it and retry uninstall."
    SetErrorLevel 4
    Abort
  ${EndIf}
  ${EndIf}
!macroend

!include "${FILE_CHECKS}"

Section "daruda"
  !insertmacro CheckShippedFiles install_
  !insertmacro CheckOwnedFile "$INSTDIR\uninstall.exe" install_uninstaller_
  !insertmacro CheckOwnedFile "$INSTDIR\daruda-install.ini" install_marker_
  SetOutPath "$INSTDIR"
  ClearErrors
  File /r "${BUNDLE}\*"
  ${If} ${Errors}
    SetErrorLevel 4
    Abort
  ${EndIf}
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteINIStr "$INSTDIR\daruda-install.ini" "install" "format" "1"
  WriteINIStr "$INSTDIR\daruda-install.ini" "uninstall" "cleanup_done" "0"
  ${If} ${Errors}
    SetErrorLevel 4
    Abort
  ${EndIf}
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

Function .onInstSuccess
  ${GetOptions} $CMDLINE "/RELAUNCH" $0
  IfErrors relaunch_done
  Exec '"$INSTDIR\daruda.exe"'
  relaunch_done:
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro CheckShippedFiles uninstall_
  !insertmacro CheckOwnedFile "$INSTDIR\daruda-install.ini" uninstall_marker_
  ReadINIStr $0 "$INSTDIR\daruda-install.ini" "uninstall" "cleanup_done"
  StrCmp $0 "1" cleanup_done
  ClearErrors
  ExecWait '"$INSTDIR\daruda.exe" --unregister-desktop' $0
  ${If} ${Errors}
  ${OrIf} $0 != 0
    IfSilent +2
    MessageBox MB_ICONEXCLAMATION "Could not remove daruda notification shortcuts. Close daruda and retry."
    SetErrorLevel 3
    Abort
  ${EndIf}
  WriteINIStr "$INSTDIR\daruda-install.ini" "uninstall" "cleanup_done" "1"
  ${If} ${Errors}
    SetErrorLevel 4
    Abort
  ${EndIf}
  cleanup_done:
  ; Delete only the shipped manifest, preserving user-created files and data.
  !include "${UNINSTALL_MANIFEST}"
  ; A previous install must never unregister a newer installation elsewhere.
  ReadRegStr $0 HKCU "Software\daruda" "InstallDir"
  StrCpy $2 "0"
  StrCmp $0 "" integration_files_done
  GetFullPathName $0 $0
  GetFullPathName $1 "$INSTDIR"
  StrCmp $0 $1 0 integration_files_done
  StrCpy $2 "1"
  !insertmacro DeleteOwnedFile "$SMPROGRAMS\daruda\daruda.lnk"
  RMDir "$SMPROGRAMS\daruda"
  !insertmacro DeleteOwnedFile "$DESKTOP\daruda.lnk"
  integration_files_done:
  !insertmacro DeleteOwnedFile "$INSTDIR\daruda-install.ini"
  StrCmp $2 "1" 0 integration_done
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda"
  DeleteRegKey HKCU "Software\daruda"
  integration_done:
  ; In-place test mode cannot delete the executing uninstaller itself.
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd

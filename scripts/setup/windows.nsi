Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef VERSION
  !error "VERSION is required"
!endif
!ifndef PAYLOAD
  !error "PAYLOAD is required"
!endif
!ifndef OUTPUT
  !error "OUTPUT is required"
!endif
Name "Hieronymus"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Hieronymus\app"
InstallDirRegKey HKCU "Software\Hieronymus" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Hieronymus"
VIAddVersionKey "FileDescription" "Hieronymus Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Pavel Obruchnikov"
Var ExtraOptions
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Welcome to Hieronymus"
!define MUI_WELCOMEPAGE_TEXT "Give your writing agent a memory.$\r$\n$\r$\nSetup downloads the app and its memory model, verifies them, and installs Hieronymus for your Windows account. Windows may ask you to allow installation of Microsoft's runtime if it is needed.$\r$\n$\r$\nKeep your internet connection on. The model download can take a few minutes."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TEXT "Hieronymus is installed. Open it and choose Connect your agent to add its MCP connection and skills. Then continue writing in your agent."
!define MUI_FINISHPAGE_RUN "$INSTDIR\bin\hiero.exe"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenHieronymus
!define MUI_FINISHPAGE_RUN_TEXT "Open Hieronymus"
!insertmacro MUI_PAGE_FINISH
!define MUI_UNCONFIRMPAGE_TEXT_TOP "Remove Hieronymus from this Windows account? Your project data and memories will be kept."
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Function OpenHieronymus
  Exec '"$INSTDIR\bin\hiero.exe" admin --data-root "$APPDATA\Hieronymus"'
FunctionEnd
Function .onInit
  SetShellVarContext current
  StrCpy $ExtraOptions ""
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/NOACTIVATE" $1
  ${IfNot} ${Errors}
    StrCpy $ExtraOptions "-NoActivate"
  ${EndIf}
FunctionEnd
Section "Hieronymus"
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=install.ps1 "${PAYLOAD}"
  DetailPrint "Downloading and installing Hieronymus. Please keep your internet connection on."
  ${DisableX64FSRedirection}
  nsExec::ExecToLog /TIMEOUT=1200000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\install.ps1" -AppDir "$INSTDIR" -LogPath "$LOCALAPPDATA\Hieronymus\setup.log" -NoOpen $ExtraOptions'
  Pop $0
  ${EnableX64FSRedirection}
  ${If} $0 != 0
    SetErrorLevel 1
    Abort "Setup could not finish. See the details above, check your connection, and run Setup again."
  ${EndIf}
  CreateDirectory "$LOCALAPPDATA\Hieronymus"
  WriteUninstaller "$LOCALAPPDATA\Hieronymus\Uninstall.exe"
  WriteRegStr HKCU "Software\Hieronymus" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "DisplayName" "Hieronymus"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "Publisher" "Pavel Obruchnikov"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "UninstallString" '$\"$LOCALAPPDATA\Hieronymus\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus" "NoRepair" 1
  CreateShortcut "$SMPROGRAMS\Hieronymus.lnk" "$INSTDIR\bin\hiero.exe" 'admin --data-root "$APPDATA\Hieronymus"'
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  ReadRegStr $INSTDIR HKCU "Software\Hieronymus" "InstallDir"
  ${If} $INSTDIR == ""
    Abort "The installation location could not be found. Your data has not been changed."
  ${EndIf}
  nsExec::ExecToStack /TIMEOUT=120000 '"$LOCALAPPDATA\Hieronymus\uninstall-hiero.exe" uninstall --yes --app-dir "$INSTDIR" --data-root "$APPDATA\Hieronymus"'
  Pop $0
  Pop $1
  DetailPrint $1
  FileOpen $2 "$LOCALAPPDATA\Hieronymus\uninstall.log" w
  FileWrite $2 $1
  FileClose $2
  ${If} $0 != 0
    SetErrorLevel 1
    Abort "Hieronymus could not be removed. See the details above. Your data has been kept."
  ${EndIf}
  Delete "$SMPROGRAMS\Hieronymus.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hieronymus"
  DeleteRegKey HKCU "Software\Hieronymus"
  Delete "$LOCALAPPDATA\Hieronymus\Uninstall.exe"
  Delete "$LOCALAPPDATA\Hieronymus\uninstall-hiero.exe"
SectionEnd

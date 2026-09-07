; Flectar Mail NSIS installer
;
; Build (from the repository root, within the GitHub Actions Windows job):
;   makensis -DVERSION=<semver> \
;            -DSTAGE=<absolute staged installer dir, forward slashes> \
;            -DICON=<absolute path to platform/windows/flectar-mail.ico> \
;            -DOUTDIR=<absolute output dir, forward slashes> \
;            platform\windows\flectar-mail.nsi
;
; The STAGE directory must already contain the exact files that belong inside
; Program Files (the .exe, README, LICENSE, THIRD_PARTY_NOTICES.md, plus the
; LICENSES/ and Licenses/ trees). The .ico is intentionally kept out of STAGE
; and passed separately via ICON so it is only used for the shortcuts/icons.

Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!define APP_NAME "Flectar Mail"
!define APP_ID "FlectarMail"
!define PUBLISHER "Flectar"
!define REG_APP "Software\${APP_ID}"
!define REG_UNINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!ifndef STAGE
  !error "STAGE directory must be defined (absolute path to the staged install tree)"
!endif
!ifndef ICON
  !define ICON "${STAGE}/flectar-mail.ico"
!endif
!ifndef OUTDIR
  !define OUTDIR "."
!endif

Name "${APP_NAME} ${VERSION}"
; Keep the output file name free of spaces so CI artifacts are easy to handle.
OutFile "${OUTDIR}/flectar-mail-${VERSION}-setup-x64.exe"

Icon "${ICON}"
UninstallIcon "${ICON}"

InstallDir "$PROGRAMFILES64\${APP_NAME}"
InstallDirRegKey HKLM "${REG_APP}" "InstallDir"

RequestExecutionLevel admin
SetCompressor /SOLID lzma

BrandingText "Flectar Mail"

; ---- MUI (Modern UI 2) ----
!define MUI_ABORTWARNING
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\flectar-mail.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${APP_NAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Flectar Mail" SEC_MAIN
  SetShellVarContext all
  SetRegView 64

  ; Install the app files and the license trees. File paths are enumerated
  ; explicitly so placement is deterministic across NSIS versions (the File /r
  ; switch nests the top-level directory differently in some releases).
  SetOutPath "$INSTDIR"
  File "${STAGE}/flectar-mail.exe" "${STAGE}/README.md" "${STAGE}/LICENSE" "${STAGE}/THIRD_PARTY_NOTICES.md"

  SetOutPath "$INSTDIR\LICENSES"
  File "${STAGE}/LICENSES/GPL-3.0-only.txt"
  File "${STAGE}/LICENSES/Apache-2.0.txt"

  SetOutPath "$INSTDIR\Licenses\Google Sans Flex"
  File "${STAGE}/Licenses/Google Sans Flex/OFL.txt"
  File "${STAGE}/Licenses/Google Sans Flex/README.md"

  SetOutPath "$INSTDIR\Licenses\Noto Emoji"
  File "${STAGE}/Licenses/Noto Emoji/OFL.txt"
  File "${STAGE}/Licenses/Noto Emoji/README.md"

  ; Start Menu + Desktop shortcuts.
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\flectar-mail.exe"
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\flectar-mail.exe"

  ; Uninstaller.
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\Uninstall ${APP_NAME}.lnk" "$INSTDIR\uninstall.exe"

  ; Add/Remove Programs registration.
  WriteRegStr HKLM "${REG_UNINSTALL}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${REG_UNINSTALL}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${REG_UNINSTALL}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${REG_UNINSTALL}" "DisplayIcon" "$INSTDIR\flectar-mail.exe"
  WriteRegStr HKLM "${REG_UNINSTALL}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${REG_UNINSTALL}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${REG_UNINSTALL}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${REG_UNINSTALL}" "NoModify" 1
  WriteRegDWORD HKLM "${REG_UNINSTALL}" "NoRepair" 1
  WriteRegStr HKLM "${REG_APP}" "InstallDir" "$INSTDIR"
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  SetRegView 64

  Delete "$INSTDIR\flectar-mail.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  RMDir /r "$INSTDIR\LICENSES"
  RMDir /r "$INSTDIR\Licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\Uninstall ${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"

  DeleteRegKey HKLM "${REG_UNINSTALL}"
  DeleteRegKey HKLM "${REG_APP}"
SectionEnd

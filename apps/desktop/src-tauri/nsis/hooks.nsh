; dasdevbot NSIS hooks. Studio Director's conditions:
; per-user install, no elevation, no updater, no service, no Run key,
; no Startup shortcut and no scheduled task. Nothing below adds any of those.
; The uninstaller removes the installed binaries and shortcuts (Tauri's
; template). Deleting data is the unchecked "Also delete my data" box, which
; removes %APPDATA%\net.dasdev.dasdevbot and %LOCALAPPDATA%\net.dasdev.dasdevbot.

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing dasdevbot from $INSTDIR"
  DetailPrint "Data is kept unless 'Also delete my data' was checked:"
  DetailPrint "  $APPDATA\net.dasdev.dasdevbot"
  DetailPrint "  $LOCALAPPDATA\net.dasdev.dasdevbot"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; The demo daemon is a sidecar next to the app, so the template's
  ; RMDir of $INSTDIR already removes it. Belt and braces:
  Delete "$INSTDIR\dasdevbotd.exe"
!macroend

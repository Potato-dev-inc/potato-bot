; The phone's Cloudflare tunnel runs as its own process so it survives app restarts and updates. Uninstalling must end it.
; An update (the old version is removed with /UPDATE) must NOT: the new version reattaches to the same process.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --phone-cleanup'
  ${EndIf}
!macroend

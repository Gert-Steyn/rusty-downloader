; Rusty Downloader's additions to Tauri's NSIS installer (tauri.conf.json,
; bundle > windows > nsis > installerHooks).

; Before the files go: close the app, then let it undo what it registered for
; this user (browser host, notifications, "startup approved") and, when
; "Delete the application data" is ticked, delete its passwords and data
; folder. See src-tauri/src/uninstall.rs. Not when an update reinstalls.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    ${If} $DeleteAppDataCheckboxState = 1
      ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall --delete-data'
    ${Else}
      ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall'
    ${EndIf}
  ${EndIf}
!macroend

!define NOVA_ELECTRON_GUID "7d673089-a13f-510f-8322-bca3e62b3103"
!define NOVA_ELECTRON_UNINSTALLER "Uninstall Nova Client.exe"

!define NOVA_TAURI_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Nova Client"

!macro NSIS_HOOK_PREINSTALL
  Push $R0
  Push $R1
  Push $R2

  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${NOVA_ELECTRON_GUID}" "UninstallString"
  ${If} $R0 != ""
    ReadRegStr $R1 HKCU "Software\${NOVA_ELECTRON_GUID}" "InstallLocation"
    ${If} $R1 == ""
      StrCpy $R1 "$LOCALAPPDATA\Programs\nova-launcher"
    ${EndIf}
    ${If} ${FileExists} "$R1\${NOVA_ELECTRON_UNINSTALLER}"
      DetailPrint "Quitando Nova Client (versión Electron)…"
      InitPluginsDir

      CopyFiles /SILENT "$R1\${NOVA_ELECTRON_UNINSTALLER}" "$PLUGINSDIR\nova-electron-uninstall.exe"
      ClearErrors
      ExecWait '"$PLUGINSDIR\nova-electron-uninstall.exe" /S /KEEP_APP_DATA /currentuser _?=$R1' $R2
      ${If} ${Errors}
        ClearErrors
        ExecWait '"$R1\${NOVA_ELECTRON_UNINSTALLER}" /S /KEEP_APP_DATA /currentuser _?=$R1' $R2
      ${EndIf}

      Delete "$R1\${NOVA_ELECTRON_UNINSTALLER}"
      RMDir "$R1"
      DetailPrint "Versión anterior quitada (código $R2). Tus perfiles, mundos y ajustes se conservan."
    ${EndIf}

    CreateDirectory "$INSTDIR"
    SetOutPath "$INSTDIR"
  ${EndIf}

  ReadRegStr $R0 HKCU "${NOVA_TAURI_KEY}" "UninstallString"
  ${If} $R0 != ""
    ReadRegStr $R1 HKCU "${NOVA_TAURI_KEY}" "InstallLocation"
    StrCpy $R2 $R1 1
    ${If} $R2 == '"'
      StrCpy $R1 $R1 "" 1
      StrCpy $R1 $R1 -1
    ${EndIf}
    ${If} $R1 != ""
    ${AndIf} ${FileExists} "$R1\uninstall.exe"
      DetailPrint "Quitando Nova Client (ahora se llama Trident Client)…"
      ClearErrors
      ExecWait '"$R1\uninstall.exe" /S _?=$R1' $R2
      Delete "$R1\uninstall.exe"
      RMDir "$R1"
      DetailPrint "Nova Client quitado (código $R2). Tus perfiles, mundos y ajustes se conservan."
    ${EndIf}
    CreateDirectory "$INSTDIR"
    SetOutPath "$INSTDIR"
  ${EndIf}

  Pop $R2
  Pop $R1
  Pop $R0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Push $R0
  ClearErrors
  ${GetOptions} $CMDLINE "--force-run" $R0
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
  ${EndIf}
  ClearErrors
  Pop $R0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Push $R0
  Push $R1
  StrCpy $R1 0
  ClearErrors
  ${GetOptions} $CMDLINE "/UPDATE" $R0
  ${IfNot} ${Errors}
    StrCpy $R1 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    StrCpy $R1 1
  ${EndIf}
  ClearErrors

  ${If} $R1 = 0
    SetShellVarContext current
    DetailPrint "Borrando todos los datos de Trident Client…"
    RMDir /r "$APPDATA\Trident Client"
    RMDir /r "$APPDATA\.tridentclient"
    RMDir /r "$APPDATA\Nova Client"
    RMDir /r "$APPDATA\.novalauncher"
    RMDir /r "$APPDATA\com.novalauncher.app"
    RMDir /r "$LOCALAPPDATA\com.novalauncher.app"

    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Trident Client"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Nova Client"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Trident Client"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Nova Client"

    RMDir "$INSTDIR"
  ${EndIf}

  Pop $R1
  Pop $R0
!macroend

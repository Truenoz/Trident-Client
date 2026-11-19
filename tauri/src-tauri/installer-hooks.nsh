; Trident Client — ganchos del instalador de Tauri (NSIS). Tauri los carga desde tauri.conf.json
; (bundle > windows > nsis > installerHooks).
;
; Antes de llamarse Trident Client, el launcher se llamaba Nova Client (primero con Electron, después con Tauri).
; Las dos versiones viejas se quitan solas al instalar esta; las carpetas de datos son las mismas, así que no se
; pierde nada.
;
; 1) Migración desde la versión Electron: si está instalada, se desinstala en silencio antes de copiar
;    los archivos nuevos, así no quedan dos «Nova Client» en «Aplicaciones instaladas».
;    Los datos del usuario NO se tocan: viven en %APPDATA%\Nova Client y %APPDATA%\.novalauncher, y el
;    desinstalador de Electron recibe /KEEP_APP_DATA (además, la versión Electron nunca borró datos).
; 2) Nova Client con Tauri (versión 1.2.0): también se desinstala en silencio. Su desinstalador, sin la casilla
;    «borrar datos» (que en modo silencioso no se marca), deja todos los datos.
; 3) Si quien abrió este instalador fue el actualizador de la versión Electron (pasa «--updated /S --force-run»),
;    Trident Client se abre solo al terminar, como hacía electron-updater.

; Clave de la versión Electron en el registro: UUID v5 de appId «com.novalauncher.app» con el espacio de nombres
; de electron-builder (50e065bc-3134-11e6-9bab-38c9862bdaf3). Es fija mientras appId no cambie.
!define NOVA_ELECTRON_GUID "7d673089-a13f-510f-8322-bca3e62b3103"
!define NOVA_ELECTRON_UNINSTALLER "Uninstall Nova Client.exe"
; Clave de la versión Tauri con el nombre viejo (Tauri usa el nombre del producto).
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
      ; Igual que electron-builder: el desinstalador se ejecuta desde una copia temporal y con _?= para esperar a
      ; que termine. Sin --updated, así también borra sus accesos directos (este instalador crea los nuevos).
      CopyFiles /SILENT "$R1\${NOVA_ELECTRON_UNINSTALLER}" "$PLUGINSDIR\nova-electron-uninstall.exe"
      ClearErrors
      ExecWait '"$PLUGINSDIR\nova-electron-uninstall.exe" /S /KEEP_APP_DATA /currentuser _?=$R1' $R2
      ${If} ${Errors}
        ClearErrors
        ExecWait '"$R1\${NOVA_ELECTRON_UNINSTALLER}" /S /KEEP_APP_DATA /currentuser _?=$R1' $R2
      ${EndIf}
      ; Con _?= el desinstalador no puede borrarse a sí mismo: se borra acá. La carpeta solo si quedó vacía.
      Delete "$R1\${NOVA_ELECTRON_UNINSTALLER}"
      RMDir "$R1"
      DetailPrint "Versión anterior quitada (código $R2). Tus perfiles, mundos y ajustes se conservan."
    ${EndIf}
    ; Por si alguien eligió para Tauri la misma carpeta que tenía Electron: el desinstalador la vació.
    CreateDirectory "$INSTDIR"
    SetOutPath "$INSTDIR"
  ${EndIf}

  ; Nova Client (Tauri). Tauri guarda la carpeta entre comillas en InstallLocation.
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

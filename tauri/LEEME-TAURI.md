# Trident Client sobre Tauri

Este es el launcher Trident Client (antes Nova Client). Reemplaza a la versión Electron, que se quitó del repositorio (queda en el historial
de git). La interfaz (`ui/`) usa `window.launcher`; el adaptador `ui/launcher-shim.js` lo conecta con el backend en Rust.
Las menciones a Electron de este archivo explican qué cambió respecto a esa versión y cómo se migra a quien la tiene instalada.

## Estado

| Parte | Contenido | Estado |
|---|---|---|
| Base | Ventana sin marco, ajustes, perfiles, versiones, novedades, carpetas, instancia única, instalador NSIS | Compila y con pruebas |
| Juego | Login Microsoft, modo sin cuenta, Minecraft/Java, lanzamiento, Fabric + Fabric API, jar de Nova Client | Compila y con pruebas |
| Contenido | Mods/Modrinth, sincronización, rendimiento, mundos, skins, exportar/importar, depuración | Compila y con pruebas |
| Sistema | Discord, bandeja, inicio con Windows, zoom, aceleración por hardware, registro de errores | Compila y con pruebas |
| Actualizaciones | Actualizador de Tauri con la misma experiencia que Electron, jars de Nova republicados, release con firma | Compila y con pruebas |
| Migración | El instalador desinstala la versión Electron; los usuarios de Electron reciben Tauri por su actualizador | Hook NSIS compilado con `makensis` |

**Falta probar en Windows real** (no se puede desde Linux): login de Microsoft, descargas reales de Mojang/Fabric/Modrinth/Adoptium,
diálogos de archivos, «Abrir carpeta», la bandeja, el instalador con la versión Electron instalada y una actualización completa
de punta a punta. Ver «La primera versión Tauri» en [PUBLICAR.md](../PUBLICAR.md).

## Cuenta y juego

Código nuevo en `src-tauri/src/`:

| Archivo | Qué hace |
|---|---|
| `account.rs` | Ventana de Microsoft → Xbox Live → XSTS → Minecraft (misma app y mismo flujo que `msmc`), modo sin cuenta (UUID offline idéntico al del servidor vanilla), sesión guardada cifrada con DPAPI |
| `launcher.rs` | JSON de versión con herencia (Fabric), reglas de Mojang, librerías, nativos, recursos, `logging`, argumentos |
| `java.rs` | Baja Temurin 8/17/21/25 a `.novalauncher/runtime/jre-N` |
| `fabric.rs` | Perfil de Fabric (meta.fabricmc.net) y Fabric API (Modrinth, con verificación SHA-512) |
| `nova.rs` | Jar de Nova Client del release `mods-rolling` + corrección de `fabric.mod.json` en jars viejos |
| `game.rs` | Sesión, preparación del perfil, lanzamiento, registro en vivo, tiempo jugado, «Detener» |
| `dl.rs` | Descargas con SHA-1, reintentos y archivo `.part` |

Cambios respecto a Electron, a propósito:
- **Si no hay internet al abrir**, la sesión guardada se conserva (Electron la borraba). Solo se borra si Microsoft la rechaza.
- **La memoria** se pasa en MB (`-Xmx4608M`) porque Java no acepta decimales en GB.
- **Al detener el juego** desde el botón, el mensaje es «Juego cerrado.» en vez de «código null».

Tampoco se usa el jar de Nova incluido en el instalador: solo el del release `mods-rolling`.

## Contenido

| Archivo | Qué hace |
|---|---|
| `content.rs` | Lista de mods y paquetes de recursos (activar/desactivar, quitar, añadir archivos locales), búsqueda y descarga desde Modrinth con dependencias, identificar archivos por hash, buscar/aplicar actualizaciones, sincronización antes de jugar, mods de rendimiento, mundos, registro del juego, exportar/importar/compartir perfiles (`.novaprofile` y códigos `NOVA1:`), copiar de otro perfil |
| `modcompat.rs` | Lee `fabric.mod.json` / `quilt.mod.json` / `mods.toml` y dice si el mod sirve para la versión y el cargador del perfil |
| `skins.rs` | Skin por nombre o UUID (sesión → Mojang → mc-heads → caché en disco) |
| `profiles.rs` | Ahora también guarda los servidores y permite cambiar campos internos (rendimiento) |
| `main.rs` | Comandos nuevos, selectores de archivos (plugin `dialog`), exportar depuración (.zip) |
| `game.rs` | Antes de lanzar un perfil Fabric: sincroniza contenido (si «auto-sincronizar» está activo) y prepara los mods de rendimiento |

Compatibilidad: los códigos para compartir y los `.novaprofile` son idénticos a los de Electron (se probó leyendo un código generado por la versión Electron), y `nova-content.json` conserva el mismo formato.

Diferencias a propósito:
- **Los paquetes de recursos también se actualizan en perfiles Fabric.** Electron filtraba todo por «fabric» y por eso nunca encontraba actualizaciones de paquetes de recursos; aquí los mods se filtran por cargador y los paquetes no.
- **Las descargas de Modrinth se verifican con SHA-1** cuando Modrinth lo informa.
- **Las acciones que cambian archivos de un perfil** (activar, quitar, instalar, actualizar, rendimiento, copiar, borrar) se bloquean mientras ese perfil está en juego, como en Electron (ahora también «borrar perfil»).
- Copiar el código de un perfil al portapapeles lo hace el adaptador (`launcher-shim.js`) en la interfaz.

## Sistema

| Archivo | Qué hace |
|---|---|
| `discord.rs` | Rich Presence hablando directo con el IPC de Discord (named pipe en Windows). Se muestra al abrirse el juego y se quita al cerrarse; reintenta cada 15 s si Discord no está abierto. `app_info` ya devuelve `discordConfigured` real |
| `system.rs` | Bandeja («Abrir lanzador» / «Salir del launcher», clic izquierdo abre la ventana), inicio con Windows (`--hidden` en modo bandeja), zoom (`uiScale`, de 0.8 a 1.6) y `errores.log` |
| `main.rs` | La ventana nace oculta y se muestra salvo que Windows la abra con `--hidden` en modo bandeja (y la bandeja exista). `settings_set` aplica zoom e inicio al instante. Comando `report_error` |
| `launcher-shim.js` | Envía los errores de JavaScript de la interfaz a `report_error` (máx. 20 por sesión) |

Detalles:
- **Inicio con Windows** escribe el valor `Trident Client` en `HKCU\...\Run` y borra el viejo `Nova Client` (Electron y Tauri 1.2.0). No hace nada en `tauri dev`.
- **errores.log** queda en `%APPDATA%\Nova Client\logs\` solo con «Guardar informes de errores» activo. Registra panics de Rust y errores de la interfaz (Electron solo registraba el proceso principal). Al pasar de 1 MB pasa a `errores.old.log`.
- **Zoom:** el backend aplica `uiScale`, pero la interfaz ya no tiene control para cambiarlo (se fija en 100 %).
- **Aceleración por hardware:** apagada, WebView2 arranca con `--disable-gpu` (por eso la ventana se crea desde `main.rs` y no desde `tauri.conf.json`). Se aplica al reiniciar, igual que en Electron.
- **Tamaño de ventana** (`winWidth`/`winHeight`/`fullscreen`) es el de la ventana de *Minecraft*, no la del launcher; lo aplica `game.rs`.

## Actualizaciones

| Archivo | Qué hace |
|---|---|
| `updater.rs` | Busca `latest.json` del último release al abrir y cada 30 min. Misma máquina de estados que `liveupdate.js`: cuenta regresiva de 10 s con Minecraft cerrado, manual con Minecraft abierto, instalar al cerrar el juego, no volver a empezar sola una versión que el usuario frenó. Descarga con progreso y ejecuta el instalador en modo pasivo, que reabre la app |
| `nova.rs` | «Buscar ahora» también vuelve a bajar los jars de Nova Client republicados en `mods-rolling` (aviso `kind: 'mods'`) |
| `launcher-shim.js` | `checkUpdates`, `holdUpdate`, `startUpdate` llaman a Rust; los eventos siguen llegando por `update:status` |

Al publicar, el workflow firma el instalador con la clave del actualizador (`src-tauri/tauri.release.conf.json` activa
`createUpdaterArtifacts`). La clave pública está en `tauri.conf.json`; la privada va en los secretos del repositorio (ver
[PUBLICAR.md](../PUBLICAR.md)). Los builds locales no la necesitan. En `tauri dev` no se busca nada.

## Migración desde Electron

`src-tauri/installer-hooks.nsh` (gancho `NSIS_HOOK_PREINSTALL`): si en `HKCU` existe la desinstalación de la versión
Electron (clave `7d673089-a13f-510f-8322-bca3e62b3103`, el UUID que electron-builder deriva de `com.novalauncher.app`),
la ejecuta en silencio con `/KEEP_APP_DATA` antes de copiar los archivos. Así queda una sola entrada en «Aplicaciones
instaladas» y los datos (`%APPDATA%\Nova Client`, `%APPDATA%\.novalauncher`) no se tocan.

Para los usuarios que ya tienen Electron, el release trae también `latest.yml` apuntando al instalador de Tauri:
electron-updater lo baja, comprueba el sha512 y lo ejecuta con `--updated /S --force-run`. El gancho
`NSIS_HOOK_POSTINSTALL` ve `--force-run` y abre Trident Client al terminar. El mismo gancho desinstala también la
versión Tauri con el nombre viejo (clave `Uninstall\Nova Client`).

Lo único que no se migra es la sesión de Microsoft (`account.bin`, cifrada con `safeStorage` de Electron): hay que iniciar
sesión una vez; desde entonces se guarda con DPAPI (etiqueta `N1:`). El inicio con Windows se registra como
«Trident Client» y se borra el «Nova Client» viejo.

## Cómo se probó

`cargo test` (50 pruebas) cubre reglas de Mojang, librerías, herencia Fabric, argumentos, UUID offline, el parche de jars,
una **prueba de punta a punta del juego** (un servidor HTTP local sirve un «Minecraft» falso, el lanzador lo descarga,
extrae nativos, arma la línea de comandos y ejecuta una JVM real), una **prueba de punta a punta del contenido** (un
«Modrinth» falso local), Discord con un servidor IPC falso, arranque oculto, zoom, aceleración por hardware, registro de
errores y la **máquina de estados del actualizador** (cuenta regresiva, Minecraft abierto, frenar, reintentar, instalar al
cerrar). En el workflow corre también en Windows (incluye el ida y vuelta con DPAPI). El gancho del instalador se compiló
con `makensis`.

## Compatibilidad de datos

Usa las mismas carpetas que la versión Electron, así que ajustes, perfiles y mundos se conservan:
- `%APPDATA%\Nova Client` → `settings.json`, `profiles.json`, caché de mods
- `%APPDATA%\.novalauncher` → versiones, instancias, mundos, Java

## Compilar

GitHub Actions: **Actions → Compilar Trident Client (Tauri) → Run workflow**. Corre `cargo check` y `cargo test` y genera
`Trident-Client-Setup.exe` (artifact `Trident-Client-Setup-Tauri`). Si los secretos de firma están cargados, también genera la
firma. Con una etiqueta `vX.Y.Z`, además publica el release.

En tu PC (Windows, con [Rust](https://rustup.rs) y Node 20):

```bash
cd tauri
npm install
npm run dev          # ventana de desarrollo
npm run build        # instalador en src-tauri/target/release/bundle/nsis/ (no necesita ninguna clave)
```

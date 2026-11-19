# Cómo publicar una versión de Trident Client

El repositorio es `Truenoz/Nova-Client`. (El launcher antes se llamaba Nova Client; el mod dentro del juego conserva ese nombre.) Las versiones se publican con el workflow **Compilar Trident Client (Tauri)** (`.github/workflows/tauri-build.yml`) y se distribuyen por GitHub Releases.

## Una sola vez: la clave del actualizador

El actualizador de Tauri solo instala instaladores firmados con tu clave. La clave pública ya está en `tauri/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`); la privada **no va en el repositorio**.

En GitHub: **Settings → Secrets and variables → Actions → New repository secret**:

| Secreto | Valor |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | el contenido completo de `nova-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | la contraseña de esa clave |

Guardá la clave y la contraseña en un lugar seguro (un gestor de contraseñas). **Si las perdés, los usuarios ya instalados no podrán recibir más actualizaciones automáticas** y tendrían que bajar el instalador a mano.

¿Preferís generar tu propia clave? `cd tauri && npx tauri signer generate -w nova-updater.key`, pegá el contenido de `nova-updater.key.pub` en `plugins.updater.pubkey` y cargá los dos secretos. Hacelo **antes** de la primera publicación de Tauri.

## Publicar (lo normal)

1. Cambiá el código.
2. Subí los cambios y creá la etiqueta con el número nuevo (más alto que el anterior):

   ```bash
   git add -A && git commit -m "Trident Client 1.3.0"
   git push
   git tag v1.3.0
   git push origin v1.3.0
   ```

3. GitHub Actions corre las pruebas, compila y crea el release `v1.3.0` con:
   - `Trident-Client-Setup.exe` — el instalador
   - `Trident-Client-Setup.exe.sig` — su firma
   - `latest.json` — lo que lee el actualizador de Tauri
   - `latest.yml` — lo que lee la versión vieja (Electron), para que se pase sola a la nueva

La versión sale de la etiqueta (el workflow la escribe en `tauri.conf.json` y `Cargo.toml` antes de compilar).

Los usuarios con el launcher abierto la detectan solos (al abrir y cada 30 min). Con Minecraft cerrado: cuenta regresiva de 10 s, descarga, el launcher se cierra, el instalador muestra una barra de progreso y Nova Client se vuelve a abrir. Con Minecraft abierto: se elige desde el cartel y se instala al cerrar el juego. Perfiles, mundos y ajustes no se tocan.

## La primera versión Tauri (paso desde Electron)

Antes de la primera etiqueta, probá el instalador: **Actions → Compilar Trident Client (Tauri) → Run workflow**, bajá `Trident-Client-Setup-Tauri` de *Artifacts* e instalalo en una PC que tenga la versión Electron. Comprobá:

- Que en «Aplicaciones instaladas» quede **un solo** Trident Client (el instalador desinstala Nova Client, tanto la versión Electron como la Tauri 1.2.0).
- Que estén tus perfiles, mundos y ajustes.
- Iniciar sesión con Microsoft (hay que hacerlo una vez: la sesión de Electron está cifrada de otra forma) y jugar.

Cuando subas la etiqueta, los usuarios de la versión Electron reciben la actualización como siempre: electron-updater baja `Trident-Client-Setup.exe` (el de Tauri) y lo ejecuta en silencio; ese instalador quita la versión Electron, instala la nueva y la abre.

## Enlace de descarga para usuarios nuevos

Siempre instala la última versión publicada (el nombre del archivo no cambia entre versiones):

```
https://github.com/Truenoz/Nova-Client/releases/latest/download/Trident-Client-Setup.exe
```

## Qué se conserva al instalar, actualizar y desinstalar

| Dato | Dónde está | Instalar / actualizar | Desinstalar |
|---|---|---|---|
| Ajustes, cuenta (cifrada), perfiles, caché de mods | `%APPDATA%\Nova Client` | se conserva | se conserva |
| Minecraft: versiones, mundos, mods, Java | `%APPDATA%\.novalauncher` | se conserva | se conserva |
| Programa | `%LOCALAPPDATA%\Trident Client` (o la carpeta elegida) | se reemplaza | se borra |

## Reglas que NO hay que romper

- `identifier` (`com.novalauncher.app`) y `productName` (`Trident Client`) en `tauri.conf.json`: de ahí salen la entrada de «Aplicaciones instaladas» y la carpeta de datos. Cambiarlos duplica la instalación.
- La clave de firma: si cambia `pubkey`, las apps ya instaladas rechazan las actualizaciones nuevas.
- El nombre `Trident-Client-Setup.exe`: es el del enlace permanente y el que esperan `latest.json` y `latest.yml`.
- Los releases de versión deben ser **normales** (no pre-release) y `mods-rolling` debe seguir siendo **pre-release** (`publish-mods.js` ya lo crea así). Si no, `releases/latest` apuntaría al lugar equivocado y las actualizaciones dejarían de funcionar.
- Cada etiqueta nueva tiene que ser un número más alto: el actualizador no instala versiones más viejas.

## Detalles del instalador

- Asistente en español con licencia, elección de carpeta y barra de progreso; instala para el usuario actual (no pide permisos de administrador).
- Crea accesos directos en el menú Inicio y en el escritorio.
- Sin firma de código: Windows muestra «Editor desconocido» y SmartScreen puede avisar («Más información → Ejecutar de todas formas»). La clave del actualizador es otra cosa: solo sirve para que el launcher acepte sus propias actualizaciones.

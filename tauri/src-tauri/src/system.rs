// Integración con el sistema: bandeja, inicio con Windows (--hidden), zoom de la interfaz
// y registro local de errores.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

// ───────────────────────── Ajustes ─────────────────────────

fn flag(s: &Map<String, Value>, key: &str) -> bool {
    s.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn start_mode_is_tray(s: &Map<String, Value>) -> bool {
    s.get("startMode").and_then(|v| v.as_str()).unwrap_or("tray") == "tray"
}

/// ¿Arranca oculto? Solo cuando Windows lo abre al iniciar (--hidden) y el modo elegido es «bandeja».
pub fn start_hidden(args: &[String], s: &Map<String, Value>) -> bool {
    args.iter().any(|a| a == "--hidden") && flag(s, "openAtLogin") && start_mode_is_tray(s)
}

// ───────────────────────── Aceleración por hardware ─────────────────────────

/// Argumentos de WebView2. Los dos primeros son los que Tauri pone por defecto; con «Aceleración por hardware»
/// apagada se agrega `--disable-gpu` (como `app.disableHardwareAcceleration()` en Electron). Se aplica al reiniciar.
pub fn browser_args(s: &Map<String, Value>) -> String {
    let base = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    if s.get("hwAccel").and_then(|v| v.as_bool()).unwrap_or(true) {
        base.to_string()
    } else {
        format!("{base} --disable-gpu")
    }
}

/// Los argumentos con los que arrancó WebView2 en esta sesión. Todas las ventanas (la principal y la de
/// Microsoft) tienen que usar exactamente los mismos: si difieren, WebView2 falla con 0x8007139F.
/// Se fijan una sola vez al abrir el launcher; cambiar «Aceleración por hardware» se aplica al reiniciar.
static SESSION_ARGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn session_browser_args(s: &Map<String, Value>) -> &'static str {
    SESSION_ARGS.get_or_init(|| browser_args(s))
}

// ───────────────────────── Zoom de la interfaz ─────────────────────────

/// Escala válida de la interfaz: de 0.8 a 1.6; si no es un número, 1.
pub fn ui_scale(s: &Map<String, Value>) -> f64 {
    let z = s
        .get("uiScale")
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|t| t.trim().parse::<f64>().ok())))
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(1.0);
    z.clamp(0.8, 1.6)
}

pub fn apply_zoom(app: &AppHandle, s: &Map<String, Value>) {
    if let Some(w) = app.get_webview_window("main") {
        if let Err(e) = w.set_zoom(ui_scale(s)) {
            eprintln!("No se pudo aplicar el zoom: {e}");
        }
    }
}

// ───────────────────────── Inicio con Windows ─────────────────────────

/// Nombre del valor en `HKCU\...\Run`.
#[cfg(windows)]
const RUN_NAME: &str = "Trident Client";
/// Nombre que usaban Nova Client (Electron y Tauri): se borra para que no queden dos entradas ni una que apunte
/// a un programa que ya no existe.
#[cfg(windows)]
const LEGACY_RUN_NAME: &str = "Nova Client";

/// Línea de comando que registra Windows: la ruta entre comillas y, en modo bandeja, `--hidden`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn login_command(exe: &Path, tray: bool) -> String {
    let base = format!("\"{}\"", exe.display());
    if tray {
        format!("{base} --hidden")
    } else {
        base
    }
}

/// Registra (o quita) el arranque con Windows según `openAtLogin` y `startMode`.
/// En desarrollo no toca el sistema.
pub fn apply_login_item(s: &Map<String, Value>) {
    if cfg!(debug_assertions) {
        return;
    }
    if let Err(e) = set_login_item(flag(s, "openAtLogin"), start_mode_is_tray(s)) {
        eprintln!("No se pudo configurar el inicio con el sistema: {e}");
    }
}

#[cfg(windows)]
fn set_login_item(enabled: bool, tray: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| e.to_string())?
        .0;
    // Si el usuario lo había desactivado desde el Administrador de tareas, esta marca lo mantiene apagado:
    // borrarla deja el arranque en su estado normal (activado).
    let approved = hkcu
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run", KEY_SET_VALUE)
        .ok();

    let _ = run.delete_value(LEGACY_RUN_NAME);
    if let Some(k) = &approved {
        let _ = k.delete_value(LEGACY_RUN_NAME);
    }
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        run.set_value(RUN_NAME, &login_command(&exe, tray)).map_err(|e| e.to_string())?;
        if let Some(k) = &approved {
            let _ = k.delete_value(RUN_NAME);
        }
    } else {
        // Si el valor no existe, no es un error.
        let _ = run.delete_value(RUN_NAME);
        if let Some(k) = &approved {
            let _ = k.delete_value(RUN_NAME);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_login_item(_enabled: bool, _tray: bool) -> Result<(), String> {
    Ok(())
}

// ───────────────────────── Bandeja ─────────────────────────

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Ícono en la bandeja con el menú «Abrir lanzador / Salir del launcher». Clic izquierdo abre la ventana.
pub fn create_tray(app: &AppHandle) -> Result<(), String> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItemBuilder::with_id("open", "Abrir lanzador").build(app).map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "Salir del launcher").build(app).map_err(|e| e.to_string())?;
    let menu = MenuBuilder::new(app).item(&open).item(&quit).build().map_err(|e| e.to_string())?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")).map_err(|e| e.to_string())?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Trident Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────── Registro de errores ─────────────────────────

const LOG_MAX_BYTES: u64 = 1_000_000;

/// Fecha y hora UTC en formato ISO 8601 (sin dependencias de fechas).
pub fn iso_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rest = unix_secs % 86_400;
    // Día civil desde días desde 1970-01-01 (algoritmo de Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// `errores.log` vive en `<datos>/logs/`, igual que en Electron.
pub fn error_log_path(user_dir: &Path) -> PathBuf {
    user_dir.join("logs").join("errores.log")
}

/// Agrega una entrada al registro, pero solo si «Enviar informes de errores» está activo.
/// Nunca falla hacia afuera: un error al registrar un error no debe causar otro.
pub fn report_error(user_dir: &Path, kind: &str, detail: &str) {
    use std::io::Write;
    let enabled = crate::settings::read(user_dir).get("sendErrorReports").and_then(|v| v.as_bool()).unwrap_or(false);
    if !enabled {
        return;
    }
    let path = error_log_path(user_dir);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Para que no crezca sin límite, el registro lleno pasa a errores.old.log (el anterior se pisa).
    if std::fs::metadata(&path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_file_name("errores.old.log"));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = write!(f, "[{}] {kind}: {detail}\n\n", iso_utc(now));
    }
}

/// Los fallos inesperados del programa (panics) también quedan en el registro.
pub fn install_panic_reporter(user_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let trace = std::backtrace::Backtrace::force_capture();
        report_error(&user_dir, "panic", &format!("{info}\n{trace}"));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn arranque_oculto_solo_con_las_tres_condiciones() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let on = map(json!({ "openAtLogin": true, "startMode": "tray" }));
        assert!(start_hidden(&args(&["nova.exe", "--hidden"]), &on));
        assert!(!start_hidden(&args(&["nova.exe"]), &on), "sin --hidden");
        assert!(!start_hidden(&args(&["nova.exe", "--hidden"]), &map(json!({ "openAtLogin": false, "startMode": "tray" }))));
        assert!(!start_hidden(&args(&["nova.exe", "--hidden"]), &map(json!({ "openAtLogin": true, "startMode": "window" }))));
        // Sin startMode guardado vale el predeterminado («tray»), como en Electron.
        assert!(start_hidden(&args(&["--hidden"]), &map(json!({ "openAtLogin": true }))));
    }

    #[test]
    fn escala_de_la_interfaz() {
        assert_eq!(ui_scale(&map(json!({}))), 1.0);
        assert_eq!(ui_scale(&map(json!({ "uiScale": 1.25 }))), 1.25);
        assert_eq!(ui_scale(&map(json!({ "uiScale": "1.2" }))), 1.2);
        assert_eq!(ui_scale(&map(json!({ "uiScale": 3 }))), 1.6);
        assert_eq!(ui_scale(&map(json!({ "uiScale": 0.1 }))), 0.8);
        assert_eq!(ui_scale(&map(json!({ "uiScale": 0 }))), 1.0);
        assert_eq!(ui_scale(&map(json!({ "uiScale": "mucho" }))), 1.0);
    }

    #[test]
    fn aceleracion_por_hardware() {
        assert!(!browser_args(&map(json!({}))).contains("--disable-gpu"));
        assert!(!browser_args(&map(json!({ "hwAccel": true }))).contains("--disable-gpu"));
        assert!(browser_args(&map(json!({ "hwAccel": false }))).ends_with(" --disable-gpu"));
    }

    #[test]
    fn linea_de_inicio_con_windows() {
        let exe = Path::new(r"C:\Users\Ana\AppData\Local\Trident Client\Trident Client.exe");
        assert_eq!(
            login_command(exe, true),
            r#""C:\Users\Ana\AppData\Local\Trident Client\Trident Client.exe" --hidden"#
        );
        assert_eq!(
            login_command(exe, false),
            r#""C:\Users\Ana\AppData\Local\Trident Client\Trident Client.exe""#
        );
    }

    #[test]
    fn fecha_iso() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z"); // año bisiesto
        assert_eq!(iso_utc(1_798_761_599), "2026-12-31T23:59:59Z");
    }

    #[test]
    fn el_registro_de_errores_respeta_el_ajuste() {
        let dir = std::env::temp_dir().join(format!("nova-errores-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Apagado (valor por defecto): no se escribe nada.
        report_error(&dir, "prueba", "no debe guardarse");
        assert!(!error_log_path(&dir).exists());

        // Encendido: queda la entrada con fecha y tipo.
        crate::settings::write(&dir, &json!({ "sendErrorReports": true })).unwrap();
        report_error(&dir, "panic", "algo falló");
        report_error(&dir, "interfaz", "otra cosa");
        let text = std::fs::read_to_string(error_log_path(&dir)).unwrap();
        assert!(text.contains("panic: algo falló"), "{text}");
        assert!(text.contains("interfaz: otra cosa"), "{text}");
        assert!(text.starts_with("[20"), "{text}");

        // Registro demasiado grande: pasa a errores.old.log y se empieza de nuevo.
        std::fs::write(error_log_path(&dir), vec![b'x'; (LOG_MAX_BYTES + 1) as usize]).unwrap();
        report_error(&dir, "nuevo", "después de rotar");
        assert!(dir.join("logs").join("errores.old.log").exists());
        let fresh = std::fs::read_to_string(error_log_path(&dir)).unwrap();
        assert!(fresh.contains("nuevo: después de rotar") && fresh.len() < 200, "{fresh}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

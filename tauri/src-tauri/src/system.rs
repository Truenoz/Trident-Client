use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

fn flag(s: &Map<String, Value>, key: &str) -> bool {
    s.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn start_mode_is_tray(s: &Map<String, Value>) -> bool {
    s.get("startMode").and_then(|v| v.as_str()).unwrap_or("tray") == "tray"
}

pub fn start_hidden(args: &[String], s: &Map<String, Value>) -> bool {
    args.iter().any(|a| a == "--hidden") && flag(s, "openAtLogin") && start_mode_is_tray(s)
}

pub fn browser_args(s: &Map<String, Value>) -> String {
    let base = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    if s.get("hwAccel").and_then(|v| v.as_bool()).unwrap_or(true) {
        base.to_string()
    } else {
        format!("{base} --disable-gpu")
    }
}

static SESSION_ARGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn session_browser_args(s: &Map<String, Value>) -> &'static str {
    SESSION_ARGS.get_or_init(|| browser_args(s))
}

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

#[cfg(windows)]
const RUN_NAME: &str = "Trident Client";

#[cfg(windows)]
const LEGACY_RUN_NAME: &str = "Nova Client";

#[cfg_attr(not(windows), allow(dead_code))]
pub fn login_command(exe: &Path, tray: bool) -> String {
    let base = format!("\"{}\"", exe.display());
    if tray {
        format!("{base} --hidden")
    } else {
        base
    }
}

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

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        let _ = w.show();
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
        let _ = w.set_always_on_top(false);
    }
}

pub const TRAY_MENU: &str = "traymenu";
const TRAY_W: f64 = 196.0;
const TRAY_H: f64 = 84.0;

pub const MENU_HOST: &str = "menu.trident.invalid";

pub fn menu_action_from_url(url: &tauri::Url) -> Option<String> {
    if url.host_str() != Some(MENU_HOST) {
        return None;
    }
    let action = url.path().trim_matches('/');
    matches!(action, "open" | "quit" | "close").then(|| action.to_string())
}

pub fn create_tray_menu(app: &AppHandle, browser_args: &str) -> bool {
    let handle = app.clone();
    tauri::WebviewWindowBuilder::new(app, TRAY_MENU, tauri::WebviewUrl::App("traymenu.html".into()))
        .on_navigation(move |url| {
            let Some(action) = menu_action_from_url(url) else { return true };
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || tray_action(&h, &action));
            false
        })
        .title("Trident Client")
        .inner_size(TRAY_W, TRAY_H)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .additional_browser_args(browser_args)
        .build()
        .is_ok()
}

fn show_tray_menu(app: &AppHandle, x: f64, y: f64) {
    let Some(w) = app.get_webview_window(TRAY_MENU) else { return };
    let monitor = app.monitor_from_point(x, y).ok().flatten();
    let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let _ = w.set_size(tauri::LogicalSize::new(TRAY_W, TRAY_H));
    let (pw, ph) = (TRAY_W * scale, TRAY_H * scale);
    let mut left = x;
    let mut top = y - ph;
    if let Some(m) = monitor.as_ref() {
        let area = m.work_area();
        let (ax, ay) = (area.position.x as f64, area.position.y as f64);
        let (aw, ah) = (area.size.width as f64, area.size.height as f64);
        if left + pw > ax + aw {
            left = x - pw;
        }
        if top < ay {
            top = y;
        }
        left = left.clamp(ax, (ax + aw - pw).max(ax));
        top = top.clamp(ay, (ay + ah - ph).max(ay));
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(left.round() as i32, top.round() as i32));
    let _ = w.show();
    let _ = w.set_focus();
}

pub fn tray_action(app: &AppHandle, action: &str) {
    match action {
        "open" => show_main_window(app),
        "quit" => {
            app.exit(0);
            return;
        }
        _ => {}
    }
    if let Some(w) = app.get_webview_window(TRAY_MENU) {
        let _ = w.hide();
    }
}

pub fn create_tray(app: &AppHandle, custom_menu: bool) -> Result<(), String> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")).map_err(|e| e.to_string())?;
    let mut builder = TrayIconBuilder::with_id("main").icon(icon).tooltip("Trident Client").show_menu_on_left_click(false);
    if !custom_menu {
        let open = MenuItemBuilder::with_id("open", "Abrir launcher").build(app).map_err(|e| e.to_string())?;
        let sep = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
        let quit = MenuItemBuilder::with_id("quit", "Salir del launcher").build(app).map_err(|e| e.to_string())?;
        let menu = MenuBuilder::new(app).item(&open).item(&sep).item(&quit).build().map_err(|e| e.to_string())?;
        builder = builder.menu(&menu).on_menu_event(|app, event| tray_action(app, event.id.as_ref()));
    }
    builder
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = event {
                match button {
                    MouseButton::Left => tray_action(tray.app_handle(), "open"),
                    MouseButton::Right if custom_menu => show_tray_menu(tray.app_handle(), position.x, position.y),
                    _ => {}
                }
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}

const LOG_MAX_BYTES: u64 = 1_000_000;

pub fn iso_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rest = unix_secs % 86_400;

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

pub fn error_log_path(user_dir: &Path) -> PathBuf {
    user_dir.join("logs").join("errores.log")
}

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
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(1_798_761_599), "2026-12-31T23:59:59Z");
    }

    #[test]
    fn el_registro_de_errores_respeta_el_ajuste() {
        let dir = std::env::temp_dir().join(format!("nova-errores-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        report_error(&dir, "prueba", "no debe guardarse");
        assert!(!error_log_path(&dir).exists());

        crate::settings::write(&dir, &json!({ "sendErrorReports": true })).unwrap();
        report_error(&dir, "panic", "algo falló");
        report_error(&dir, "interfaz", "otra cosa");
        let text = std::fs::read_to_string(error_log_path(&dir)).unwrap();
        assert!(text.contains("panic: algo falló"), "{text}");
        assert!(text.contains("interfaz: otra cosa"), "{text}");
        assert!(text.starts_with("[20"), "{text}");

        std::fs::write(error_log_path(&dir), vec![b'x'; (LOG_MAX_BYTES + 1) as usize]).unwrap();
        report_error(&dir, "nuevo", "después de rotar");
        assert!(dir.join("logs").join("errores.old.log").exists());
        let fresh = std::fs::read_to_string(error_log_path(&dir)).unwrap();
        assert!(fresh.contains("nuevo: después de rotar") && fresh.len() < 200, "{fresh}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tray_tests {
    use super::*;

    #[test]
    fn menu_urls() {
        let u = |s: &str| tauri::Url::parse(s).unwrap();
        assert_eq!(menu_action_from_url(&u("https://menu.trident.invalid/open")).as_deref(), Some("open"));
        assert_eq!(menu_action_from_url(&u("https://menu.trident.invalid/quit")).as_deref(), Some("quit"));
        assert_eq!(menu_action_from_url(&u("https://menu.trident.invalid/rm")), None);
        assert_eq!(menu_action_from_url(&u("tauri://localhost/traymenu.html")), None);
    }
}

// Ajustes del launcher: mismo archivo (settings.json) y mismos valores que la versión Electron,
// así quien ya usa Nova Client conserva su configuración.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

fn defaults() -> Map<String, Value> {
    let v = json!({
        "ramGB": 4,
        "javaPath": "",
        "lastVersion": "1.21.4",
        "showSnapshots": false,
        "closeOnLaunch": false,
        "novaClient": true,
        "discord": true,
        "discordClientId": "",
        "lastProfile": "",
        "language": "es",
        "openAtLogin": false,
        "startMode": "tray",
        "sendErrorReports": false,
        "uiScale": 1,
        "reduceMotion": false,
        "hwAccel": true,
        "jvmArgs": "",
        "winWidth": 0,
        "winHeight": 0,
        "fullscreen": false
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Valores por defecto + lo guardado en `settings.json`.
pub fn read(user_dir: &Path) -> Map<String, Value> {
    let mut m = defaults();
    if let Ok(text) = fs::read_to_string(user_dir.join("settings.json")) {
        let text = text.trim_start_matches('\u{feff}');
        if let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(text) {
            for (k, v) in saved {
                m.insert(k, v);
            }
        }
    }
    m
}

/// Mezcla `patch` con lo guardado y escribe el archivo. Devuelve los ajustes resultantes.
pub fn write(user_dir: &Path, patch: &Value) -> Result<Map<String, Value>, String> {
    let mut m = read(user_dir);
    if let Value::Object(p) = patch {
        for (k, v) in p {
            m.insert(k.clone(), v.clone());
        }
    }

    // La RAM va de 1 a 64 GB; si el valor no es un número válido, 4 GB.
    let ram = m
        .get("ramGB")
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok())))
        .filter(|n| n.is_finite() && *n != 0.0)
        .unwrap_or(4.0)
        .clamp(1.0, 64.0);
    let ram_value = if ram.fract() == 0.0 { json!(ram as i64) } else { json!(ram) };
    m.insert("ramGB".to_string(), ram_value);

    fs::create_dir_all(user_dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&Value::Object(m.clone())).map_err(|e| e.to_string())?;
    fs::write(user_dir.join("settings.json"), text).map_err(|e| e.to_string())?;
    Ok(m)
}

/// «Restablecer ajustes»: borra el archivo y devuelve los valores por defecto.
pub fn reset(user_dir: &Path) -> Map<String, Value> {
    let _ = fs::remove_file(user_dir.join("settings.json"));
    read(user_dir)
}

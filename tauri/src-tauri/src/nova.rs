// Mod «Nova Client» (jar de Fabric): se baja del release fijo `mods-rolling` de GitHub a `mod-cache/`
// (mismo índice `index.json` que la versión Electron) y se copia a la carpeta de mods del perfil.

use serde_json::{json, Value};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::dl;

// Configuración del repositorio en trident.config.json (GitHub, Discord, servidor de amigos).
const CONFIG: &str = include_str!("../../../trident.config.json");
const ROLLING_TAG: &str = "mods-rolling";

fn config() -> Value {
    serde_json::from_str(CONFIG.trim_start_matches('\u{feff}')).unwrap_or(Value::Null)
}

/// Texto de una clave de `trident.config.json` (vacío si no existe).
pub fn config_string(key: &str) -> String {
    config()[key].as_str().unwrap_or("").trim().to_string()
}

pub fn repo() -> Option<String> {
    let r = config()["githubRepo"].as_str()?.to_string();
    let ok = r.split('/').count() == 2
        && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    ok.then_some(r)
}

/// Servidor de amistades: el de Ajustes manda, si no el de trident.config.json. Vacío = sin amigos en línea.
pub fn net_url(settings: &serde_json::Map<String, Value>) -> String {
    let own = settings.get("novaNetUrl").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if !own.is_empty() {
        return own;
    }
    config()["novaNetUrl"].as_str().unwrap_or("").trim().to_string()
}

fn read_index(cache: &Path) -> Value {
    std::fs::read_to_string(cache.join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({ "tag": "0.0.0", "jars": {} }))
}

fn write_index(cache: &Path, index: &Value) {
    if let Ok(t) = serde_json::to_string_pretty(index) {
        let _ = std::fs::write(cache.join("index.json"), t);
    }
}

fn looks_like_jar(path: &Path) -> bool {
    let mut head = [0u8; 2];
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    size >= 1000 && std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == *b"PK"
}

/// Release fijo `mods-rolling`. `Ok(None)` = sin repositorio configurado o el release todavía no existe;
/// `Err` = sin conexión o GitHub respondió con un error.
async fn rolling_release() -> Result<Option<Value>, String> {
    let repo = match repo() {
        Some(r) => r,
        None => return Ok(None),
    };
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{ROLLING_TAG}");
    let r = dl::client()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    match r.status().as_u16() {
        404 => Ok(None),
        s if (200..300).contains(&s) => r.json().await.map(Some).map_err(|e| e.to_string()),
        s => Err(format!("HTTP {s}")),
    }
}

/// Archivo del release para esa versión: `trident-client-<mc>.jar` (o el nombre viejo `nova-client-<mc>.jar`).
fn find_asset<'a>(release: &'a Value, mc: &str) -> Option<&'a Value> {
    let assets = release["assets"].as_array()?;
    [format!("trident-client-{mc}.jar"), format!("nova-client-{mc}.jar")]
        .iter()
        .find_map(|name| assets.iter().find(|x| x["name"].as_str() == Some(name.as_str())))
}

/// Baja el jar de `mc` del release si es distinto del guardado. Devuelve true si lo actualizó.
/// En la PC se guarda siempre como `mod-cache/nova-client-<mc>.jar` (nombre interno, no se ve).
async fn take_from_release(cache: &Path, release: &Value, mc: &str, log: &(dyn Fn(String) + Send + Sync)) -> Result<bool, String> {
    let wanted = format!("nova-client-{mc}.jar");
    let jar = cache.join(&wanted);
    let asset = match find_asset(release, mc) {
        Some(a) => a,
        None => {
            log(format!("[Trident] El release «{ROLLING_TAG}» no tiene trident-client-{mc}.jar."));
            return Ok(false);
        }
    };
    let mut index = read_index(cache);
    if jar.exists() && index["jars"][mc]["id"] == asset["id"] {
        return Ok(false);
    }
    let download_url = asset["browser_download_url"].as_str().ok_or("El release no trae la URL del jar.")?;
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let tmp = cache.join(format!("{wanted}.new"));
    let got = dl::download(download_url, &tmp, None, |_, _| {}).await;
    if got.is_err() || !looks_like_jar(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("No se pudo descargar {wanted}."));
    }
    std::fs::rename(&tmp, &jar).map_err(|e| e.to_string())?;
    index["jars"][mc] = json!({ "id": asset["id"], "tag": release["tag_name"], "rolling": true });
    write_index(cache, &index);
    log(format!("[Trident] {mc}: jar descargado del release «{ROLLING_TAG}»."));
    Ok(true)
}

/// Jar de Nova Client para esa versión de Minecraft: el del release (si hay uno nuevo se baja) o el ya guardado.
/// `Ok(None)` = no hay jar publicado para esa versión (el juego arranca solo con Fabric).
pub async fn ensure_jar(user_dir: &Path, mc: &str, log: &(dyn Fn(String) + Send + Sync)) -> Result<Option<PathBuf>, String> {
    let cache = user_dir.join("mod-cache");
    let jar = cache.join(format!("nova-client-{mc}.jar"));
    let have = || jar.exists().then(|| jar.clone());
    // Revisado hace menos de una hora: se usa el que ya está (el juego abre sin esperar a GitHub).
    let key = format!("trident:{mc}");
    if jar.exists() && crate::dl::checked_recently(&key, 1) {
        return Ok(have());
    }
    let release = match rolling_release().await {
        Ok(Some(r)) => r,
        Ok(None) => {
            // Sin esto el juego arrancaba solo con Fabric sin decir por qué.
            match repo() {
                Some(r) => log(format!(
                    "[Trident] El repositorio {r} no tiene el release «{ROLLING_TAG}» (o es privado): no hay de dónde bajar el mod. Publicá los jars con «npm run publish:mods»."
                )),
                None => log("[Trident] Falta «githubRepo» en trident.config.json: no hay de dónde bajar el mod.".to_string()),
            }
            return Ok(have());
        }
        Err(e) => {
            if e.starts_with("HTTP") {
                log(format!("[Trident] No se pudo consultar el release ({e}); se usa lo que hay."));
            }
            return Ok(have()); // sin conexión
        }
    };
    match take_from_release(&cache, &release, mc, log).await {
        Ok(_) => crate::dl::mark_checked(&key),
        Err(e) => log(format!("[Trident] {e} Se usa lo que hay.")),
    }
    Ok(have())
}

/// «Buscar actualizaciones»: vuelve a bajar los jars ya guardados que se republicaron en `mods-rolling`.
/// Devuelve el tag del release y las versiones de Minecraft actualizadas.
pub async fn refresh_cached(user_dir: &Path, log: &(dyn Fn(String) + Send + Sync)) -> Result<(String, Vec<String>), String> {
    let cache = user_dir.join("mod-cache");
    let mine: Vec<String> = read_index(&cache)["jars"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if mine.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let release = match rolling_release().await? {
        Some(r) => r,
        None => return Ok((String::new(), Vec::new())),
    };
    let mut updated = Vec::new();
    for mc in mine {
        if take_from_release(&cache, &release, &mc, log).await? {
            updated.push(mc);
        }
    }
    Ok((release["tag_name"].as_str().unwrap_or("").to_string(), updated))
}

/// Corrige `fabric.mod.json` de un jar ya publicado:
///  · Los Fabric API viejos (hasta 1.19.x) se llaman «fabric», no «fabric-api»: la dependencia pasa a «fabric-api-base».
///  · Se quitan los mixins que el jar declara pero no trae (Mixin tira «was invalid or could not be read»).
/// Devuelve el jar nuevo, o `None` si no hace falta tocar nada.
pub fn patch_jar(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut text = String::new();
    match zip.by_name("fabric.mod.json") {
        Ok(mut f) => {
            f.read_to_string(&mut text).map_err(|e| e.to_string())?;
        }
        Err(_) => return Ok(None),
    }
    let mut meta: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let mut changed = false;

    if let Some(deps) = meta.get_mut("depends").and_then(|d| d.as_object_mut()) {
        if let Some(v) = deps.remove("fabric-api") {
            deps.insert("fabric-api-base".to_string(), v);
            changed = true;
        }
    }
    if let Some(list) = meta.get("mixins").and_then(|m| m.as_array()).cloned() {
        let names: Vec<String> = (0..zip.len()).filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.name().to_string())).collect();
        let keep: Vec<Value> = list
            .into_iter()
            .filter(|m| {
                let cfg = m.as_str().or_else(|| m["config"].as_str()).unwrap_or("");
                names.iter().any(|n| n == cfg)
            })
            .collect();
        if keep.len() != meta["mixins"].as_array().map_or(0, |a| a.len()) {
            meta["mixins"] = Value::Array(keep);
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }

    let new_json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).map_err(|e| e.to_string())?;
        if file.name() == "fabric.mod.json" {
            continue;
        }
        out.raw_copy_file(file).map_err(|e| e.to_string())?;
    }
    out.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated))
        .map_err(|e| e.to_string())?;
    out.write_all(&new_json).map_err(|e| e.to_string())?;
    Ok(Some(out.finish().map_err(|e| e.to_string())?.into_inner()))
}

/// Copia el jar a la carpeta de mods aplicando las correcciones de `patch_jar` (si fallan, queda el jar original).
pub fn install_jar(src: &Path, dest: &Path, log: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    match patch_jar(&bytes) {
        Ok(Some(patched)) => std::fs::write(dest, patched).map_err(|e| e.to_string()),
        Ok(None) => std::fs::write(dest, bytes).map_err(|e| e.to_string()),
        Err(e) => {
            log(format!("[Trident] No se pudo ajustar la dependencia de Fabric API: {e}"));
            std::fs::write(dest, bytes).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_asset_acepta_el_nombre_nuevo_y_el_viejo() {
        let r = json!({ "assets": [
            { "name": "trident-client-1.21.4.jar", "id": 1 },
            { "name": "nova-client-1.20.1.jar", "id": 2 },
            { "name": "trident-client-1.21.jar", "id": 3 }
        ]});
        assert_eq!(find_asset(&r, "1.21.4").unwrap()["id"], 1);
        assert_eq!(find_asset(&r, "1.20.1").unwrap()["id"], 2);
        assert_eq!(find_asset(&r, "1.21").unwrap()["id"], 3);
        assert!(find_asset(&r, "1.21.9").is_none());
    }

    fn make_jar(meta: &str, extra: &[&str]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("fabric.mod.json", o).unwrap();
        z.write_all(meta.as_bytes()).unwrap();
        for n in extra {
            z.start_file(*n, o).unwrap();
            z.write_all(b"x").unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn config_valida() {
        assert_eq!(repo().as_deref(), Some("Truenoz/Trident-Client"));
    }

    #[test]
    fn corrige_dependencia_y_mixins() {
        let jar = make_jar(
            r#"{"id":"novaclient","depends":{"fabric-api":">=0.1","minecraft":"1.19.2"},"mixins":["ok.mixins.json","falta.mixins.json",{"config":"ok2.mixins.json"}]}"#,
            &["ok.mixins.json", "ok2.mixins.json", "Clase.class"],
        );
        let out = patch_jar(&jar).unwrap().expect("debía cambiar");
        let mut z = zip::ZipArchive::new(Cursor::new(out)).unwrap();
        let mut s = String::new();
        z.by_name("fabric.mod.json").unwrap().read_to_string(&mut s).unwrap();
        let m: Value = serde_json::from_str(&s).unwrap();
        assert!(m["depends"].get("fabric-api").is_none());
        assert_eq!(m["depends"]["fabric-api-base"], ">=0.1");
        assert_eq!(m["mixins"].as_array().unwrap().len(), 2);
        assert!(z.by_name("Clase.class").is_ok());
        assert!(z.by_name("ok.mixins.json").is_ok());
    }

    #[test]
    fn no_toca_lo_que_esta_bien() {
        let jar = make_jar(r#"{"id":"novaclient","depends":{"fabric-api-base":"*"}}"#, &[]);
        assert!(patch_jar(&jar).unwrap().is_none());
    }
}

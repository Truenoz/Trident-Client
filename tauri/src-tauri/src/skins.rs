use crate::dl;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(5 * 60);
const MAX_BYTES: usize = 2 * 1024 * 1024;
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

type Memory = Mutex<HashMap<String, (Instant, Value)>>;

fn memory() -> &'static Memory {
    static M: OnceLock<Memory> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mem_get(key: &str, force: bool) -> Option<Value> {
    if force {
        return None;
    }
    memory().lock().ok()?.get(key).filter(|(at, _)| at.elapsed() < TTL).map(|(_, v)| v.clone())
}

fn mem_put(key: String, v: &Value) {
    if let Ok(mut m) = memory().lock() {
        m.insert(key, (Instant::now(), v.clone()));
    }
}

fn undash(s: &str) -> String {
    s.replace('-', "").to_lowercase()
}

fn is_uuid(s: &str) -> bool {
    let u = undash(s);
    u.len() == 32 && u.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_name(s: &str) -> bool {
    (1..=16).contains(&s.chars().count()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn api_base(default: &str, var: &str) -> String {
    if cfg!(test) {
        if let Ok(u) = std::env::var(var) {
            return u;
        }
    }
    default.to_string()
}

async fn get_json_opt(url: &str) -> Result<Option<Value>, String> {
    let res = dl::client()
        .get(url)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = res.status().as_u16();
    if st == 204 || st == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("HTTP {st}"));
    }
    res.json::<Value>().await.map(Some).map_err(|e| e.to_string())
}

async fn get_png(url: &str, host_ok: impl Fn(&str) -> bool) -> Result<String, String> {
    let mut u = reqwest::Url::parse(url).map_err(|_| "Origen de skin no permitido".to_string())?;
    if u.scheme() == "http" && !cfg!(test) {
        let _ = u.set_scheme("https");
    }
    let host = u.host_str().unwrap_or("").to_string();
    if !(u.scheme() == "https" || cfg!(test)) || !host_ok(&host) {
        return Err("Origen de skin no permitido".to_string());
    }
    let res = dl::client()
        .get(u)
        .header("Accept", "image/png")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("La skin es demasiado grande".to_string());
    }
    if bytes.len() < 8 || bytes[..8] != PNG_MAGIC {
        return Err("La skin no es un PNG válido".to_string());
    }
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

fn mojang_textures_host(h: &str) -> bool {
    let h = h.to_lowercase();
    h == "textures.minecraft.net" || h.ends_with(".textures.minecraft.net") || (cfg!(test) && h == "127.0.0.1")
}

fn mcheads_host(h: &str) -> bool {
    h.eq_ignore_ascii_case("mc-heads.net") || (cfg!(test) && h == "127.0.0.1")
}

async fn lookup_name(name: &str) -> Result<Option<(String, String)>, String> {
    let enc = reqwest::Url::parse_with_params("http://x/", &[("k", name)])
        .map(|u| u.query().unwrap_or("").trim_start_matches("k=").to_string())
        .unwrap_or_else(|_| name.to_string());
    let pick = |r: &Value| r["id"].as_str().map(|id| (undash(id), r["name"].as_str().unwrap_or(name).to_string()));
    match get_json_opt(&format!("{}/minecraft/profile/lookup/name/{enc}", api_base("https://api.minecraftservices.com", "NOVA_MCSERVICES"))).await {
        Ok(Some(r)) => {
            if let Some(x) = pick(&r) {
                return Ok(Some(x));
            }
        }
        Ok(None) => return Ok(None),
        Err(_) => {}
    }
    Ok(get_json_opt(&format!("{}/users/profiles/minecraft/{enc}", api_base("https://api.mojang.com", "NOVA_MOJANG")))
        .await?
        .and_then(|r| pick(&r)))
}

struct Textures {
    name: String,
    skin_url: Option<String>,
    model: &'static str,
}

async fn session_textures(uuid: &str) -> Result<Option<Textures>, String> {
    let url = format!("{}/session/minecraft/profile/{uuid}", api_base("https://sessionserver.mojang.com", "NOVA_SESSION"));
    let Some(p) = get_json_opt(&url).await? else { return Ok(None) };
    let mut t = Textures { name: p["name"].as_str().unwrap_or("").to_string(), skin_url: None, model: "classic" };
    let prop = p["properties"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some("textures")));
    if let Some(value) = prop.and_then(|x| x["value"].as_str()) {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|e| e.to_string())?;
        let j: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        t.skin_url = j["textures"]["SKIN"]["url"].as_str().map(|s| s.to_string());
        if j["textures"]["SKIN"]["metadata"]["model"].as_str() == Some("slim") {
            t.model = "slim";
        }
    }
    Ok(Some(t))
}

fn cache_path(dir: &Path, uuid: &str) -> PathBuf {
    dir.join(format!("{uuid}.json"))
}

fn read_disk(dir: &Path, uuid: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(cache_path(dir, uuid)).ok()?).ok()
}

fn write_disk(dir: &Path, uuid: &str, v: &Value) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(cache_path(dir, uuid), v.to_string());
}

fn offline_skin_file(user_dir: &Path, name: &str) -> PathBuf {
    user_dir.join("offline-skins").join(format!("{}.json", name.to_lowercase()))
}

pub fn offline_skin(user_dir: &Path, name: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(offline_skin_file(user_dir, name)).ok()?).ok()?;
    let skin = v["skin"].as_str()?.to_string();
    let model = if v["model"].as_str() == Some("slim") { "slim" } else { "classic" };
    Some(json!({ "name": name, "uuid": "", "model": model, "skin": skin, "source": "propia" }))
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?), u32::from_be_bytes(bytes[20..24].try_into().ok()?)))
}

pub fn set_offline_skin(user_dir: &Path, name: &str, data_url: &str, model: &str) -> Result<Value, String> {
    let b64 = data_url.strip_prefix("data:image/png;base64,").ok_or("La skin tiene que ser una imagen PNG.")?;
    if b64.len() > 200_000 {
        return Err("La imagen es demasiado grande para ser una skin.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| "No se pudo leer la imagen.".to_string())?;
    match png_size(&bytes) {
        Some((64, 64)) | Some((64, 32)) => {}
        Some((w, h)) => return Err(format!("La skin tiene que medir 64×64 o 64×32 píxeles (esta mide {w}×{h}).")),
        None => return Err("La skin tiene que ser una imagen PNG.".into()),
    }
    let model = if model == "slim" { "slim" } else { "classic" };
    let f = offline_skin_file(user_dir, name);
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&f, json!({ "skin": data_url, "model": model }).to_string()).map_err(|e| e.to_string())?;
    offline_skin(user_dir, name).ok_or_else(|| "No se pudo guardar la skin.".to_string())
}

pub fn write_menu_skin_files(dir: &Path, skin: Option<&Value>) {
    let wide = dir.join("menu_skin.png");
    let slim = dir.join("menu_skin_slim.png");
    let _ = std::fs::remove_file(&wide);
    let _ = std::fs::remove_file(&slim);
    let Some(v) = skin else { return };
    let Some(b64) = v["skin"].as_str().and_then(|s| s.strip_prefix("data:image/png;base64,")) else { return };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return };
    if png_size(&bytes) != Some((64, 64)) {
        return;
    }
    let target = if v["model"].as_str() == Some("slim") { slim } else { wide };
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(target, bytes);
}

pub async fn menu_skin(user_dir: &Path, name: &str) -> Option<Value> {
    if let Some(v) = offline_skin(user_dir, name) {
        return Some(v);
    }
    get_skin(&user_dir.join("skin-cache"), name, false, None).await.ok().filter(|v| v["skin"].is_string())
}

pub fn clear_offline_skin(user_dir: &Path, name: &str) {
    let _ = std::fs::remove_file(offline_skin_file(user_dir, name));
}

pub struct Known {
    pub name: String,
    pub skin_url: Option<String>,
    pub slim: bool,
}

pub async fn get_skin(cache_dir: &Path, query: &str, force: bool, known: Option<&Known>) -> Result<Value, String> {
    let query = query.trim();
    let mut uuid = if is_uuid(query) { Some(undash(query)) } else { None };
    let mut name = if uuid.is_some() { known.map(|k| k.name.clone()).unwrap_or_default() } else { query.to_string() };

    if uuid.is_none() {
        if !is_name(query) {
            return Err("Nombre de jugador no válido.".to_string());
        }
        if let Some(v) = mem_get(&format!("n:{}", query.to_lowercase()), force) {
            return Ok(v);
        }
        match lookup_name(query).await {
            Ok(Some((id, n))) => {
                uuid = Some(id);
                name = n;
            }
            other => {

                let stale = memory().lock().ok().and_then(|m| {
                    m.values().map(|(_, v)| v.clone()).find(|v| v["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(query)))
                });
                if let Some(v) = stale {
                    return Ok(v);
                }
                return Err(match other {
                    Ok(_) => format!("No se encontró al jugador «{query}»."),
                    Err(_) => format!("No se encontró al jugador «{query}»."),
                });
            }
        }
    }
    let uuid = uuid.expect("se resolvió arriba");
    if let Some(v) = mem_get(&uuid, force) {
        return Ok(v);
    }

    let mut result = json!({ "name": name, "uuid": uuid, "model": "auto", "skin": null, "source": "" });
    let mut errors: Vec<String> = Vec::new();

    if let Some(url) = known.and_then(|k| k.skin_url.as_deref()) {
        match get_png(url, mojang_textures_host).await {
            Ok(png) => {
                result["skin"] = json!(png);
                result["model"] = json!(if known.is_some_and(|k| k.slim) { "slim" } else { "classic" });
                result["source"] = json!("sesión");
            }
            Err(e) => errors.push(e),
        }
    }

    if result["skin"].is_null() {
        match session_textures(&uuid).await {
            Ok(Some(t)) => {
                if !t.name.is_empty() {
                    result["name"] = json!(t.name);
                }
                if let Some(url) = t.skin_url {
                    match get_png(&url, mojang_textures_host).await {
                        Ok(png) => {
                            result["skin"] = json!(png);
                            result["model"] = json!(t.model);
                            result["source"] = json!("mojang");
                        }
                        Err(e) => errors.push(e),
                    }
                }
            }
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }

    if result["skin"].is_null() {
        match get_png(&format!("{}/skin/{uuid}", api_base("https://mc-heads.net", "NOVA_MCHEADS")), mcheads_host).await {
            Ok(png) => {
                result["skin"] = json!(png);
                result["model"] = json!("auto");
                result["source"] = json!("mc-heads");
            }
            Err(e) => errors.push(e),
        }
    }

    if result["skin"].is_null() {
        if let Some(mut disk) = read_disk(cache_dir, &uuid).filter(|d| d["skin"].is_string()) {
            disk["source"] = json!("caché");
            return Ok(disk);
        }
        let why = if errors.is_empty() { "sin conexión".to_string() } else { errors.join("; ") };
        return Err(format!("No se pudo obtener la skin ({why})."));
    }

    if result["name"].as_str().is_none_or(|n| n.is_empty()) {
        result["name"] = json!(known.map(|k| k.name.clone()).unwrap_or_default());
    }
    mem_put(uuid.clone(), &result);
    if let Some(n) = result["name"].as_str().filter(|n| !n.is_empty()) {
        mem_put(format!("n:{}", n.to_lowercase()), &result);
    }
    write_disk(cache_dir, &uuid, &result);
    Ok(result)
}

#[cfg(test)]
mod offline_skin_tests {
    use super::*;

    fn png(w: u32, h: u32) -> String {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b))
    }

    #[test]
    fn guarda_y_borra_la_skin_no_premium() {
        let dir = std::env::temp_dir().join(format!("trident-oskin-{}", std::process::id()));
        assert!(offline_skin(&dir, "Dave").is_none());
        assert!(set_offline_skin(&dir, "Dave", &png(128, 128), "classic").is_err());
        assert!(set_offline_skin(&dir, "Dave", "data:image/jpeg;base64,AAAA", "classic").is_err());
        let v = set_offline_skin(&dir, "Dave", &png(64, 64), "slim").unwrap();
        assert_eq!(v["model"], "slim");
        assert_eq!(offline_skin(&dir, "dave").unwrap()["source"], "propia");
        assert!(set_offline_skin(&dir, "Dave", &png(64, 32), "x").is_ok());
        assert_eq!(offline_skin(&dir, "DAVE").unwrap()["model"], "classic");
        let out = dir.join("cfg");
        set_offline_skin(&dir, "Dave", &png(64, 64), "slim").unwrap();
        write_menu_skin_files(&out, offline_skin(&dir, "Dave").as_ref());
        assert!(out.join("menu_skin_slim.png").exists() && !out.join("menu_skin.png").exists());
        set_offline_skin(&dir, "Dave", &png(64, 32), "classic").unwrap();
        write_menu_skin_files(&out, offline_skin(&dir, "Dave").as_ref());
        assert!(!out.join("menu_skin_slim.png").exists() && !out.join("menu_skin.png").exists());
        set_offline_skin(&dir, "Dave", &png(64, 64), "classic").unwrap();
        write_menu_skin_files(&out, offline_skin(&dir, "Dave").as_ref());
        assert!(out.join("menu_skin.png").exists());
        write_menu_skin_files(&out, None);
        assert!(!out.join("menu_skin.png").exists());
        clear_offline_skin(&dir, "Dave");
        assert!(offline_skin(&dir, "Dave").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn png() -> Vec<u8> {
        let mut b = PNG_MAGIC.to_vec();
        b.extend_from_slice(&[0u8; 32]);
        b
    }

    #[test]
    fn valida_consultas() {
        assert!(is_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5"));
        assert!(is_uuid("069A79F444E94726A5BEFCA90E38AAF5"));
        assert!(!is_uuid("Notch"));
        assert!(is_name("Notch_1"));
        assert!(!is_name("nombre-demasiado-largo-y-raro"));
        assert!(!is_name(""));
        assert!(mojang_textures_host("textures.minecraft.net"));
        assert!(!mojang_textures_host("evil.com"));
        assert!(!mojang_textures_host("textures.minecraft.net.evil.com"));
        assert!(mcheads_host("mc-heads.net"));
    }

    #[tokio::test]
    async fn busca_por_nombre_con_servidores_falsos() {
        std::env::set_var("NO_PROXY", "127.0.0.1");
        std::env::set_var("no_proxy", "127.0.0.1");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let b2 = base.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { continue };
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (ctype, body): (&str, Vec<u8>) = if path.starts_with("/minecraft/profile/lookup/name/Steve") {
                    ("application/json", br#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Steve"}"#.to_vec())
                } else if path.starts_with("/session/minecraft/profile/") {
                    let tex = serde_json::json!({ "textures": { "SKIN": { "url": format!("{b2}/tex/abc"), "metadata": { "model": "slim" } } } });
                    let v = base64::engine::general_purpose::STANDARD.encode(tex.to_string());
                    ("application/json", serde_json::json!({ "name": "Steve", "properties": [{ "name": "textures", "value": v }] }).to_string().into_bytes())
                } else if path.starts_with("/tex/abc") {
                    ("image/png", png())
                } else {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                let _ = s.write_all(&body);
            }
        });
        std::env::set_var("NOVA_MCSERVICES", &base);
        std::env::set_var("NOVA_SESSION", &base);
        let dir = std::env::temp_dir().join(format!("nova-skin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let r = get_skin(&dir, "Steve", true, None).await.unwrap();
        assert_eq!(r["uuid"], "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(r["model"], "slim");
        assert_eq!(r["source"], "mojang");
        assert!(r["skin"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(cache_path(&dir, "069a79f444e94726a5befca90e38aaf5").exists());

        assert!(get_skin(&dir, "NoExiste", true, None).await.unwrap_err().contains("No se encontró"));
        assert!(get_skin(&dir, "nombre inválido!", true, None).await.is_err());

        let known = Known { name: "Steve".into(), skin_url: Some(format!("{base}/tex/abc")), slim: false };
        let r = get_skin(&dir, "069a79f4-44e9-4726-a5be-fca90e38aaf5", true, Some(&known)).await.unwrap();
        assert_eq!(r["source"], "sesión");
        assert_eq!(r["model"], "classic");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

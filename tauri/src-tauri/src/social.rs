use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use crate::{account, dl, game, nova, settings, AppState};

const DEFAULT_URL: &str = "https://api.tridentclient.com";
const MOJANG_JOIN: &str = "https://sessionserver.mojang.com/session/minecraft/join";
const ACTIONS: [&str; 5] = ["request", "accept", "decline", "cancel", "remove"];
const LINKS_FILE: &str = "trident-net.bin";
pub const NEED_LINK: &str = "Vinculá tu cuenta no premium con Trident Net (pestaña Cosméticos) para usar amigos y cosméticos.";

struct Cached {
    account: String,
    token: String,
    until: Instant,
}

static TOKEN: Mutex<Option<Cached>> = Mutex::new(None);
static ONLINE: AtomicI64 = AtomicI64::new(-1);

fn install_id(user_dir: &Path) -> String {
    let path = user_dir.join("install-id");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let t = text.trim().to_lowercase();
        if t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            return t;
        }
    }
    let bytes: [u8; 16] = rand::random();
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let _ = std::fs::create_dir_all(user_dir);
    let _ = std::fs::write(&path, &id);
    id
}

pub fn online_count() -> Option<i64> {
    let n = ONLINE.load(Ordering::Relaxed);
    (n >= 0).then_some(n)
}

pub fn start_online(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let user_dir = app.state::<AppState>().user_dir.clone();
        let id = install_id(&user_dir);
        loop {
            if let Ok(url) = base_url(&app) {
                if let Ok((200, v)) = post(&format!("{url}/v1/stats/ping"), None, &json!({ "id": id })).await {
                    if let Some(n) = v["online"].as_i64() {
                        ONLINE.store(n, Ordering::Relaxed);
                        let _ = tauri::Emitter::emit(&app, "online:count", n);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

fn base_url(app: &AppHandle) -> Result<String, String> {
    let user_dir = app.state::<AppState>().user_dir.clone();
    let mut url = nova::net_url(&settings::read(&user_dir));
    if url.is_empty() {
        url = DEFAULT_URL.to_string();
    }
    let url = url.trim_end_matches('/').to_string();
    if !url.starts_with("http") || url.contains(char::is_whitespace) {
        return Err("La dirección del servidor de amigos no es válida.".into());
    }
    Ok(url)
}

fn message(code: &str) -> String {
    match code {
        "not_found" => "Ese jugador todavía no usa Trident Client.",
        "self" => "No podés agregarte a vos mismo.",
        "invalid_name" => "Ese nombre de Minecraft no es válido.",
        "already_friends" => "Ya son amigos.",
        "already_requested" => "Ya le mandaste una solicitud.",
        "too_many_friends" => "Llegaste al límite de amigos.",
        "too_many_pending" => "Hay demasiadas solicitudes pendientes.",
        "no_request" => "Esa solicitud ya no existe.",
        "not_friends" => "Solo podés escribirle a tus amigos.",
        "rate_limited" => "Vas muy rápido. Probá de nuevo en un rato.",
        "empty" => "El mensaje está vacío.",
        "unauthorized" => "No se pudo verificar tu cuenta.",
        "not_enough_coins" => "No te alcanzan las Trident Coins.",
        "unknown_item" => "Ese cosmético ya no está en la tienda.",
        "not_owned" => "Todavía no tenés ese cosmético.",
        "game_only" => "Las coins del día se reclaman abriendo Minecraft con Trident Client.",
        "weak_password" => "La contraseña tiene que tener entre 6 y 72 caracteres.",
        "name_taken" => "Ese nombre ya tiene contraseña en Trident Net. Si es tuyo, entrá con tu contraseña.",
        "premium_name" => "Ese nombre es de una cuenta premium. Usá otro nombre o entrá con Microsoft.",
        "bad_password" => "Contraseña incorrecta.",
        "no_account" => "Ese nombre todavía no tiene contraseña. Creala primero.",
        "device_unknown" => NEED_LINK,
        "no_route" => "Trident Net está desactualizado: falta subir el server.js nuevo a Railway.",
        "server_error" => "Trident Net tuvo un error interno. Probá de nuevo en un rato.",
        "" => "El servidor de amigos respondió con un error.",
        other => return format!("El servidor de amigos respondió con un error ({other})."),
    }
    .to_string()
}

fn cached(account: &str) -> Option<String> {
    let guard = TOKEN.lock().ok()?;
    let c = guard.as_ref()?;
    (c.account == account && Instant::now() < c.until).then(|| c.token.clone())
}

fn store(account: String, token: String, secs: u64) {
    if let Ok(mut guard) = TOKEN.lock() {
        *guard = Some(Cached { account, token, until: Instant::now() + Duration::from_secs(secs) });
    }
}

fn forget() {
    if let Ok(mut guard) = TOKEN.lock() {
        *guard = None;
    }
}

async fn post(url: &str, token: Option<&str>, body: &Value) -> Result<(u16, Value), String> {
    let mut req = dl::client().post(url).json(body).timeout(Duration::from_secs(10));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.map_err(|_| "No se pudo conectar con el servidor de amigos.".to_string())?;
    let status = r.status().as_u16();
    let value = r.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, value))
}

fn links_read(user_dir: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(user_dir.join(LINKS_FILE))
        .ok()
        .and_then(|raw| account::unseal(&raw))
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn link_get(user_dir: &Path, name: &str) -> Option<String> {
    links_read(user_dir).get(&name.to_lowercase())?.as_str().map(String::from)
}

fn link_set(user_dir: &Path, name: &str, device: Option<&str>) {
    let mut map = links_read(user_dir);
    match device {
        Some(d) => {
            map.insert(name.to_lowercase(), Value::String(d.to_string()));
        }
        None => {
            map.remove(&name.to_lowercase());
        }
    }
    let _ = std::fs::create_dir_all(user_dir);
    if let Err(e) = account::write_private(&user_dir.join(LINKS_FILE), &account::seal(Value::Object(map).to_string().as_bytes())) {
        eprintln!("No se pudo guardar la cuenta de Trident Net: {e}");
    }
}

pub fn game_link(user_dir: &Path, inst: &Path, s: &account::Session) {
    let f = inst.join("config").join("tridentclient").join("net-offline.json");
    match if s.offline { link_get(user_dir, &s.name) } else { None } {
        Some(device) => {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&f, json!({ "name": s.name, "device": device }).to_string());
        }
        None => {
            let _ = std::fs::remove_file(&f);
        }
    }
}

fn offline_key(name: &str) -> String {
    format!("offline:{}", name.to_lowercase())
}

async fn offline_token(app: &AppHandle, base: &str, s: &account::Session) -> Result<String, String> {
    let key = offline_key(&s.name);
    if let Some(t) = cached(&key) {
        return Ok(t);
    }
    let user_dir = app.state::<AppState>().user_dir.clone();
    let device = link_get(&user_dir, &s.name).ok_or(NEED_LINK)?;
    let (status, r) = post(&format!("{base}/v1/auth/offline/device"), None, &json!({ "name": s.name, "device": device })).await?;
    if status == 401 {
        link_set(&user_dir, &s.name, None);
        return Err(NEED_LINK.into());
    }
    let token = r["token"].as_str().filter(|_| status == 200).ok_or_else(|| message(r["error"].as_str().unwrap_or("")))?.to_string();
    let secs = r["expiresIn"].as_u64().unwrap_or(3600).saturating_sub(120).max(60);
    store(key, token.clone(), secs);
    Ok(token)
}

pub fn offline_status(app: &AppHandle) -> Value {
    match game::get_session(app) {
        Some(s) if s.offline => {
            let user_dir = app.state::<AppState>().user_dir.clone();
            json!({ "offline": true, "name": s.name, "linked": link_get(&user_dir, &s.name).is_some() })
        }
        Some(s) => json!({ "offline": false, "name": s.name, "linked": true }),
        None => json!({ "offline": false, "name": "", "linked": false }),
    }
}

pub async fn offline_link(app: &AppHandle, password: &str, register: bool) -> Result<Value, String> {
    let s = game::get_session(app).ok_or("Iniciá sesión primero.")?;
    if !s.offline {
        return Err("Tu cuenta es premium: no necesita contraseña.".into());
    }
    let base = base_url(app)?;
    let path = if register { "/v1/auth/offline/register" } else { "/v1/auth/offline/login" };
    let (status, r) = post(&format!("{base}{path}"), None, &json!({ "name": s.name, "password": password })).await?;
    if status != 200 {
        return Err(message(r["error"].as_str().unwrap_or("")));
    }
    let device = r["device"].as_str().filter(|d| d.len() == 64).ok_or("El servidor de amigos no respondió bien.")?;
    let user_dir = app.state::<AppState>().user_dir.clone();
    link_set(&user_dir, &s.name, Some(device));
    if let Some(t) = r["token"].as_str() {
        let secs = r["expiresIn"].as_u64().unwrap_or(3600).saturating_sub(120).max(60);
        store(offline_key(&s.name), t.to_string(), secs);
    }
    Ok(json!({ "ok": true, "name": r["name"] }))
}

pub fn offline_unlink(app: &AppHandle) {
    if let Some(s) = game::get_session(app) {
        let user_dir = app.state::<AppState>().user_dir.clone();
        link_set(&user_dir, &s.name, None);
    }
    forget();
}

async fn token(app: &AppHandle, base: &str) -> Result<String, String> {
    let s = game::get_session(app).ok_or("Iniciá sesión para usar amigos.")?;
    if s.offline {
        return offline_token(app, base, &s).await;
    }
    let account = s.uuid_plain();
    if let Some(t) = cached(&account) {
        return Ok(t);
    }
    let (status, challenge) = post(&format!("{base}/v1/auth/challenge"), None, &json!({})).await?;
    let nonce = challenge["nonce"]
        .as_str()
        .filter(|_| status == 200)
        .ok_or("El servidor de amigos no respondió bien.")?
        .to_string();
    let joined = dl::client()
        .post(MOJANG_JOIN)
        .json(&json!({ "accessToken": s.access_token, "selectedProfile": account, "serverId": nonce }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "No se pudo conectar con Mojang.".to_string())?;
    if !joined.status().is_success() {
        return Err("Mojang no aceptó tu sesión. Cerrá sesión y volvé a entrar.".into());
    }
    let (status, login) = post(&format!("{base}/v1/auth/login"), None, &json!({ "name": s.name, "nonce": nonce })).await?;
    let token = login["token"]
        .as_str()
        .filter(|_| status == 200)
        .ok_or("No se pudo verificar tu cuenta.")?
        .to_string();
    let secs = login["expiresIn"].as_u64().unwrap_or(3600).saturating_sub(120).max(60);
    store(account, token.clone(), secs);
    Ok(token)
}

async fn call(app: &AppHandle, path: &str, body: Value) -> Result<Value, String> {
    let base = base_url(app)?;
    for attempt in 0..2 {
        let t = token(app, &base).await?;
        let (status, value) = post(&format!("{base}{path}"), Some(&t), &body).await?;
        if status == 401 && attempt == 0 {
            forget();
            continue;
        }
        if (200..300).contains(&status) {
            return Ok(value);
        }
        return Err(message(value["error"].as_str().unwrap_or("")));
    }
    Err(message("unauthorized"))
}

pub async fn sync(app: &AppHandle, after: f64) -> Result<Value, String> {
    call(app, "/v1/sync", json!({ "after": after, "client": "launcher", "names": [] })).await
}

pub async fn action(app: &AppHandle, action: &str, name: &str) -> Result<Value, String> {
    if !ACTIONS.contains(&action) {
        return Err("Acción no válida.".into());
    }
    call(app, &format!("/v1/friends/{action}"), json!({ "name": name.trim() })).await
}

pub async fn shop(app: &AppHandle, op: &str, id: &str, on: bool) -> Result<Value, String> {
    match op {
        "buy" => call(app, "/v1/shop/buy", json!({ "id": id.trim() })).await,
        "equip" => call(app, "/v1/shop/equip", json!({ "id": id.trim(), "on": on })).await,
        _ => Err("Acción no válida.".into()),
    }
}

pub async fn send(app: &AppHandle, to: &str, text: &str) -> Result<Value, String> {
    call(app, "/v1/msg", json!({ "to": to.trim(), "text": text })).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensajes_de_error_en_castellano() {
        assert_eq!(message("not_found"), "Ese jugador todavía no usa Trident Client.");
        assert_eq!(message("algo_raro"), "El servidor de amigos respondió con un error (algo_raro).");
        assert_eq!(message(""), "El servidor de amigos respondió con un error.");
    }

    #[test]
    fn los_vinculos_no_premium_se_guardan_por_nombre() {
        let dir = std::env::temp_dir().join(format!("trident-links-{}", std::process::id()));
        assert_eq!(link_get(&dir, "Dave"), None);
        link_set(&dir, "Dave", Some(&"a".repeat(64)));
        assert_eq!(link_get(&dir, "dave").as_deref(), Some("a".repeat(64).as_str()));
        link_set(&dir, "DAVE", None);
        assert_eq!(link_get(&dir, "Dave"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn el_token_se_guarda_por_cuenta() {
        store("aaa".into(), "t1".into(), 60);
        assert_eq!(cached("aaa").as_deref(), Some("t1"));
        assert_eq!(cached("bbb"), None);
        forget();
        assert_eq!(cached("aaa"), None);
    }
}

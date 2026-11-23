use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use crate::{dl, game, nova, settings, AppState};

const DEFAULT_URL: &str = "https://api.tridentclient.com";
const MOJANG_JOIN: &str = "https://sessionserver.mojang.com/session/minecraft/join";
const ACTIONS: [&str; 5] = ["request", "accept", "decline", "cancel", "remove"];

struct Cached {
    account: String,
    token: String,
    until: Instant,
}

static TOKEN: Mutex<Option<Cached>> = Mutex::new(None);

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
        _ => "El servidor de amigos respondió con un error.",
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

async fn token(app: &AppHandle, base: &str) -> Result<String, String> {
    let s = game::get_session(app).ok_or("Iniciá sesión para usar amigos.")?;
    if s.offline {
        return Err("Los amigos necesitan una cuenta de Microsoft: las cuentas sin conexión no se pueden verificar.".into());
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
        assert_eq!(message("algo_raro"), "El servidor de amigos respondió con un error.");
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

use crate::dl;
use base64::Engine;
use md5::{Digest, Md5};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const CLIENT_ID: &str = "00000000402b5328";
const REDIRECT: &str = "https://login.live.com/oauth20_desktop.srf";
const TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";

const REJECTED: &str = "Microsoft rechazó el inicio de sesión";
const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Clone, Debug)]
pub struct Session {
    pub offline: bool,
    pub name: String,

    pub id: String,
    pub access_token: String,
    pub xuid: String,

    pub user_type: String,
    pub skins: Value,
    pub refresh_token: Option<String>,
}

impl Session {

    pub fn info(&self) -> Value {
        json!({ "name": self.name, "uuid": self.id, "offline": self.offline })
    }

    pub fn uuid_plain(&self) -> String {
        self.id.replace('-', "")
    }
}

pub fn offline_uuid(name: &str) -> String {
    let mut d: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    d[6] = (d[6] & 0x0f) | 0x30;
    d[8] = (d[8] & 0x3f) | 0x80;
    let h = hex::encode(d);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub fn offline_session(raw_name: &str) -> Result<Session, String> {
    let name = raw_name.trim().to_string();
    let valid = (3..=16).contains(&name.chars().count()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err("El nombre debe tener entre 3 y 16 caracteres: letras, números o guion bajo.".to_string());
    }
    Ok(Session {
        offline: true,
        id: offline_uuid(&name),
        name,
        access_token: "0".to_string(),
        xuid: String::new(),
        user_type: "mojang".to_string(),
        skins: json!([]),
        refresh_token: None,
    })
}

async fn oauth_code(app: &AppHandle) -> Result<String, String> {
    if let Some(old) = app.get_webview_window("ms-login") {
        let _ = old.close();
    }
    let url = format!(
        "https://login.live.com/oauth20_authorize.srf?client_id={CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&prompt=select_account&mkt=es-ES",
        urlenc(REDIRECT),
        urlenc(SCOPE)
    );
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{e}"))?;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let send = {
        let tx = tx.clone();
        move |r: Result<String, String>| {
            if let Some(t) = tx.lock().ok().and_then(|mut g| g.take()) {
                let _ = t.send(r);
            }
        }
    };
    let on_nav = send.clone();

    let user_dir = app.state::<crate::AppState>().user_dir.clone();
    let args = crate::system::session_browser_args(&crate::settings::read(&user_dir));
    let win = WebviewWindowBuilder::new(app, "ms-login", WebviewUrl::External(parsed))
        .additional_browser_args(args)
        .title("Iniciar sesión con Microsoft")
        .inner_size(520.0, 700.0)
        .center()
        .on_navigation(move |u| {
            if u.as_str().starts_with(REDIRECT) {
                let find = |key: &str| u.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
                match find("code") {
                    Some(code) => on_nav(Ok(code)),
                    None => on_nav(Err(find("error_description")
                        .unwrap_or_else(|| "Microsoft no devolvió ningún código.".to_string()))),
                }
                false
            } else {
                true
            }
        })
        .build()
        .map_err(|e| format!("No se pudo abrir la ventana de Microsoft: {e}"))?;

    let on_close = send.clone();
    win.on_window_event(move |e| {
        if let tauri::WindowEvent::Destroyed = e {
            on_close(Err("Inicio de sesión cancelado.".to_string()));
        }
    });

    let result = rx.await.map_err(|_| "Inicio de sesión cancelado.".to_string())?;
    let _ = win.close();
    result
}

fn urlenc(s: &str) -> String {
    reqwest::Url::parse_with_params("http://x/", &[("v", s)])
        .ok()
        .and_then(|u| u.query().map(|q| q.trim_start_matches("v=").replace('+', "%20")))
        .unwrap_or_default()
}

async fn ms_token(form: &[(&str, &str)]) -> Result<(String, String), String> {
    let res = dl::client()
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("No se pudo conectar con Microsoft: {e}"))?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("sin detalle");
        return Err(format!("{REJECTED} (HTTP {}): {}", status.as_u16(), why));
    }
    let access = body["access_token"].as_str().ok_or("Microsoft no devolvió el token de acceso.")?.to_string();
    let refresh = body["refresh_token"].as_str().unwrap_or("").to_string();
    Ok((access, refresh))
}

async fn post_json(url: &str, body: Value, what: &str) -> Result<(u16, Value), String> {
    let res = dl::client()
        .post(url)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{what}: {e}"))?;
    let status = res.status().as_u16();
    let json: Value = res.json().await.unwrap_or(Value::Null);
    Ok((status, json))
}

pub fn is_rejected(err: &str) -> bool {
    err.starts_with(REJECTED)
}

pub fn xsts_error(code: u64) -> &'static str {
    match code {
        2148916233 => "Esta cuenta de Microsoft no tiene un perfil de Xbox. Creá uno en xbox.com y volvé a intentar.",
        2148916235 => "Xbox Live no está disponible en tu país.",
        2148916236 | 2148916237 => "La cuenta necesita una verificación de edad de adulto (Corea del Sur).",
        2148916238 => "Es una cuenta de menor de edad: hay que agregarla a un grupo familiar de Microsoft.",
        _ => "Xbox Live rechazó la cuenta.",
    }
}

async fn minecraft_session(ms_access: &str, ms_refresh: String) -> Result<Session, String> {
    let (st, xbl) = post_json(
        "https://user.auth.xboxlive.com/user/authenticate",
        json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_access}") },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }),
        "Xbox Live",
    )
    .await?;
    let xbl_token = xbl["Token"].as_str().ok_or_else(|| format!("Xbox Live rechazó el inicio de sesión (HTTP {st})."))?;

    let (st, xsts) = post_json(
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }),
        "XSTS",
    )
    .await?;
    if let Some(code) = xsts["XErr"].as_u64() {
        return Err(xsts_error(code).to_string());
    }
    let xsts_token = xsts["Token"].as_str().ok_or_else(|| format!("XSTS rechazó el inicio de sesión (HTTP {st})."))?;
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().ok_or("XSTS no devolvió el identificador de usuario.")?;

    let (st, mc) = post_json(
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }),
        "Minecraft",
    )
    .await?;
    let mc_token = mc["access_token"]
        .as_str()
        .ok_or_else(|| format!("Minecraft rechazó el inicio de sesión (HTTP {st})."))?
        .to_string();

    let res = dl::client()
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&mc_token)
        .send()
        .await
        .map_err(|e| format!("Perfil de Minecraft: {e}"))?;
    let profile: Value = res.json().await.unwrap_or(Value::Null);
    let (id, name) = match (profile["id"].as_str(), profile["name"].as_str()) {
        (Some(id), Some(name)) => (id.to_string(), name.to_string()),
        _ => return Err("Esta cuenta no tiene Minecraft: Java Edition. Comprala en minecraft.net o usá «Jugar sin cuenta».".to_string()),
    };

    Ok(Session {
        offline: false,
        id,
        name,
        xuid: jwt_xuid(&mc_token),
        access_token: mc_token,
        user_type: "msa".to_string(),
        skins: profile["skins"].clone(),
        refresh_token: if ms_refresh.is_empty() { None } else { Some(ms_refresh) },
    })
}

fn jwt_xuid(token: &str) -> String {
    token
        .split('.')
        .nth(1)
        .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|j| j["xuid"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

pub async fn login(app: &AppHandle) -> Result<Session, String> {
    let code = oauth_code(app).await?;
    let (access, refresh) = ms_token(&[
        ("client_id", CLIENT_ID),
        ("code", &code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", REDIRECT),
    ])
    .await?;
    minecraft_session(&access, refresh).await
}

pub async fn refresh(refresh_token: &str) -> Result<Session, String> {
    let (access, new_refresh) = ms_token(&[
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
        ("scope", SCOPE),
    ])
    .await?;
    let keep = if new_refresh.is_empty() { refresh_token.to_string() } else { new_refresh };
    minecraft_session(&access, keep).await
}

pub fn save_refresh(user_dir: &Path, token: &str) {
    let blob = match protect(token.as_bytes()) {
        Some(b) => [b"N1:".as_slice(), &b].concat(),
        None => [b"P1:".as_slice(), token.as_bytes()].concat(),
    };
    let _ = std::fs::create_dir_all(user_dir);
    if let Err(e) = write_private(&user_dir.join("account.bin"), &blob) {
        eprintln!("No se pudo guardar la sesión: {e}");
    }
}

pub fn load_refresh(user_dir: &Path) -> Option<String> {
    let raw = std::fs::read(user_dir.join("account.bin")).ok()?;
    if raw.len() < 3 {
        return None;
    }
    let (tag, body) = raw.split_at(3);
    match tag {
        b"N1:" => unprotect(body).and_then(|b| String::from_utf8(b).ok()),
        b"P1:" => String::from_utf8(body.to_vec()).ok(),
        _ => None,
    }
}

pub fn clear_refresh(user_dir: &Path) {
    let _ = std::fs::remove_file(user_dir.join("account.bin"));
}

pub fn save_offline(user_dir: &Path, name: &str) {
    let _ = std::fs::create_dir_all(user_dir);
    if let Err(e) = std::fs::write(user_dir.join("offline.json"), json!({ "name": name }).to_string()) {
        eprintln!("No se pudo guardar la sesión sin cuenta: {e}");
    }
}

pub fn load_offline(user_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(user_dir.join("offline.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v["name"].as_str().map(|n| n.to_string())
}

pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(data)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

pub fn seal(data: &[u8]) -> Vec<u8> {
    match protect(data) {
        Some(b) => [b"N1:".as_slice(), &b].concat(),
        None => [b"P1:".as_slice(), data].concat(),
    }
}

pub fn unseal(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() < 3 {
        return None;
    }
    let (tag, body) = raw.split_at(3);
    match tag {
        b"N1:" => unprotect(body),
        b"P1:" => Some(body.to_vec()),
        _ => None,
    }
}

pub fn clear_offline(user_dir: &Path) {
    let _ = std::fs::remove_file(user_dir.join("offline.json"));
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out);
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut core::ffi::c_void);
        Some(v)
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out);
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut core::ffi::c_void);
        Some(v)
    }
}

#[cfg(not(windows))]
fn protect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_offline_igual_que_vanilla() {

        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(offline_uuid("Steve"), "5627dd98-e6be-3c21-b8a8-e92344183641");
    }

    #[test]
    fn sesion_sin_cuenta_se_guarda() {
        let dir = std::env::temp_dir().join(format!("nova-offline-{}", std::process::id()));
        assert_eq!(load_offline(&dir), None);
        save_offline(&dir, "Nova_01");
        assert_eq!(load_offline(&dir).as_deref(), Some("Nova_01"));
        clear_offline(&dir);
        assert_eq!(load_offline(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nombre_offline_valido() {
        assert!(offline_session("ab").is_err());
        assert!(offline_session("tiene espacio").is_err());
        assert!(offline_session("muy_largo_muy_largo_x").is_err());
        assert!(offline_session("ñandú").is_err());
        let s = offline_session("  Nova_01 ").unwrap();
        assert_eq!(s.name, "Nova_01");
        assert!(s.offline);
        assert_eq!(s.uuid_plain().len(), 32);
        assert_eq!(s.user_type, "mojang");
    }

    #[test]
    fn xuid_del_jwt() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"xuid":"2535400000000000","x":1}"#);
        assert_eq!(jwt_xuid(&format!("aaa.{payload}.bbb")), "2535400000000000");
        assert_eq!(jwt_xuid("sin-puntos"), "");
    }

    #[test]
    fn urlencode() {
        assert_eq!(urlenc("https://login.live.com/x.srf"), "https%3A%2F%2Flogin.live.com%2Fx.srf");
        assert_eq!(urlenc("XboxLive.signin offline_access"), "XboxLive.signin%20offline_access");
    }

    #[test]
    fn guardar_y_leer_sesion() {
        let dir = std::env::temp_dir().join(format!("nova-test-{}", std::process::id()));
        save_refresh(&dir, "M.R3_BAY.token-de-prueba");
        assert_eq!(load_refresh(&dir).as_deref(), Some("M.R3_BAY.token-de-prueba"));
        clear_refresh(&dir);
        assert_eq!(load_refresh(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager};

use crate::AppState;

pub const SCHEME: &str = "art";
static REFRESHING: AtomicBool = AtomicBool::new(false);
static PREFETCHING: AtomicBool = AtomicBool::new(false);

fn remote_file(user_dir: &Path) -> PathBuf {
    user_dir.join("version-art.json")
}

fn art_dir(user_dir: &Path) -> PathBuf {
    user_dir.join("version-art")
}

fn read_remote(user_dir: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(remote_file(user_dir)).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(m) if !m.is_empty() => Some(m),
        _ => None,
    }
}

fn fnv(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn safe_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 32 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !v.contains("..")
}

fn file_for(user_dir: &Path, version: &str, url: &str) -> PathBuf {
    art_dir(user_dir).join(format!("{version}-{:016x}.img", fnv(url)))
}

fn local_url(version: &str) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{version}")
    } else {
        format!("{SCHEME}://localhost/{version}")
    }
}

fn to_local(remote: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for (v, u) in remote {
        if safe_version(v) && u.is_string() {
            out.insert(v.clone(), Value::String(local_url(v)));
        }
    }
    Value::Object(out)
}

fn client(app_version: &str) -> Result<reqwest::Client, String> {
    crate::dl::tls_init();
    reqwest::Client::builder()
        .user_agent(format!("TridentClient/{app_version}"))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch_image(app_version: &str, user_dir: &Path, version: &str, url: &str) -> Result<Vec<u8>, String> {
    let path = file_for(user_dir, version, url);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }
    let res = client(app_version)?.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?.to_vec();
    if bytes.is_empty() {
        return Err("vacío".into());
    }
    let dir = art_dir(user_dir);
    let _ = tokio::fs::create_dir_all(&dir).await;
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        let prefix = format!("{version}-");
        while let Ok(Some(e)) = entries.next_entry().await {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && e.path() != path {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    let tmp = path.with_extension("part");
    if tokio::fs::write(&tmp, &bytes).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, &path).await;
    }
    Ok(bytes)
}

fn mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn prefetch(app: &AppHandle, remote: Map<String, Value>) {
    if PREFETCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let user_dir = app.state::<AppState>().user_dir.clone();
        let app_version = app.package_info().version.to_string();
        let mut wanted: Vec<(String, String)> = remote
            .iter()
            .filter(|(v, _)| safe_version(v) && crate::net::is_allowed(v))
            .filter_map(|(v, u)| u.as_str().map(|u| (v.clone(), u.to_string())))
            .collect();
        wanted.sort_by(|a, b| b.0.cmp(&a.0));
        for chunk in wanted.chunks(4) {
            let jobs = chunk.iter().map(|(v, u)| fetch_image(&app_version, &user_dir, v, u));
            futures_util::future::join_all(jobs).await;
        }
        PREFETCHING.store(false, Ordering::SeqCst);
    });
}

async fn refresh(app: &AppHandle) -> Result<Map<String, Value>, String> {
    let user_dir = app.state::<AppState>().user_dir.clone();
    let fresh = crate::net::version_art(&app.package_info().version.to_string()).await?;
    let Value::Object(map) = fresh else { return Err("respuesta inválida".into()) };
    if map.is_empty() {
        return Err("sin imágenes".into());
    }
    let _ = std::fs::write(remote_file(&user_dir), Value::Object(map.clone()).to_string());
    prefetch(app, map.clone());
    Ok(map)
}

pub async fn version_art(app: &AppHandle) -> Result<Value, String> {
    let user_dir = app.state::<AppState>().user_dir.clone();
    if let Some(cached) = read_remote(&user_dir) {
        if !REFRESHING.swap(true, Ordering::SeqCst) {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = refresh(&app2).await;
            });
        }
        prefetch(app, cached.clone());
        return Ok(to_local(&cached));
    }
    let map = refresh(app).await?;
    REFRESHING.store(true, Ordering::SeqCst);
    Ok(to_local(&map))
}

fn reply(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "max-age=604800")
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

pub async fn serve(app: AppHandle, req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let version = req.uri().path().trim_matches('/').to_string();
    if !safe_version(&version) {
        return reply(StatusCode::BAD_REQUEST, "text/plain", Vec::new());
    }
    let user_dir = app.state::<AppState>().user_dir.clone();
    let Some(url) = read_remote(&user_dir).and_then(|m| m.get(&version).and_then(|u| u.as_str().map(String::from))) else {
        return reply(StatusCode::NOT_FOUND, "text/plain", Vec::new());
    };
    match fetch_image(&app.package_info().version.to_string(), &user_dir, &version, &url).await {
        Ok(bytes) => {
            let m = mime(&bytes);
            reply(StatusCode::OK, m, bytes)
        }
        Err(_) => reply(StatusCode::BAD_GATEWAY, "text/plain", Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_map_keeps_only_safe_versions() {
        let mut m = Map::new();
        m.insert("26.3".into(), Value::String("https://x/a.jpg".into()));
        m.insert("../etc".into(), Value::String("https://x/b.jpg".into()));
        let out = to_local(&m);
        assert!(out.get("26.3").and_then(|v| v.as_str()).is_some_and(|u| u.ends_with("/26.3")));
        assert!(out.get("../etc").is_none());
    }

    #[test]
    fn image_files_change_with_url() {
        let d = Path::new("/tmp/x");
        assert_ne!(file_for(d, "26.3", "https://x/a.jpg"), file_for(d, "26.3", "https://x/b.jpg"));
        assert_eq!(mime(&[0x89, b'P', b'N', b'G', 0]), "image/png");
    }
}

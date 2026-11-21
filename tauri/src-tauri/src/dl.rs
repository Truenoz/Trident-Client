// Descargas y consultas HTTP compartidas: cliente único, JSON, archivos con verificación SHA-1 y reintentos.

use futures_util::StreamExt;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

// ───────────────────────── Consultas recientes ─────────────────────────
// Para que el juego abra rápido: lo que se consultó hace poco (Fabric, Fabric API, el mod de Trident…)
// no se vuelve a pedir a internet en cada «Jugar». Se guarda en disco para que valga entre aperturas.
static CHECKS_FILE: OnceLock<PathBuf> = OnceLock::new();
static CHECKS: std::sync::Mutex<Option<serde_json::Map<String, Value>>> = std::sync::Mutex::new(None);

/// Dónde se guardan las consultas (se llama una vez al abrir el launcher).
pub fn set_checks_file(path: PathBuf) {
    let _ = CHECKS_FILE.set(path);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn with_checks<R>(f: impl FnOnce(&mut serde_json::Map<String, Value>) -> R) -> R {
    let mut guard = CHECKS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(|| {
        CHECKS_FILE
            .get()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    });
    f(map)
}

/// ¿Se consultó `key` hace menos de `hours` horas?
pub fn checked_recently(key: &str, hours: u64) -> bool {
    with_checks(|m| m.get(key).and_then(|v| v.as_u64()).is_some_and(|t| now_secs().saturating_sub(t) < hours * 3600))
}

/// Anota que `key` se acaba de consultar.
pub fn mark_checked(key: &str) {
    with_checks(|m| {
        m.insert(key.to_string(), Value::from(now_secs()));
        if let Some(p) = CHECKS_FILE.get() {
            let _ = std::fs::write(p, Value::Object(m.clone()).to_string());
        }
    });
}

pub fn user_agent() -> String {
    format!("TridentClient/{}", env!("CARGO_PKG_VERSION"))
}

/// Criptografía de TLS (rustls con `ring`, la misma que usa el actualizador). Se puede llamar varias veces.
pub fn tls_init() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Cliente HTTP único (reutiliza conexiones). Sin tiempo total máximo porque hay descargas grandes;
/// solo se limita el tiempo para conectar y el silencio entre bloques de datos.
pub fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        tls_init();
        reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("no se pudo crear el cliente HTTP")
    })
}

pub async fn get_json(url: &str) -> Result<Value, String> {
    let res = client().get(url).send().await.map_err(|e| format!("{}: {}", host(url), e))?;
    if !res.status().is_success() {
        return Err(format!("HTTP {} al pedir {}", res.status().as_u16(), host(url)));
    }
    res.json::<Value>().await.map_err(|e| format!("{}: respuesta no válida ({})", host(url), e))
}

fn host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| url.to_string())
}

/// ¿El archivo existe y (si se conoce) tiene el tamaño esperado?
pub fn present(path: &Path, size: Option<u64>) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && size.is_none_or(|s| s == 0 || m.len() == s),
        Err(_) => false,
    }
}

/// Descarga `url` en `dest` (por un archivo `.part` que se renombra al terminar).
/// Verifica SHA-1 si se indica y reintenta hasta 3 veces. `on_bytes` recibe (descargado, total).
pub async fn download<F>(url: &str, dest: &Path, sha1: Option<&str>, mut on_bytes: F) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let mut part: PathBuf = dest.to_path_buf();
    let mut name = part.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    part.set_file_name(name);

    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(600 * attempt)).await;
        }
        match try_download(url, &part, sha1, &mut on_bytes).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(dest).await;
                tokio::fs::rename(&part, dest).await.map_err(|e| e.to_string())?;
                return Ok(());
            }
            Err(e) => last = e,
        }
    }
    let _ = tokio::fs::remove_file(&part).await;
    Err(format!("No se pudo descargar {} ({})", host(url), last))
}

async fn try_download<F>(url: &str, part: &Path, sha1: Option<&str>, on_bytes: &mut F) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let res = client().get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(part).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha1::new();
    let mut got: u64 = 0;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        on_bytes(got, total);
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if let Some(want) = sha1 {
        let have = hex::encode(hasher.finalize());
        if !have.eq_ignore_ascii_case(want) {
            return Err("el archivo llegó dañado (SHA-1 distinto)".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod checks_tests {
    use super::*;

    #[test]
    fn consultas_recientes() {
        assert!(!checked_recently("prueba:nunca", 24));
        mark_checked("prueba:ahora");
        assert!(checked_recently("prueba:ahora", 1));
        assert!(!checked_recently("prueba:ahora", 0));
    }
}

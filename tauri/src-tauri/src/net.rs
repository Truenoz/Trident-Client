// Consultas HTTP simples: lista de versiones de Minecraft y novedades de Mojang.

use serde_json::{json, Value};
use std::time::Duration;

const MANIFEST_URL: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";
const NEWS_BASE: &str = "https://launchercontent.mojang.com";

// Únicas versiones de Minecraft que se pueden elegir (igual que client-mod/versions.txt).
const ALLOWED_VERSIONS: &[&str] = &[
    "1.19.2", "1.19.3", "1.19.4",
    "1.20.1", "1.20.2", "1.20.3", "1.20.4", "1.20.5", "1.20.6",
    "1.21", "1.21.1", "1.21.2", "1.21.3", "1.21.4", "1.21.5", "1.21.6", "1.21.7", "1.21.8", "1.21.10", "1.21.11",
    "26.1", "26.1.1", "26.1.2", "26.2", "26.3",
];

fn client(app_version: &str) -> Result<reqwest::Client, String> {
    crate::dl::tls_init();
    reqwest::Client::builder()
        .user_agent(format!("TridentClient/{}", app_version))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

async fn get_json(app_version: &str, url: &str) -> Result<Value, String> {
    let res = client(app_version)?
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    res.json::<Value>().await.map_err(|e| e.to_string())
}

pub fn is_allowed(id: &str) -> bool {
    ALLOWED_VERSIONS.contains(&id)
}

/// Orden del manifiesto de Mojang (más nuevas primero), solo las versiones permitidas.
pub async fn list_versions(app_version: &str) -> Result<Value, String> {
    let manifest = get_json(app_version, MANIFEST_URL).await?;
    let mut out: Vec<Value> = Vec::new();
    if let Some(list) = manifest["versions"].as_array() {
        for v in list {
            let allowed = v["id"].as_str().is_some_and(|id| ALLOWED_VERSIONS.contains(&id));
            if allowed {
                out.push(json!({ "id": v["id"], "type": v["type"] }));
            }
        }
    }
    Ok(Value::Array(out))
}

/// Últimas notas de versión oficiales (las 3 más recientes de tipo «release»).
pub async fn news(app_version: &str) -> Result<Value, String> {
    let url = format!("{}/v2/javaPatchNotes.json", NEWS_BASE);
    let json = get_json(app_version, &url).await?;
    let base = reqwest::Url::parse(NEWS_BASE).map_err(|e| e.to_string())?;
    let mut out: Vec<Value> = Vec::new();
    if let Some(entries) = json["entries"].as_array() {
        for e in entries.iter().filter(|e| e["type"].as_str() == Some("release")).take(3) {
            let image = e["image"]["url"]
                .as_str()
                .and_then(|u| base.join(u).ok())
                .map(|u| u.to_string());
            out.push(json!({
                "title": e["title"],
                "version": e["version"],
                "date": e["date"],
                "image": image
            }));
        }
    }
    Ok(Value::Array(out))
}

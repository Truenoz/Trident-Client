// Fabric: perfil del cargador (meta.fabricmc.net) y Fabric API (Modrinth). Igual que ensureFabric / ensureFabricApi.

use crate::{dl, launcher::Progress};
use sha2::{Digest, Sha512};
use std::path::Path;

/// Instala (si hace falta) el perfil de Fabric para esa versión y devuelve su id (`fabric-loader-<loader>-<mc>`).
pub async fn ensure_fabric(root: &Path, mc: &str, progress: Progress<'_>) -> Result<String, String> {
    match install_fabric(root, mc, progress).await {
        Ok(id) => Ok(id),
        // Sin internet: usar un Fabric ya instalado, si lo hay.
        Err(e) => installed_fabric(root, mc).ok_or(e),
    }
}

/// Fabric ya instalado para esa versión (el más nuevo), si lo hay.
fn installed_fabric(root: &Path, mc: &str) -> Option<String> {
    let suffix = format!("-{mc}");
    let mut old: Vec<String> = std::fs::read_dir(root.join("versions"))
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("fabric-loader-") && n.ends_with(&suffix))
        .filter(|n| crate::launcher::version_json_path(root, n).exists())
        .collect();
    old.sort();
    old.pop()
}

async fn install_fabric(root: &Path, mc: &str, progress: Progress<'_>) -> Result<String, String> {
    // Consultado hace poco: no se vuelve a preguntar a Fabric en cada «Jugar».
    let key = format!("fabric:{mc}");
    if dl::checked_recently(&key, 24) {
        if let Some(id) = installed_fabric(root, mc) {
            return Ok(id);
        }
    }
    let loaders = dl::get_json(&format!("https://meta.fabricmc.net/v2/versions/loader/{mc}")).await?;
    let list = loaders.as_array().cloned().unwrap_or_default();
    let pick = list
        .iter()
        .find(|l| l["loader"]["stable"].as_bool().unwrap_or(false))
        .or_else(|| list.first())
        .ok_or_else(|| format!("Fabric no soporta Minecraft {mc}"))?;
    let loader = pick["loader"]["version"].as_str().ok_or("Fabric no informó la versión del cargador.")?;
    let id = format!("fabric-loader-{loader}-{mc}");
    let json_path = crate::launcher::version_json_path(root, &id);
    if !json_path.exists() {
        progress("Instalando Fabric…".to_string(), None);
        let profile = dl::get_json(&format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{loader}/profile/json")).await?;
        if let Some(dir) = json_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
        std::fs::write(&json_path, text).map_err(|e| e.to_string())?;
    }
    dl::mark_checked(&key);
    Ok(id)
}

fn existing_api(mods_dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(mods_dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                    n.starts_with("fabric-api-") && n.ends_with(".jar")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Un jar a medio bajar o dañado hace que Fabric diga «falta fabric-api» aunque el archivo esté: se verifica.
fn intact(file: &Path, sha512: Option<&str>) -> bool {
    let ok_zip = std::fs::File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|mut z| z.by_name("fabric.mod.json").is_ok());
    if !ok_zip {
        return false;
    }
    match (sha512, std::fs::read(file)) {
        (Some(want), Ok(bytes)) => hex::encode(Sha512::digest(&bytes)).eq_ignore_ascii_case(want),
        (Some(_), Err(_)) => false,
        (None, _) => true,
    }
}

pub async fn ensure_fabric_api(mc: &str, mods_dir: &Path, progress: Progress<'_>) -> Result<(), String> {
    match install_fabric_api(mc, mods_dir, progress).await {
        Ok(()) => Ok(()),
        Err(e) => {
            if existing_api(mods_dir).is_empty() {
                Err(e)
            } else {
                Ok(()) // sin internet pero ya descargada: seguir
            }
        }
    }
}

async fn install_fabric_api(mc: &str, mods_dir: &Path, progress: Progress<'_>) -> Result<(), String> {
    let key = format!("fabric-api:{}:{mc}", mods_dir.display());
    if dl::checked_recently(&key, 24) && !existing_api(mods_dir).is_empty() {
        return Ok(());
    }
    let url = reqwest::Url::parse_with_params(
        "https://api.modrinth.com/v2/project/fabric-api/version",
        &[("game_versions", serde_json::to_string(&[mc]).unwrap_or_default()), ("loaders", "[\"fabric\"]".to_string())],
    )
    .map_err(|e| e.to_string())?;
    let versions = dl::get_json(url.as_str()).await?;
    let list = versions.as_array().cloned().unwrap_or_default();
    let v = list
        .iter()
        .find(|x| x["version_type"].as_str() == Some("release"))
        .or_else(|| list.first())
        .ok_or("Fabric API no disponible para esta versión")?;
    let files = v["files"].as_array().cloned().unwrap_or_default();
    let file = files
        .iter()
        .find(|f| f["primary"].as_bool().unwrap_or(false))
        .or_else(|| files.first())
        .ok_or("Fabric API no disponible para esta versión")?;
    let filename = file["filename"].as_str().ok_or("Modrinth no informó el nombre del archivo.")?;
    let furl = file["url"].as_str().ok_or("Modrinth no informó la URL del archivo.")?;
    let sha512 = file["hashes"]["sha512"].as_str();
    // Solo el nombre: nunca una ruta.
    let dest = mods_dir.join(Path::new(filename).file_name().ok_or("Nombre de archivo no válido.")?);

    if !dest.exists() || !intact(&dest, sha512) {
        for old in existing_api(mods_dir) {
            let _ = std::fs::remove_file(old);
        }
        progress("Descargando Fabric API…".to_string(), Some(0.0));
        dl::download(furl, &dest, None, |got, total| {
            if total > 0 {
                progress("Descargando Fabric API…".to_string(), Some(got as f64 / total as f64 * 100.0));
            }
        })
        .await?;
        if !intact(&dest, sha512) {
            let _ = std::fs::remove_file(&dest);
            return Err("Fabric API se descargó dañada; reintentá".to_string());
        }
    }
    dl::mark_checked(&key);
    Ok(())
}


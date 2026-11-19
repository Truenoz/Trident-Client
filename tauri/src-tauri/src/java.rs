// Java automático (igual que ensureJava de la versión Electron): usa la ruta de Ajustes si existe;
// si no, baja un JRE de Eclipse Temurin a `.novalauncher/runtime/jre-<N>`.

use crate::{dl, launcher::Progress};
use std::path::{Path, PathBuf};

/// Java que se descarga: 8, 17, 21, o la versión pedida si es mayor (p. ej. 25 para Minecraft 26.x).
pub fn adopt_major(major: u32) -> u32 {
    match major {
        0..=8 => 8,
        9..=17 => 17,
        18..=21 => 21,
        m => m,
    }
}

const EXE: &str = if cfg!(windows) { "javaw.exe" } else { "java" };

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let ft = entry.file_type().ok()?;
        if ft.is_file() && entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(path);
        }
        if ft.is_dir() {
            if let Some(f) = find_file(&path, name) {
                return Some(f);
            }
        }
    }
    None
}

pub async fn ensure_java(game_dir: &Path, custom: &str, major: u32, progress: Progress<'_>) -> Result<PathBuf, String> {
    if !custom.trim().is_empty() && Path::new(custom.trim()).exists() {
        return Ok(PathBuf::from(custom.trim()));
    }
    // Fuera de Windows no se baja nada: se usa el Java del sistema (solo para pruebas).
    if !cfg!(windows) {
        return Ok(PathBuf::from("java"));
    }

    let n = adopt_major(major);
    let runtime = game_dir.join("runtime").join(format!("jre-{n}"));
    if let Some(found) = find_file(&runtime, EXE) {
        return Ok(found);
    }

    let label = format!("Descargando Java {n}…");
    progress(label.clone(), Some(0.0));
    tokio::fs::create_dir_all(&runtime).await.map_err(|e| e.to_string())?;
    let zip_path = runtime.join("jre.zip");
    let url = format!("https://api.adoptium.net/v3/binary/latest/{n}/ga/windows/x64/jre/hotspot/normal/eclipse");
    dl::download(&url, &zip_path, None, |got, total| {
        if total > 0 {
            progress(label.clone(), Some(got as f64 / total as f64 * 100.0));
        }
    })
    .await?;

    progress(format!("Instalando Java {n}…"), None);
    let (zp, rt) = (zip_path.clone(), runtime.clone());
    tokio::task::spawn_blocking(move || unzip(&zp, &rt)).await.map_err(|e| e.to_string())??;
    let _ = tokio::fs::remove_file(&zip_path).await;

    find_file(&runtime, EXE).ok_or_else(|| format!("No se encontró {EXE} tras instalar Java."))
}

/// Extrae un zip sin permitir rutas que se salgan de la carpeta de destino.
pub fn unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("El archivo de Java está dañado: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let rel = match entry.enclosed_name() {
            Some(p) => p,
            None => continue,
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut f = std::fs::File::create(&out).map_err(|e| format!("{}: {}", out.display(), e))?;
            std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_de_java() {
        assert_eq!(adopt_major(8), 8);
        assert_eq!(adopt_major(16), 17);
        assert_eq!(adopt_major(17), 17);
        assert_eq!(adopt_major(21), 21);
        assert_eq!(adopt_major(25), 25);
    }
}

use base64::Engine;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const UWP_PACKAGE: &str = "Microsoft.MinecraftUWP_8wekyb3d8bbwe";
const MAX_ICON: u64 = 400 * 1024;
pub const STORE_URL: &str = "ms-windows-store://pdp/?productid=9NBLGGH2JHXJ";

#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub dir: PathBuf,
    pub source: &'static str,
    pub user: String,
}

pub fn roots(roaming: &Path, local: &Path) -> Vec<Root> {
    let mut out = Vec::new();
    let users = roaming.join("Minecraft Bedrock").join("Users");
    if let Ok(rd) = std::fs::read_dir(&users) {
        let mut entries: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let dir = e.path().join("games").join("com.mojang").join("minecraftWorlds");
            if dir.is_dir() {
                out.push(Root { dir, source: "gdk", user: e.file_name().to_string_lossy().to_string() });
            }
        }
    }
    let uwp = local.join("Packages").join(UWP_PACKAGE).join("LocalState").join("games").join("com.mojang").join("minecraftWorlds");
    if uwp.is_dir() {
        out.push(Root { dir: uwp, source: "uwp", user: String::new() });
    }
    out
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => dir_size(&e.path()),
                    Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn world_name(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("levelname.txt"))
        .ok()
        .map(|t| t.trim_start_matches('\u{feff}').trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

fn world_icon(dir: &Path) -> Option<String> {
    let p = dir.join("world_icon.jpeg");
    let len = std::fs::metadata(&p).ok()?.len();
    if len == 0 || len > MAX_ICON {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub fn worlds(roots: &[Root]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for r in roots {
        let Ok(rd) = std::fs::read_dir(&r.dir) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.join("level.dat").is_file() {
                continue;
            }
            let modified = mtime_ms(&dir.join("level.dat")).max(mtime_ms(&dir));
            out.push(json!({
                "path": dir.to_string_lossy(),
                "name": world_name(&dir),
                "icon": world_icon(&dir),
                "size": dir_size(&dir),
                "modified": modified,
                "source": r.source,
                "user": r.user,
            }));
        }
    }
    out.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    out
}

pub fn world_dir(roots: &[Root], path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let canon = p.canonicalize().map_err(|_| "Ese mundo ya no existe.".to_string())?;
    let inside = roots.iter().any(|r| r.dir.canonicalize().is_ok_and(|d| canon.parent() == Some(d.as_path())));
    if !inside || !canon.join("level.dat").is_file() {
        return Err("Ese mundo no es de Minecraft Bedrock.".to_string());
    }
    Ok(canon)
}

pub fn display_name(dir: &Path) -> String {
    world_name(dir)
}

pub fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_().".contains(c) { c } else { '_' })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "mundo".to_string() } else { s.chars().take(80).collect() }
}

fn add_dir(z: &mut zip::ZipWriter<std::fs::File>, base: &Path, dir: &Path, opt: zip::write::SimpleFileOptions) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            z.add_directory(format!("{rel}/"), opt).map_err(|e| e.to_string())?;
            add_dir(z, base, &path, opt)?;
        } else {
            z.start_file(rel, opt).map_err(|e| e.to_string())?;
            let mut f = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            z.write_all(&buf).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn backup(world: &Path, dest: &Path) -> Result<(), String> {
    let tmp = dest.with_extension("mcworld.part");
    let file = std::fs::File::create(&tmp).map_err(|e| format!("No se pudo crear la copia: {e}"))?;
    let mut z = zip::ZipWriter::new(file);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let res = add_dir(&mut z, world, world, opt).and_then(|_| z.finish().map(|_| ()).map_err(|e| e.to_string()));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("No se pudo hacer la copia: {e}"));
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("No se pudo guardar la copia: {e}"))
}

fn new_folder_name(root: &Path) -> PathBuf {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    loop {
        let name: String = (0..12).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect();
        let p = root.join(name);
        if !p.exists() {
            return p;
        }
    }
}

pub fn import_target(roots: &[Root]) -> Option<&Root> {
    roots
        .iter()
        .find(|r| r.source == "gdk" && !r.user.eq_ignore_ascii_case("shared"))
        .or_else(|| roots.iter().find(|r| r.source == "gdk"))
        .or_else(|| roots.first())
}

pub fn import(file: &Path, root: &Path) -> Result<String, String> {
    let f = std::fs::File::open(file).map_err(|e| format!("No se pudo abrir el archivo: {e}"))?;
    let mut zip = zip::ZipArchive::new(f).map_err(|_| "El archivo no es un mundo de Bedrock (.mcworld).".to_string())?;
    let prefix = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().and_then(|e| e.enclosed_name()))
        .filter(|p| p.file_name().is_some_and(|n| n == "level.dat"))
        .min_by_key(|p| p.components().count())
        .map(|p| p.parent().map(|x| x.to_path_buf()).unwrap_or_default())
        .ok_or("El archivo no tiene un mundo de Bedrock adentro (falta level.dat).")?;
    let dest = new_folder_name(root);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let res = (|| -> Result<(), String> {
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let Ok(rel) = rel.strip_prefix(&prefix) else { continue };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let out = dest.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut w).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(format!("No se pudo importar el mundo: {e}"));
    }
    Ok(world_name(&dest))
}

#[cfg(windows)]
pub fn protocol_registered() -> bool {
    use winreg::enums::HKEY_CLASSES_ROOT;
    winreg::RegKey::predef(HKEY_CLASSES_ROOT).open_subkey("minecraft").is_ok()
}

#[cfg(not(windows))]
pub fn protocol_registered() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trident-bedrock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_world(root: &Path, folder: &str, name: &str) -> PathBuf {
        let w = root.join(folder);
        std::fs::create_dir_all(w.join("db")).unwrap();
        std::fs::write(w.join("level.dat"), b"dat").unwrap();
        std::fs::write(w.join("levelname.txt"), name).unwrap();
        std::fs::write(w.join("db").join("000001.ldb"), vec![7u8; 2048]).unwrap();
        w
    }

    #[test]
    fn encuentra_mundos_gdk_y_uwp() {
        let base = tmp("roots");
        let (roaming, local) = (base.join("Roaming"), base.join("Local"));
        let gdk = roaming.join("Minecraft Bedrock/Users/123456/games/com.mojang/minecraftWorlds");
        let uwp = local.join("Packages").join(UWP_PACKAGE).join("LocalState/games/com.mojang/minecraftWorlds");
        std::fs::create_dir_all(&gdk).unwrap();
        std::fs::create_dir_all(&uwp).unwrap();
        make_world(&gdk, "aaa", "Survival");
        make_world(&uwp, "bbb", "Viejo");
        std::fs::create_dir_all(gdk.join("roto")).unwrap();

        let r = roots(&roaming, &local);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].source, "gdk");
        assert_eq!(r[0].user, "123456");
        let w = worlds(&r);
        assert_eq!(w.len(), 2);
        let names: Vec<&str> = w.iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Survival") && names.contains(&"Viejo"));
        assert!(w[0]["size"].as_u64().unwrap() >= 2048);

        assert!(world_dir(&r, &gdk.join("aaa").to_string_lossy()).is_ok());
        assert!(world_dir(&r, &base.to_string_lossy()).is_err());
        assert!(world_dir(&r, &gdk.join("roto").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn copia_e_importa_un_mundo() {
        let base = tmp("io");
        let src = base.join("worlds");
        let w = make_world(&src, "xyz", "Mi Mundo");
        let file = base.join("Mi Mundo.mcworld");
        backup(&w, &file).unwrap();
        assert!(file.is_file());

        let dest = base.join("destino");
        std::fs::create_dir_all(&dest).unwrap();
        let name = import(&file, &dest).unwrap();
        assert_eq!(name, "Mi Mundo");
        let imported: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().collect();
        assert_eq!(imported.len(), 1);
        let d = imported[0].path();
        assert!(d.join("level.dat").is_file());
        assert_eq!(std::fs::read(d.join("db/000001.ldb")).unwrap().len(), 2048);

        let nested = base.join("anidado.mcworld");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&nested).unwrap());
            let opt = zip::write::SimpleFileOptions::default();
            z.start_file("Carpeta/level.dat", opt).unwrap();
            z.write_all(b"dat").unwrap();
            z.start_file("Carpeta/levelname.txt", opt).unwrap();
            z.write_all(b"Anidado").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(import(&nested, &dest).unwrap(), "Anidado");

        let bad = base.join("malo.mcworld");
        std::fs::write(&bad, b"no es zip").unwrap();
        assert!(import(&bad, &dest).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nombre_de_archivo_seguro() {
        assert_eq!(safe_file_name("Mi/Mundo:1"), "Mi_Mundo_1");
        assert_eq!(safe_file_name("   "), "mundo");
    }

    #[test]
    fn destino_de_importacion() {
        let r = vec![
            Root { dir: PathBuf::from("a"), source: "gdk", user: "Shared".into() },
            Root { dir: PathBuf::from("b"), source: "gdk", user: "123".into() },
            Root { dir: PathBuf::from("c"), source: "uwp", user: String::new() },
        ];
        assert_eq!(import_target(&r).unwrap().dir, PathBuf::from("b"));
        assert!(import_target(&[]).is_none());
    }
}

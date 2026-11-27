use crate::dl;
use futures_util::StreamExt;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const MANIFEST_URL: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";
const LIBRARIES_URL: &str = "https://libraries.minecraft.net/";
const ASSETS_URL: &str = "https://resources.download.minecraft.net/";

pub type Progress<'a> = &'a (dyn Fn(String, Option<f64>) + Send + Sync);

pub struct Env<'a> {
    pub os: &'a str,
    pub arch: &'a str,
    pub os_version: &'a str,
}

#[cfg(test)]
pub const WINDOWS: Env<'static> = Env { os: "windows", arch: "x64", os_version: "10.0" };

pub fn current() -> Env<'static> {
    let os = if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "windows"
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x64",
    };
    let os_version = match os {
        "osx" => "14.0",
        "linux" => "6.0",
        _ => "10.0",
    };
    Env { os, arch, os_version }
}

fn rule_matches(rule: &Value, env: &Env, features: &HashSet<String>) -> bool {
    if let Some(os) = rule.get("os") {
        if let Some(n) = os["name"].as_str() {
            if n != env.os {
                return false;
            }
        }
        if let Some(a) = os["arch"].as_str() {
            if a != env.arch {
                return false;
            }
        }
        if let Some(v) = os["version"].as_str() {
            match Regex::new(v) {
                Ok(re) if re.is_match(env.os_version) => {}
                _ => return false,
            }
        }
    }
    if let Some(f) = rule.get("features").and_then(|f| f.as_object()) {
        for (k, want) in f {
            if features.contains(k) != want.as_bool().unwrap_or(false) {
                return false;
            }
        }
    }
    true
}

pub fn rules_allow(rules: Option<&Value>, env: &Env, features: &HashSet<String>) -> bool {
    let list = match rules.and_then(|r| r.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return true,
    };
    let mut allowed = false;
    for r in list {
        if rule_matches(r, env, features) {
            allowed = r["action"].as_str() == Some("allow");
        }
    }
    allowed
}

pub fn version_json_path(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.json"))
}

pub fn version_jar_path(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.jar"))
}

pub fn merge_versions(parent: Value, child: Value) -> Value {
    let mut out = match parent {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let child = match child {
        Value::Object(m) => m,
        _ => return Value::Object(out),
    };
    for (k, v) in child {
        match k.as_str() {
            "inheritsFrom" => {}
            "libraries" => {
                let mut libs: Vec<Value> = v.as_array().cloned().unwrap_or_default();
                libs.extend(out.get("libraries").and_then(|l| l.as_array()).cloned().unwrap_or_default());
                out.insert(k, Value::Array(libs));
            }
            "arguments" => {
                let mut args = out.get("arguments").and_then(|a| a.as_object()).cloned().unwrap_or_default();
                for kind in ["game", "jvm"] {
                    let mut list: Vec<Value> = args.get(kind).and_then(|a| a.as_array()).cloned().unwrap_or_default();
                    list.extend(v.get(kind).and_then(|a| a.as_array()).cloned().unwrap_or_default());
                    args.insert(kind.to_string(), Value::Array(list));
                }
                out.insert(k, Value::Object(args));
            }
            _ => {
                out.insert(k, v);
            }
        }
    }
    Value::Object(out)
}

pub fn load_version(root: &Path, id: &str) -> Result<Value, String> {
    fn go(root: &Path, id: &str, depth: u8) -> Result<Value, String> {
        if depth > 5 {
            return Err(format!("La versión {id} hereda de demasiadas versiones."));
        }
        let path = version_json_path(root, id);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("No se pudo leer {}: {}", path.display(), e))?;
        let json: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("El JSON de la versión {id} está dañado: {e}"))?;
        match json["inheritsFrom"].as_str() {
            Some(parent) => {
                let mut merged = merge_versions(go(root, parent, depth + 1)?, json.clone());
                if merged.get("jarId").is_none() {
                    merged["jarId"] = Value::String(parent.to_string());
                }
                Ok(merged)
            }
            None => {
                let mut j = json;

                let jar = j["jar"].as_str().unwrap_or(id).to_string();
                j["jarId"] = Value::String(jar);
                Ok(j)
            }
        }
    }
    go(root, id, 0)
}

fn local_version_json(root: &Path, id: &str) -> Option<Value> {
    let t = std::fs::read_to_string(version_json_path(root, id)).ok()?;
    serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok().filter(|v| v["id"].is_string())
}

pub async fn ensure_vanilla_json(root: &Path, id: &str) -> Result<String, String> {
    if let Some(v) = local_version_json(root, id) {
        return Ok(v["type"].as_str().unwrap_or("release").to_string());
    }
    let manifest = dl::get_json(MANIFEST_URL).await?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|l| l.iter().find(|v| v["id"].as_str() == Some(id)))
        .ok_or_else(|| format!("Versión desconocida: {id}"))?;
    let kind = entry["type"].as_str().unwrap_or("release").to_string();
    let path = version_json_path(root, id);
    let ok = std::fs::read_to_string(&path)
        .ok()
        .is_some_and(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).is_ok());
    if !ok {
        let url = entry["url"].as_str().ok_or("El manifiesto no trae la URL de la versión.")?;
        let sha1 = entry["sha1"].as_str();
        dl::download(url, &path, sha1, |_, _| {}).await?;
    }
    Ok(kind)
}

pub async fn required_java_major(root: &Path, id: &str) -> Result<u32, String> {
    if let Some(v) = local_version_json(root, id) {
        return Ok(v["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32);
    }
    let manifest = dl::get_json(MANIFEST_URL).await?;
    let url = manifest["versions"]
        .as_array()
        .and_then(|l| l.iter().find(|v| v["id"].as_str() == Some(id)))
        .and_then(|v| v["url"].as_str())
        .ok_or_else(|| format!("Versión desconocida: {id}"))?
        .to_string();
    let meta = dl::get_json(&url).await?;
    Ok(meta["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lib {

    pub rel: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,

    pub extract: Option<Vec<String>>,
}

pub fn maven_path(name: &str, classifier: Option<&str>) -> Option<String> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let mut parts = coords.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let own_class = parts.next();
    let class = classifier.or(own_class);
    let file = match class {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{}/{}/{}/{}", group.replace('.', "/"), artifact, version, file))
}

fn dedupe_key(name: &str) -> String {
    let coords = name.split('@').next().unwrap_or(name);
    let p: Vec<&str> = coords.split(':').collect();
    match p.len() {
        0..=2 => coords.to_string(),
        3 => format!("{}:{}", p[0], p[1]),
        _ => format!("{}:{}:{}", p[0], p[1], p[3]),
    }
}

pub fn collect_libs(v: &Value, env: &Env, features: &HashSet<String>) -> Vec<Lib> {
    let mut out: Vec<Lib> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for lib in v["libraries"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        if !rules_allow(lib.get("rules"), env, features) {
            continue;
        }
        let name = match lib["name"].as_str() {
            Some(n) => n,
            None => continue,
        };

        let native_class = lib["natives"][env.os]
            .as_str()
            .map(|c| c.replace("${arch}", if env.arch == "x86" { "32" } else { "64" }));
        let (artifact, classifier, extract) = match &native_class {
            Some(c) => {
                let excl: Vec<String> = lib["extract"]["exclude"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                (lib["downloads"]["classifiers"].get(c.as_str()), Some(c.as_str()), Some(excl))
            }
            None => (lib["downloads"].get("artifact"), None, None),
        };

        let key = match classifier {
            Some(c) => format!("{}:{}", dedupe_key(name), c),
            None => dedupe_key(name),
        };
        if !seen.insert(key) {
            continue;
        }

        let built = match artifact {
            Some(a) if a["path"].is_string() => Lib {
                rel: a["path"].as_str().unwrap_or("").to_string(),
                url: a["url"].as_str().unwrap_or("").to_string(),
                sha1: a["sha1"].as_str().map(|s| s.to_string()),
                size: a["size"].as_u64(),
                extract,
            },
            _ => {

                let rel = match maven_path(name, classifier) {
                    Some(r) => r,
                    None => continue,
                };
                let mut base = lib["url"].as_str().unwrap_or(LIBRARIES_URL).to_string();
                if !base.ends_with('/') {
                    base.push('/');
                }
                Lib {
                    url: format!("{base}{rel}"),
                    rel,
                    sha1: lib["sha1"].as_str().map(|s| s.to_string()),
                    size: lib["size"].as_u64(),
                    extract,
                }
            }
        };
        out.push(built);
    }
    out
}

struct Item {
    url: String,
    dest: PathBuf,
    sha1: Option<String>,
}

async fn download_many(items: Vec<Item>, label: &str, concurrency: usize, progress: Progress<'_>) -> Result<(), String> {
    let total = items.len();
    if total == 0 {
        return Ok(());
    }
    let done = Arc::new(AtomicUsize::new(0));
    progress(format!("Descargando {label}…"), Some(0.0));
    let mut stream = futures_util::stream::iter(items.into_iter().map(|it| async move {
        if it.url.is_empty() {
            return Err(format!("Falta {} y no hay de dónde bajarlo.", it.dest.display()));
        }
        dl::download(&it.url, &it.dest, it.sha1.as_deref(), |_, _| {}).await
    }))
    .buffer_unordered(concurrency);
    while let Some(r) = stream.next().await {
        r?;
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        progress(format!("Descargando {label}…"), Some(n as f64 / total as f64 * 100.0));
    }
    Ok(())
}

pub struct Prepared {

    pub version: Value,
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub asset_index: String,
    pub logging_arg: Option<String>,
}

pub async fn prepare(root: &Path, launch_id: &str, env: &Env<'_>, progress: Progress<'_>) -> Result<Prepared, String> {
    let version = load_version(root, launch_id)?;
    let features: HashSet<String> = HashSet::new();
    let jar_id = version["jarId"].as_str().unwrap_or(launch_id).to_string();

    let jar = version_jar_path(root, &jar_id);
    let client = &version["downloads"]["client"];
    if !dl::present(&jar, client["size"].as_u64()) {
        let url = client["url"].as_str().ok_or("La versión no trae el cliente de Minecraft.")?;
        progress("Descargando Minecraft…".to_string(), Some(0.0));
        dl::download(url, &jar, client["sha1"].as_str(), |got, total| {
            if total > 0 {
                progress("Descargando Minecraft…".to_string(), Some(got as f64 / total as f64 * 100.0));
            }
        })
        .await?;
    }

    let libs = collect_libs(&version, env, &features);
    let lib_dir = root.join("libraries");
    let missing: Vec<Item> = libs
        .iter()
        .filter(|l| !dl::present(&lib_dir.join(&l.rel), l.size))
        .map(|l| Item { url: l.url.clone(), dest: lib_dir.join(&l.rel), sha1: l.sha1.clone() })
        .collect();
    download_many(missing, "librerías", 12, progress).await?;

    let natives_dir = root.join("versions").join(launch_id).join("natives");
    std::fs::create_dir_all(&natives_dir).map_err(|e| e.to_string())?;
    for lib in libs.iter().filter(|l| l.extract.is_some()) {
        extract_natives(&lib_dir.join(&lib.rel), &natives_dir, lib.extract.as_deref().unwrap_or(&[]))?;
    }

    let index_id = version["assetIndex"]["id"]
        .as_str()
        .or_else(|| version["assets"].as_str())
        .ok_or("La versión no indica su índice de recursos.")?
        .to_string();
    let index_path = root.join("assets").join("indexes").join(format!("{index_id}.json"));
    let index_ok = std::fs::read_to_string(&index_path)
        .ok()
        .is_some_and(|t| serde_json::from_str::<Value>(&t).is_ok());
    if !index_ok {
        let url = version["assetIndex"]["url"].as_str().ok_or("Falta la URL del índice de recursos.")?;
        dl::download(url, &index_path, version["assetIndex"]["sha1"].as_str(), |_, _| {}).await?;
    }
    let index: Value = serde_json::from_str(&std::fs::read_to_string(&index_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("El índice de recursos está dañado: {e}"))?;
    let objects_dir = root.join("assets").join("objects");
    let mut wanted: Vec<Item> = Vec::new();
    if let Some(objs) = index["objects"].as_object() {
        for o in objs.values() {
            let hash = match o["hash"].as_str() {
                Some(h) if h.len() >= 2 => h,
                _ => continue,
            };
            let dest = objects_dir.join(&hash[..2]).join(hash);
            if !dl::present(&dest, o["size"].as_u64()) {
                wanted.push(Item { url: format!("{ASSETS_URL}{}/{}", &hash[..2], hash), dest, sha1: Some(hash.to_string()) });
            }
        }
    }
    download_many(wanted, "recursos", 32, progress).await?;

    let mut logging_arg = None;
    let log_client = &version["logging"]["client"];
    if let (Some(file_id), Some(url), Some(arg)) =
        (log_client["file"]["id"].as_str(), log_client["file"]["url"].as_str(), log_client["argument"].as_str())
    {
        let dest = root.join("assets").join("log_configs").join(file_id);
        if !dl::present(&dest, log_client["file"]["size"].as_u64()) {
            dl::download(url, &dest, log_client["file"]["sha1"].as_str(), |_, _| {}).await?;
        }
        logging_arg = Some(arg.replace("${path}", &dest.to_string_lossy()));
    }

    let mut classpath: Vec<PathBuf> = libs.iter().filter(|l| l.extract.is_none()).map(|l| lib_dir.join(&l.rel)).collect();
    classpath.push(jar);

    Ok(Prepared { version, classpath, natives_dir, asset_index: index_id, logging_arg })
}

fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<(), String> {
    let file = std::fs::File::open(jar).map_err(|e| format!("{}: {}", jar.display(), e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {}", jar.display(), e))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if name.starts_with("META-INF/") || exclude.iter().any(|x| name.starts_with(x.as_str())) {
            continue;
        }

        let leaf = match Path::new(&name).file_name() {
            Some(l) => l.to_owned(),
            None => continue,
        };
        let target = dest.join(leaf);
        let mut out = std::fs::File::create(&target).map_err(|e| format!("{}: {}", target.display(), e))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn substitute(s: &str, vars: &HashMap<String, String>) -> String {
    let re = Regex::new(r"\$\{([^}]+)\}").expect("regex fija");
    re.replace_all(s, |c: &regex::Captures| vars.get(&c[1]).cloned().unwrap_or_else(|| c[0].to_string()))
        .into_owned()
}

fn expand_args(list: &[Value], env: &Env, features: &HashSet<String>, vars: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for a in list {
        match a {
            Value::String(s) => out.push(substitute(s, vars)),
            Value::Object(_) => {
                if !rules_allow(a.get("rules"), env, features) {
                    continue;
                }
                match &a["value"] {
                    Value::String(s) => out.push(substitute(s, vars)),
                    Value::Array(vs) => out.extend(vs.iter().filter_map(|v| v.as_str()).map(|s| substitute(s, vars))),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

pub fn build_args(v: &Value, env: &Env, features: &HashSet<String>, vars: &HashMap<String, String>) -> (Vec<String>, Vec<String>) {
    if let Some(args) = v.get("arguments").filter(|a| a.is_object()) {
        let jvm = expand_args(args["jvm"].as_array().map(|a| a.as_slice()).unwrap_or(&[]), env, features, vars);
        let game = expand_args(args["game"].as_array().map(|a| a.as_slice()).unwrap_or(&[]), env, features, vars);
        return (jvm, game);
    }
    let jvm = ["-Djava.library.path=${natives_directory}", "-Dminecraft.launcher.brand=${launcher_name}",
        "-Dminecraft.launcher.version=${launcher_version}", "-cp", "${classpath}"]
        .iter()
        .map(|s| substitute(s, vars))
        .collect();
    let game = v["minecraftArguments"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .map(|s| substitute(s, vars))
        .collect();
    (jvm, game)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feats(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reglas_de_sistema_y_funciones() {
        let w = &WINDOWS;
        let none = feats(&[]);
        assert!(rules_allow(None, w, &none));
        let solo_osx = json!([{ "action": "allow", "os": { "name": "osx" } }]);
        assert!(!rules_allow(Some(&solo_osx), w, &none));
        let todo_menos_osx = json!([{ "action": "allow" }, { "action": "disallow", "os": { "name": "osx" } }]);
        assert!(rules_allow(Some(&todo_menos_osx), w, &none));
        let x86 = json!([{ "action": "allow", "os": { "name": "windows", "arch": "x86" } }]);
        assert!(!rules_allow(Some(&x86), w, &none));
        let win10 = json!([{ "action": "allow", "os": { "name": "windows", "version": "^10\\." } }]);
        assert!(rules_allow(Some(&win10), w, &none));
        let demo = json!([{ "action": "allow", "features": { "is_demo_user": true } }]);
        assert!(!rules_allow(Some(&demo), w, &none));
        assert!(rules_allow(Some(&demo), w, &feats(&["is_demo_user"])));
    }

    #[test]
    fn rutas_maven() {
        assert_eq!(maven_path("net.fabricmc:fabric-loader:0.16.9", None).unwrap(),
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
        assert_eq!(maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows", None).unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar");
        assert_eq!(maven_path("a.b:c:1.0", Some("natives-windows")).unwrap(), "a/b/c/1.0/c-1.0-natives-windows.jar");
        assert_eq!(maven_path("a.b:c:1.0@zip", None).unwrap(), "a/b/c/1.0/c-1.0.zip");
    }

    #[test]
    fn librerias_modernas_y_fabric() {
        let v = json!({ "libraries": [
            { "name": "net.fabricmc:fabric-loader:0.16.9", "url": "https://maven.fabricmc.net/", "sha1": "abc" },
            { "name": "org.ow2.asm:asm:9.7", "url": "https://maven.fabricmc.net/" },
            { "name": "org.ow2.asm:asm:9.6", "downloads": { "artifact": { "path": "org/ow2/asm/asm/9.6/asm-9.6.jar", "url": "https://libraries.minecraft.net/org/ow2/asm/asm/9.6/asm-9.6.jar", "sha1": "x", "size": 5 } } },
            { "name": "org.lwjgl:lwjgl:3.3.3", "downloads": { "artifact": { "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar", "url": "u1", "sha1": "s1", "size": 1 } } },
            { "name": "org.lwjgl:lwjgl:3.3.3:natives-windows", "rules": [{ "action": "allow", "os": { "name": "windows" } }],
              "downloads": { "artifact": { "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar", "url": "u2", "sha1": "s2", "size": 2 } } },
            { "name": "org.lwjgl:lwjgl:3.3.3:natives-linux", "rules": [{ "action": "allow", "os": { "name": "linux" } }],
              "downloads": { "artifact": { "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar", "url": "u3", "sha1": "s3", "size": 3 } } }
        ]});
        let libs = collect_libs(&v, &WINDOWS, &feats(&[]));
        let rels: Vec<&str> = libs.iter().map(|l| l.rel.as_str()).collect();
        assert_eq!(rels, vec![
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar",
            "org/ow2/asm/asm/9.7/asm-9.7.jar",
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar",
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar",
        ]);
        assert_eq!(libs[0].url, "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
        assert_eq!(libs[0].sha1.as_deref(), Some("abc"));
    }

    #[test]
    fn nativos_antiguos() {
        let v = json!({ "libraries": [{
            "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4",
            "natives": { "windows": "natives-windows", "linux": "natives-linux" },
            "extract": { "exclude": ["META-INF/"] },
            "downloads": { "classifiers": { "natives-windows": { "path": "p/natives-windows.jar", "url": "u", "sha1": "s", "size": 9 } } }
        }]});
        let libs = collect_libs(&v, &WINDOWS, &feats(&[]));
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].rel, "p/natives-windows.jar");
        assert_eq!(libs[0].extract.as_deref(), Some(&["META-INF/".to_string()][..]));
    }

    #[test]
    fn herencia_de_fabric() {
        let vanilla = json!({
            "id": "1.21.4", "mainClass": "net.minecraft.client.main.Main", "assets": "19",
            "libraries": [{ "name": "a:vanilla:1" }],
            "arguments": { "game": ["--username", "${auth_player_name}"], "jvm": ["-Xss1M"] }
        });
        let fabric = json!({
            "id": "fabric-loader-0.16.9-1.21.4", "inheritsFrom": "1.21.4",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "libraries": [{ "name": "a:fabric:1" }],
            "arguments": { "game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "] }
        });
        let m = merge_versions(vanilla, fabric);
        assert_eq!(m["mainClass"], "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(m["id"], "fabric-loader-0.16.9-1.21.4");
        assert_eq!(m["assets"], "19");
        assert_eq!(m["libraries"][0]["name"], "a:fabric:1");
        assert_eq!(m["libraries"][1]["name"], "a:vanilla:1");
        assert_eq!(m["arguments"]["jvm"], json!(["-Xss1M", "-DFabricMcEmu= net.minecraft.client.main.Main "]));
        assert!(m.get("inheritsFrom").is_none());
    }

    #[test]
    fn argumentos_con_reglas_y_variables() {
        let v = json!({ "arguments": {
            "jvm": [
                { "rules": [{ "action": "allow", "os": { "name": "osx" } }], "value": ["-XstartOnFirstThread"] },
                { "rules": [{ "action": "allow", "os": { "name": "windows" } }], "value": "-XX:HeapDumpPath=x" },
                "-Djava.library.path=${natives_directory}", "-cp", "${classpath}"
            ],
            "game": [
                "--username", "${auth_player_name}",
                { "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }], "value": ["--width", "${resolution_width}"] },
                { "rules": [{ "action": "allow", "features": { "is_quick_play_multiplayer": true } }], "value": ["--quickPlayMultiplayer", "${quickPlayMultiplayer}"] }
            ]
        }});
        let mut vars = HashMap::new();
        vars.insert("natives_directory".to_string(), "C:\\n".to_string());
        vars.insert("classpath".to_string(), "a.jar;b.jar".to_string());
        vars.insert("auth_player_name".to_string(), "Steve".to_string());
        vars.insert("resolution_width".to_string(), "1280".to_string());
        let (jvm, game) = build_args(&v, &WINDOWS, &feats(&["has_custom_resolution"]), &vars);
        assert_eq!(jvm, vec!["-XX:HeapDumpPath=x", "-Djava.library.path=C:\\n", "-cp", "a.jar;b.jar"]);
        assert_eq!(game, vec!["--username", "Steve", "--width", "1280"]);
    }

    #[test]
    fn argumentos_formato_antiguo() {
        let v = json!({ "minecraftArguments": "--username ${auth_player_name} --version ${version_name}" });
        let mut vars = HashMap::new();
        vars.insert("auth_player_name".to_string(), "Alex".to_string());
        vars.insert("version_name".to_string(), "1.12.2".to_string());
        vars.insert("natives_directory".to_string(), "n".to_string());
        vars.insert("classpath".to_string(), "c".to_string());
        let (jvm, game) = build_args(&v, &WINDOWS, &feats(&[]), &vars);
        assert_eq!(game, vec!["--username", "Alex", "--version", "1.12.2"]);
        assert!(jvm.contains(&"-cp".to_string()));
    }
}

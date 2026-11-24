use crate::{dl, modcompat, profiles};
use base64::Engine;
use regex::Regex;
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

const MODRINTH: &str = "https://api.modrinth.com/v2";

pub const PERF_MODS: &[(&str, &str, &str)] = &[
    ("sodium", "sodium", "Sodium"),
    ("iris", "iris", "Iris Shaders"),
    ("lithium", "lithium", "Lithium"),
    ("ferritecore", "ferrite-core", "FerriteCore"),
    ("immediatelyfast", "immediatelyfast", "ImmediatelyFast"),
    ("entityculling", "entityculling", "Entity Culling"),
    ("badoptimizations", "badoptimizations", "BadOptimizations"),
    ("modmenu", "modmenu", "Mod Menu"),
];

#[derive(Clone)]
pub struct Env {
    pub user_dir: PathBuf,
    pub game_dir: PathBuf,
    pub log: Arc<dyn Fn(String) + Send + Sync>,

    pub progress: Arc<dyn Fn(String, String) + Send + Sync>,
}

impl Env {
    fn profile(&self, id: &str) -> Result<Value, String> {
        profiles::get(&self.user_dir, &self.game_dir, id)
    }
}

fn inst_of(p: &Value) -> PathBuf {
    PathBuf::from(p["dir"].as_str().unwrap_or(""))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20 { '_' } else { c })
        .take(120)
        .collect();
    if s.is_empty() { "archivo".to_string() } else { s }
}

fn base_name(s: &str) -> String {
    s.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

fn strip_disabled(s: &str) -> &str {
    s.strip_suffix(".disabled").unwrap_or(s)
}

fn has_ext(name: &str, ext: &str) -> bool {
    name.to_lowercase().ends_with(ext)
}

#[derive(Default, Clone)]
pub struct Index {
    pub files: Map<String, Value>,
    pub unmatched: Map<String, Value>,
}

pub fn read_index(dir: &Path) -> Index {
    let parsed = std::fs::read_to_string(dir.join("nova-content.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok());
    let obj = |v: &Value| v.as_object().cloned().unwrap_or_default();
    match parsed {
        Some(j) => Index { files: obj(&j["files"]), unmatched: obj(&j["unmatched"]) },
        None => Index::default(),
    }
}

pub fn write_index(dir: &Path, idx: &Index) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let v = json!({ "files": idx.files, "unmatched": idx.unmatched });
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("nova-content.json"), text).map_err(|e| e.to_string())
}

type JarCache = Mutex<HashMap<PathBuf, (u128, modcompat::JarInfo)>>;

fn jar_info(file: &Path) -> Option<modcompat::JarInfo> {
    static CACHE: OnceLock<JarCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mtime = std::fs::metadata(file)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    if let Some((m, info)) = cache.lock().ok()?.get(file) {
        if *m == mtime {
            return Some(info.clone());
        }
    }
    let info = modcompat::read_mod_jar(file);
    if let Ok(mut c) = cache.lock() {
        c.insert(file.to_path_buf(), (mtime, info.clone()));
    }
    Some(info)
}

fn is_managed(meta: Option<&Value>) -> bool {
    meta.is_some_and(|m| truthy(&m["managed"]))
}

fn is_core_mod(base: &str) -> bool {
    let l = base.to_lowercase();
    l.starts_with("nova-client") || l.starts_with("trident-client") || l.starts_with("fabric-api")
}

pub fn list_content(env: &Env, id: &str) -> Result<Value, String> {
    let p = env.profile(id)?;
    let dir = inst_of(&p);
    let idx = read_index(&dir);
    let (loader, mc) = (p["loader"].as_str().unwrap_or(""), p["mc"].as_str().unwrap_or(""));
    let mut out: Vec<Value> = Vec::new();
    for (kind, sub) in [("mod", "mods"), ("resourcepack", "resourcepacks")] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for entry in rd.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            let full = entry.path();
            let Ok(st) = std::fs::metadata(&full) else { continue };
            let enabled = !file.ends_with(".disabled");
            let base = strip_disabled(&file).to_string();
            if kind == "mod" && !has_ext(&base, ".jar") {
                continue;
            }
            if kind == "resourcepack" && !(st.is_dir() || has_ext(&base, ".zip")) {
                continue;
            }
            let meta = idx.files.get(&format!("{sub}/{base}"));
            if is_managed(meta) {
                continue;
            }
            if kind == "mod" && is_core_mod(&base) {
                continue;
            }
            let m = |k: &str| meta.and_then(|m| m[k].as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string());
            let (mut name, mut version, mut author) = (m("title"), m("versionNumber"), m("author"));
            let mut compat = Value::Null;
            if kind == "mod" {
                let info = jar_info(&full).unwrap_or_default();
                name = name.or_else(|| info.name.clone().filter(|s| !s.is_empty()));
                version = version.or_else(|| info.version.clone().filter(|s| !s.is_empty()));
                author = author.or_else(|| info.author.clone().filter(|s| !s.is_empty()));
                compat = modcompat::check_compat(&info, loader, mc);
            }
            let fallback = {
                let b = base.clone();
                let l = b.to_lowercase();
                if l.ends_with(".jar") || l.ends_with(".zip") { b[..b.len() - 4].to_string() } else { b }
            };
            out.push(json!({
                "file": file,
                "base": base,
                "type": kind,
                "enabled": enabled,
                "name": name.unwrap_or(fallback),
                "author": author.unwrap_or_default(),
                "version": version.unwrap_or_default(),
                "source": if meta.is_some_and(|m| m["source"].as_str() == Some("modrinth")) { "Modrinth" } else { "Local" },
                "projectId": meta.map(|m| m["projectId"].clone()).filter(|v| !v.is_null()).unwrap_or(Value::Null),
                "icon": meta.map(|m| m["icon"].clone()).filter(|v| !v.is_null()).unwrap_or(Value::Null),
                "compat": compat,
                "size": st.len()
            }));
        }
    }
    out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
    Ok(Value::Array(out))
}

fn content_path(inst: &Path, file: &str) -> (&'static str, String, PathBuf) {
    let base = base_name(file);
    let clean = strip_disabled(&base).to_string();
    let sub = if has_ext(&clean, ".zip") { "resourcepacks" } else { "mods" };
    let path = inst.join(sub).join(&base);
    (sub, clean, path)
}

pub fn toggle_content(env: &Env, id: &str, file: &str, enabled: bool) -> Result<(), String> {
    let p = env.profile(id)?;
    let (sub, clean, from) = content_path(&inst_of(&p), file);
    let to = inst_of(&p).join(sub).join(if enabled { clean.clone() } else { format!("{clean}.disabled") });
    if from != to && from.exists() {
        std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

pub fn remove_content(env: &Env, id: &str, file: &str) -> Result<(), String> {
    let p = env.profile(id)?;
    let dir = inst_of(&p);
    let (sub, clean, path) = content_path(&dir, file);
    remove_path(&path);
    let mut idx = read_index(&dir);
    idx.files.remove(&format!("{sub}/{clean}"));
    write_index(&dir, &idx)
}

pub fn add_local(env: &Env, id: &str, paths: &[String]) -> Result<Value, String> {
    let p = env.profile(id)?;
    let dir = inst_of(&p);
    let (loader, mc) = (p["loader"].as_str().unwrap_or(""), p["mc"].as_str().unwrap_or(""));
    let mut n = 0;
    let mut incompatible: Vec<Value> = Vec::new();
    for src in paths {
        let name = safe_name(&base_name(src));
        let sub = if has_ext(&name, ".zip") { "resourcepacks" } else if has_ext(&name, ".jar") { "mods" } else { continue };
        std::fs::create_dir_all(dir.join(sub)).map_err(|e| e.to_string())?;
        let dest = dir.join(sub).join(&name);
        std::fs::copy(src, &dest).map_err(|e| format!("No se pudo copiar {name}: {e}"))?;
        n += 1;
        if sub == "mods" {
            let info = jar_info(&dest).unwrap_or_default();
            let c = modcompat::check_compat(&info, loader, mc);
            if c["status"] == "bad" {
                let shown = info.name.clone().unwrap_or_else(|| name.trim_end_matches(".jar").to_string());
                incompatible.push(json!({ "name": shown, "reason": c["reason"] }));
            }
        }
    }
    Ok(json!({ "n": n, "incompatible": incompatible }))
}

pub fn copy_from(env: &Env, id: &str, source_id: &str) -> Result<Value, String> {
    if id == source_id {
        return Err("Elegí otro perfil.".to_string());
    }
    let dst = env.profile(id)?;
    let src = env.profile(source_id)?;
    let (from, to) = (inst_of(&src), inst_of(&dst));
    for sub in ["mods", "resourcepacks", "config", "shaderpacks"] {
        profiles::copy_dir_all(&from.join(sub), &to.join(sub)).map_err(|e| e.to_string())?;
    }
    let a = read_index(&from);
    let mut b = read_index(&to);
    for (k, v) in a.files {
        b.files.insert(k, v);
    }
    write_index(&to, &b)?;
    env.profile(id)
}

fn api() -> String {
    if cfg!(test) {
        if let Ok(u) = std::env::var("NOVA_MODRINTH_API") {
            return u;
        }
    }
    MODRINTH.to_string()
}

async fn mr_send(req: reqwest::RequestBuilder) -> Result<Value, String> {
    let res = req
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("No se pudo conectar con Modrinth ({e})"))?;
    if !res.status().is_success() {
        return Err(format!("Modrinth respondió HTTP {}", res.status().as_u16()));
    }
    res.json::<Value>().await.map_err(|e| format!("Modrinth envió una respuesta no válida ({e})"))
}

fn mr_url(path: &str, params: &[(&str, String)]) -> Result<reqwest::Url, String> {
    reqwest::Url::parse_with_params(&format!("{}{}", api(), path), params).map_err(|e| e.to_string())
}

async fn mr_get(path: &str, params: &[(&str, String)]) -> Result<Value, String> {
    mr_send(dl::client().get(mr_url(path, params)?)).await
}

async fn mr_post(path: &str, body: &Value) -> Result<Value, String> {
    mr_send(dl::client().post(mr_url(path, &[])?).json(body)).await
}

fn enc(s: &str) -> String {
    reqwest::Url::parse_with_params("http://x/", &[("k", s)])
        .map(|u| u.query().unwrap_or("").trim_start_matches("k=").to_string())
        .unwrap_or_else(|_| s.to_string())
}

fn is_fabric(p: &Value) -> bool {
    p["loader"].as_str() == Some("fabric")
}

fn loaders_for(p: &Value, kind: &str) -> Option<Vec<&'static str>> {
    if kind == "resourcepack" || kind == "shader" || !is_fabric(p) { None } else { Some(vec!["fabric"]) }
}

fn norm_kind(t: &str) -> &'static str {
    match t {
        "resourcepack" => "resourcepack",
        "shader" => "shader",
        _ => "mod",
    }
}

fn hit_view(h: &Value, kind: &str, installed: &HashSet<String>, extra: bool) -> Value {
    let pid = h["project_id"].as_str().unwrap_or("");
    let mut v = json!({
        "projectId": h["project_id"],
        "slug": h["slug"],
        "title": h["title"],
        "author": h["author"],
        "description": h["description"],
        "icon": h["icon_url"].as_str().map_or(Value::Null, |s| json!(s)),
        "downloads": h["downloads"],
        "type": kind,
        "installed": installed.contains(pid)
    });
    if extra {
        v["follows"] = h["follows"].clone();
        v["updated"] = h["date_modified"].clone();
    }
    v
}

fn installed_projects(dir: &Path) -> HashSet<String> {
    read_index(dir)
        .files
        .values()
        .filter_map(|f| f["projectId"].as_str().map(|s| s.to_string()))
        .collect()
}

pub async fn search(env: &Env, id: &str, q: &Value) -> Result<Value, String> {
    let p = env.profile(id)?;
    let kind = if q["type"].as_str() == Some("resourcepack") { "resourcepack" } else { "mod" };
    let mc = p["mc"].as_str().unwrap_or("");
    let mut facets = vec![vec![format!("project_type:{kind}")], vec![format!("versions:{mc}")]];
    if kind == "mod" && is_fabric(&p) {
        facets.push(vec!["categories:fabric".to_string()]);
    }
    let query = q["query"].as_str().unwrap_or("").to_string();
    let offset = q["offset"].as_u64().unwrap_or(0);
    let index = if query.is_empty() { "downloads" } else { "relevance" };
    let r = mr_get(
        "/search",
        &[
            ("query", query),
            ("facets", serde_json::to_string(&facets).unwrap_or_default()),
            ("limit", "20".into()),
            ("offset", offset.to_string()),
            ("index", index.into()),
        ],
    )
    .await?;
    let installed = installed_projects(&inst_of(&p));
    let hits: Vec<Value> = r["hits"].as_array().map(|a| a.iter().map(|h| hit_view(h, kind, &installed, false)).collect()).unwrap_or_default();
    Ok(json!({ "total": r["total_hits"], "hits": hits }))
}

fn clean_list(v: &Value) -> Vec<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"^[A-Za-z0-9._+-]{1,40}$").expect("regex válida"));
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).filter(|s| re.is_match(s)).map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

pub async fn browse(env: &Env, q: &Value) -> Result<Value, String> {
    let kind = q["type"].as_str().unwrap_or("mod");
    let mut facets = vec![vec![format!("project_type:{kind}")]];
    let vs = clean_list(&q["versions"]);
    let ls = if kind == "mod" || kind == "modpack" { clean_list(&q["loaders"]) } else { vec![] };
    let cs = clean_list(&q["categories"]);
    if !vs.is_empty() {
        facets.push(vs.iter().map(|v| format!("versions:{v}")).collect());
    }
    if !ls.is_empty() {
        facets.push(ls.iter().map(|l| format!("categories:{l}")).collect());
    }
    if !cs.is_empty() {
        facets.push(cs.iter().map(|c| format!("categories:{c}")).collect());
    }
    let query = q["query"].as_str().unwrap_or("").to_string();
    let index = match q["sort"].as_str() {
        Some(s @ ("relevance" | "downloads" | "follows" | "newest" | "updated")) => s,
        _ => if query.is_empty() { "downloads" } else { "relevance" },
    };
    let limit = q["limit"].as_f64().filter(|n| *n >= 1.0).unwrap_or(20.0).min(50.0) as u64;
    let offset = q["offset"].as_f64().filter(|n| *n >= 0.0).unwrap_or(0.0) as u64;
    let r = mr_get(
        "/search",
        &[
            ("query", query),
            ("facets", serde_json::to_string(&facets).unwrap_or_default()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            ("index", index.into()),
        ],
    )
    .await?;
    let installed = match q["profileId"].as_str() {
        Some(pid) => env.profile(pid).map(|p| installed_projects(&inst_of(&p))).unwrap_or_default(),
        None => HashSet::new(),
    };
    let hits: Vec<Value> = r["hits"].as_array().map(|a| a.iter().map(|h| hit_view(h, kind, &installed, true)).collect()).unwrap_or_default();
    Ok(json!({ "total": r["total_hits"], "hits": hits }))
}

pub fn targets_other_mc(label: &str, mc: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();

    let re = R.get_or_init(|| Regex::new(r"(?i)[+\-_](mc)?((?:1|2\d)\.\d{1,2}(?:\.\d{1,2})?)").expect("regex válida"));
    let new_mc = !mc.starts_with("1.");
    let mut other = false;
    let mut pos = 0;
    while pos <= label.len() {
        let Some(c) = re.captures_at(label, pos) else { break };
        let whole = c.get(0).expect("coincidencia completa");
        let rest = &label[whole.end()..];
        let run: String = rest.chars().take_while(|ch| ch.is_ascii_digit() || *ch == '.').collect();
        if !run.chars().any(|ch| ch.is_ascii_digit()) {
            let v = &c[2];
            if v == mc {
                return false;
            }
            let same_family = v.starts_with("1.") != new_mc;
            if same_family || c.get(1).is_some() {
                other = true;
            }
        }
        pos = whole.start() + 1;
        while pos < label.len() && !label.is_char_boundary(pos) {
            pos += 1;
        }
    }
    other
}

pub fn is_prerelease(label: &str) -> bool {
    let l = label.to_lowercase();
    ["alpha", "beta", "-rc", "+rc", ".rc", "pre", "snapshot"].iter().any(|t| l.contains(t))
}

fn version_fits_mc(v: &Value, mc: &str) -> bool {
    let files = v["files"].as_array().cloned().unwrap_or_default();
    let file = files.iter().find(|f| f["primary"].as_bool().unwrap_or(false)).or_else(|| files.first());
    let fname = file.and_then(|f| f["filename"].as_str()).unwrap_or("");
    !targets_other_mc(v["version_number"].as_str().unwrap_or(""), mc) && !targets_other_mc(fname, mc)
}

async fn best_version(p: &Value, project_id: &str, kind: &str, strict: bool) -> Result<Option<Value>, String> {
    let mc = p["mc"].as_str().unwrap_or("");
    let mut params = vec![("game_versions", serde_json::to_string(&[mc]).unwrap_or_default())];
    if let Some(l) = loaders_for(p, kind) {
        params.push(("loaders", serde_json::to_string(&l).unwrap_or_default()));
    }
    let res = mr_get(&format!("/project/{}/version", enc(project_id)), &params).await?;
    let mut list: Vec<Value> = res.as_array().cloned().unwrap_or_default();
    if strict {
        list.retain(|v| version_fits_mc(v, mc));
    }
    let pick = list.iter().find(|v| v["version_type"].as_str() == Some("release")).or_else(|| list.first()).cloned();
    Ok(pick)
}

async fn install_version(p: &Value, version: &Value, kind: &str, extra: Value) -> Result<String, String> {
    let dir = inst_of(p);
    let sub = match kind {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        _ => "mods",
    };
    let files = version["files"].as_array().cloned().unwrap_or_default();
    let file = files
        .iter()
        .find(|f| f["primary"].as_bool().unwrap_or(false))
        .or_else(|| files.first())
        .ok_or("Esa versión no tiene archivos.")?;
    let name = safe_name(file["filename"].as_str().ok_or("Modrinth no informó el nombre del archivo.")?);
    let url = file["url"].as_str().ok_or("Modrinth no informó la URL del archivo.")?;
    let dest = dir.join(sub).join(&name);
    dl::download(url, &dest, file["hashes"]["sha1"].as_str(), |_, _| {}).await?;
    let mut idx = read_index(&dir);
    let mut meta = json!({
        "source": "modrinth",
        "projectId": version["project_id"],
        "versionId": version["id"],
        "versionNumber": version["version_number"]
    });
    if let (Some(m), Some(e)) = (meta.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            m.insert(k.clone(), v.clone());
        }
    }
    idx.files.insert(format!("{sub}/{name}"), meta);
    write_index(&dir, &idx)?;
    Ok(name)
}

async fn project_info(project_id: &str) -> Result<(String, Value), String> {
    let pr = mr_get(&format!("/project/{}", enc(project_id)), &[]).await?;
    Ok((pr["title"].as_str().unwrap_or("").to_string(), pr["icon_url"].as_str().map_or(Value::Null, |s| json!(s))))
}

pub fn install_project<'a>(
    env: &'a Env,
    id: &'a str,
    project_id: &'a str,
    kind: &'a str,
    extra: Value,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send + 'a>> {
    Box::pin(async move {
        let p = env.profile(id)?;
        let dir = inst_of(&p);
        let kind = norm_kind(kind);
        if read_index(&dir).files.values().any(|f| f["projectId"].as_str() == Some(project_id)) {
            return Ok(vec![]);
        }
        let version = best_version(&p, project_id, kind, false).await?.ok_or_else(|| {
            format!(
                "No hay una versión compatible con Minecraft {}{}.",
                p["mc"].as_str().unwrap_or(""),
                if is_fabric(&p) && kind == "mod" { " + Fabric" } else { "" }
            )
        })?;
        let (title, icon) = project_info(project_id).await?;
        let author = match mr_get(&format!("/project/{}/members", enc(project_id)), &[]).await {
            Ok(m) => {
                let list = m.as_array().cloned().unwrap_or_default();
                list.iter()
                    .find(|x| x["role"].as_str() == Some("Owner"))
                    .or_else(|| list.first())
                    .and_then(|x| x["user"]["username"].as_str())
                    .unwrap_or("")
                    .to_string()
            }
            Err(_) => String::new(),
        };
        let mut meta = json!({ "title": title, "icon": icon, "author": author });
        if let (Some(m), Some(e)) = (meta.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                m.insert(k.clone(), v.clone());
            }
        }
        install_version(&p, &version, kind, meta).await?;
        let mut done = vec![title];
        if kind == "mod" && depth < 3 {
            for dep in version["dependencies"].as_array().cloned().unwrap_or_default() {
                let Some(dep_id) = dep["project_id"].as_str() else { continue };
                if dep["dependency_type"].as_str() != Some("required") || dep_id.eq_ignore_ascii_case("fabric-api") {
                    continue;
                }
                match install_project(env, id, dep_id, "mod", json!({ "dependency": true }), depth + 1).await {
                    Ok(more) => done.extend(more),
                    Err(e) => (env.log)(format!("[Contenido] No se pudo instalar una dependencia: {e}")),
                }
            }
        }
        Ok(done)
    })
}

fn sha1_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub async fn identify_local(env: &Env, id: &str) -> Result<u32, String> {
    static BUSY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let busy = BUSY.get_or_init(|| Mutex::new(HashSet::new()));
    if !busy.lock().map_err(|e| e.to_string())?.insert(id.to_string()) {
        return Ok(0);
    }
    let result = identify_inner(env, id).await;
    if let Ok(mut b) = busy.lock() {
        b.remove(id);
    }
    result
}

async fn identify_inner(env: &Env, id: &str) -> Result<u32, String> {
    let p = env.profile(id)?;
    let dir = inst_of(&p);
    let mut idx = read_index(&dir);
    let mut by_hash: HashMap<String, (String, String)> = HashMap::new();
    for (kind, sub) in [("mod", "mods"), ("resourcepack", "resourcepacks")] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for entry in rd.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            let base = strip_disabled(&file).to_string();
            if !(if kind == "mod" { has_ext(&base, ".jar") } else { has_ext(&base, ".zip") }) {
                continue;
            }
            if kind == "mod" && is_core_mod(&base) {
                continue;
            }
            let key = format!("{sub}/{base}");
            if idx.files.contains_key(&key) {
                continue;
            }
            let Ok(st) = std::fs::metadata(entry.path()) else { continue };
            if !st.is_file() {
                continue;
            }
            let mtime = st.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
            let stamp = format!("{}:{}", st.len(), mtime);
            if idx.unmatched.get(&key).and_then(|v| v.as_str()) == Some(stamp.as_str()) {
                continue;
            }
            if let Ok(h) = sha1_file(&entry.path()) {
                by_hash.insert(h, (key, stamp));
            }
        }
    }
    if by_hash.is_empty() {
        return Ok(0);
    }
    let hashes: Vec<&String> = by_hash.keys().collect();
    let found = mr_post("/version_files", &json!({ "hashes": hashes, "algorithm": "sha1" })).await?;
    let found = found.as_object().cloned().unwrap_or_default();
    let mut ids: Vec<String> = found.values().filter_map(|v| v["project_id"].as_str().map(|s| s.to_string())).collect();
    ids.sort();
    ids.dedup();
    let mut projects: HashMap<String, Value> = HashMap::new();
    if !ids.is_empty() {
        let list = mr_get("/projects", &[("ids", serde_json::to_string(&ids).unwrap_or_default())]).await?;
        for pr in list.as_array().cloned().unwrap_or_default() {
            if let Some(pid) = pr["id"].as_str() {
                projects.insert(pid.to_string(), pr.clone());
            }
        }
    }
    let mut n = 0;
    for (hash, (key, stamp)) in by_hash {
        let Some(v) = found.get(&hash) else {
            idx.unmatched.insert(key, json!(stamp));
            continue;
        };
        let pr = v["project_id"].as_str().and_then(|pid| projects.get(pid));
        idx.files.insert(
            key.clone(),
            json!({
                "source": "modrinth",
                "projectId": v["project_id"],
                "versionId": v["id"],
                "versionNumber": v["version_number"],
                "title": pr.and_then(|p| p["title"].as_str()).map_or(Value::Null, |s| json!(s)),
                "icon": pr.and_then(|p| p["icon_url"].as_str()).map_or(Value::Null, |s| json!(s)),
                "detected": true
            }),
        );
        idx.unmatched.remove(&key);
        n += 1;
    }
    write_index(&dir, &idx)?;
    Ok(n)
}

pub async fn check_updates(env: &Env, id: &str) -> Result<Vec<(String, Value)>, String> {
    let p = env.profile(id)?;
    let dir = inst_of(&p);
    let idx = read_index(&dir);
    let mc = p["mc"].as_str().unwrap_or("");

    let mut mods: HashMap<String, String> = HashMap::new();
    let mut other: HashMap<String, String> = HashMap::new();
    for (key, meta) in &idx.files {
        if meta["source"].as_str() != Some("modrinth") {
            continue;
        }
        let full = dir.join(key);
        let disabled = dir.join(format!("{key}.disabled"));
        let hash = if full.is_file() {
            sha1_file(&full).ok()
        } else if disabled.is_file() {
            sha1_file(&disabled).ok()
        } else {
            None
        };
        if let Some(h) = hash {
            if key.starts_with("mods/") { mods.insert(h, key.clone()); } else { other.insert(h, key.clone()); }
        }
    }
    let mut out: Vec<(String, Value)> = Vec::new();
    for (group, with_loader) in [(mods, true), (other, false)] {
        if group.is_empty() {
            continue;
        }
        let hashes: Vec<&String> = group.keys().collect();
        let mut body = json!({ "hashes": hashes, "algorithm": "sha1", "game_versions": [mc] });
        if with_loader && is_fabric(&p) {
            body["loaders"] = json!(["fabric"]);
        }
        let res = mr_post("/version_files/update", &body).await?;
        for (hash, v) in res.as_object().cloned().unwrap_or_default() {
            if let Some(key) = group.get(&hash) {
                let installed = idx.files[key]["versionNumber"].as_str().unwrap_or("");
                let to_pre = v["version_type"].as_str().is_some_and(|t| t != "release");
                if to_pre && !is_prerelease(installed) {
                    continue;
                }
                if idx.files[key]["versionId"] != v["id"] {
                    out.push((key.clone(), v));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

async fn apply_update(env: &Env, p: &Value, key: &str, version: &Value) -> Result<(), String> {
    let dir = inst_of(p);
    let idx = read_index(&dir);
    let old = idx.files.get(key).cloned().unwrap_or(Value::Null);
    let kind = if key.starts_with("resourcepacks/") { "resourcepack" } else { "mod" };
    let was_disabled = !dir.join(key).exists() && dir.join(format!("{key}.disabled")).exists();
    let mut extra = json!({});
    for k in ["title", "icon", "author", "managed", "dependency"] {
        if !old[k].is_null() {
            extra[k] = old[k].clone();
        }
    }
    let name = install_version(p, version, kind, extra).await?;
    let sub = key.split('/').next().unwrap_or("mods");
    let new_key = format!("{sub}/{name}");
    if new_key != key {
        for f in [key.to_string(), format!("{key}.disabled")] {
            let _ = std::fs::remove_file(dir.join(f));
        }
        let mut idx2 = read_index(&dir);
        idx2.files.remove(key);
        write_index(&dir, &idx2)?;
    }
    if was_disabled {
        toggle_content(env, p["id"].as_str().unwrap_or(""), &name, false)?;
    }
    Ok(())
}

pub async fn update_content(env: &Env, id: &str, file: &str) -> Result<Value, String> {
    let p = env.profile(id)?;
    let (sub, clean, _) = content_path(&inst_of(&p), file);
    let want = format!("{sub}/{clean}");
    let found = check_updates(env, id).await?.into_iter().find(|(k, _)| *k == want);
    match found {
        None => Ok(json!({ "updated": false })),
        Some((key, v)) => {
            apply_update(env, &p, &key, &v).await?;
            Ok(json!({ "updated": true, "version": v["version_number"] }))
        }
    }
}

pub async fn sync_all(env: &Env, id: &str) -> u32 {
    let p = match env.profile(id) {
        Ok(p) => p,
        Err(_) => return 0,
    };
    match check_updates(env, id).await {
        Ok(ups) => {
            for (key, v) in &ups {
                if let Err(e) = apply_update(env, &p, key, v).await {
                    (env.log)(format!("[Sincronización] {key}: {e}"));
                }
            }
            ups.len() as u32
        }
        Err(e) => {
            (env.log)(format!("[Sincronización] No se pudo buscar actualizaciones: {e}"));
            0
        }
    }
}

pub fn get_perf(env: &Env, id: &str) -> Result<Value, String> {
    let p = env.profile(id)?;
    let idx = read_index(&inst_of(&p));
    let mut by_key: HashMap<String, &Value> = HashMap::new();
    for meta in idx.files.values() {
        if let Some(k) = meta["managed"].as_str().and_then(|m| m.strip_prefix("perf:")) {
            by_key.insert(k.to_string(), meta);
        }
    }
    let mods: Vec<Value> = PERF_MODS
        .iter()
        .map(|(key, _, name)| {
            json!({
                "key": key,
                "name": name,
                "installed": by_key.contains_key(*key),
                "version": by_key.get(*key).and_then(|m| m["versionNumber"].as_str()).unwrap_or("")
            })
        })
        .collect();
    Ok(json!({ "available": is_fabric(&p), "mods": mods }))
}

fn remove_perf_files(dir: &Path, only: Option<&str>) {
    let mut idx = read_index(dir);
    let keys: Vec<String> = idx
        .files
        .iter()
        .filter(|(_, m)| {
            m["managed"].as_str().is_some_and(|g| g.starts_with("perf:") && only.is_none_or(|k| g == format!("perf:{k}")))
        })
        .map(|(k, _)| k.clone())
        .collect();
    for key in keys {
        for f in [key.clone(), format!("{key}.disabled")] {
            let _ = std::fs::remove_file(dir.join(f));
        }
        idx.files.remove(&key);
    }
    let _ = write_index(dir, &idx);
}

pub async fn ensure_perf(env: &Env, id: &str) -> Result<(), String> {
    let p = env.profile(id)?;
    if !is_fabric(&p) {
        return Ok(());
    }
    let dir = inst_of(&p);
    let mc = p["mc"].as_str().unwrap_or("").to_string();

    let old: Vec<String> = read_index(&dir)
        .files
        .values()
        .filter_map(|m| m["managed"].as_str().and_then(|g| g.strip_prefix("perf:")).map(|k| k.to_string()))
        .filter(|k| !PERF_MODS.iter().any(|(key, _, _)| key == k))
        .collect();
    for k in old {
        remove_perf_files(&dir, Some(&k));
    }
    for (key, slug, name) in PERF_MODS {
        let idx = read_index(&dir);
        let tag = format!("perf:{key}");
        let have: Vec<(&String, &Value)> = idx.files.iter().filter(|(_, f)| f["managed"].as_str() == Some(tag.as_str())).collect();

        let wrong = have.iter().any(|(k, f)| {
            targets_other_mc(f["versionNumber"].as_str().unwrap_or(""), &mc) || targets_other_mc(&base_name(k), &mc)
        });
        let pre_key = format!("perf-pre:{slug}:{mc}");
        let pre = have.iter().any(|(_, f)| is_prerelease(f["versionNumber"].as_str().unwrap_or("")));
        if wrong {
            (env.log)(format!("[Rendimiento] {name}: la versión instalada no es para Minecraft {mc}, se quita."));
            remove_perf_files(&dir, Some(key));
        } else if pre && !crate::dl::checked_recently(&pre_key, 6) {
            crate::dl::mark_checked(&pre_key);
            match best_version(&p, slug, "mod", true).await {
                Ok(Some(v)) if v["version_type"].as_str() == Some("release") && have.iter().all(|(_, f)| f["versionId"] != v["id"]) => {
                    (env.log)(format!("[Rendimiento] {name}: se cambia la versión de prueba por la estable {}.", v["version_number"].as_str().unwrap_or("")));
                    remove_perf_files(&dir, Some(key));
                }
                _ => continue,
            }
        } else if !have.is_empty() {
            continue;
        }

        let none_key = format!("perf-none2:{slug}:{mc}");
        if crate::dl::checked_recently(&none_key, 24) {
            continue;
        }
        let step = async {
            let Some(version) = best_version(&p, slug, "mod", true).await? else {
                (env.log)(format!("[Rendimiento] {name} no tiene una versión para Minecraft {mc}, se omite."));
                crate::dl::mark_checked(&none_key);
                return Ok::<(), String>(());
            };
            let (title, icon) = project_info(slug).await?;
            install_version(&p, &version, "mod", json!({ "title": title, "icon": icon, "managed": tag })).await?;
            (env.log)(format!("[Rendimiento] {name} {} instalado.", version["version_number"].as_str().unwrap_or("")));
            Ok(())
        };
        if let Err(e) = step.await {
            (env.log)(format!("[Rendimiento] {name}: {e}"));
        }
    }
    Ok(())
}

pub fn list_worlds(env: &Env, id: &str) -> Result<Value, String> {
    let saves = inst_of(&env.profile(id)?).join("saves");
    let Ok(rd) = std::fs::read_dir(&saves) else { return Ok(json!([])) };
    let mut out: Vec<(f64, Value)> = Vec::new();
    for entry in rd.flatten() {
        let dir = entry.path();
        let level = dir.join("level.dat");
        let Ok(st) = std::fs::metadata(&level) else { continue };
        if !dir.is_dir() {
            continue;
        }
        let last = st.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let icon = std::fs::metadata(dir.join("icon.png"))
            .ok()
            .filter(|m| m.is_file() && m.len() < 200_000)
            .and_then(|_| std::fs::read(dir.join("icon.png")).ok())
            .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
        out.push((
            last,
            json!({ "name": entry.file_name().to_string_lossy(), "lastPlayed": last.round(), "icon": icon }),
        ));
    }
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Value::Array(out.into_iter().map(|x| x.1).collect()))
}

pub fn world_dir(env: &Env, id: &str, name: &str) -> Result<PathBuf, String> {
    Ok(inst_of(&env.profile(id)?).join("saves").join(base_name(name)))
}

pub fn read_log(env: &Env, id: &str) -> Result<String, String> {
    let file = inst_of(&env.profile(id)?).join("logs").join("latest.log");
    let read = || -> std::io::Result<String> {
        let mut f = std::fs::File::open(&file)?;
        let size = f.metadata()?.len();
        let len = size.min(200 * 1024);
        f.seek(SeekFrom::Start(size - len))?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).to_string())
    };
    Ok(read().unwrap_or_default())
}

pub fn snapshot(env: &Env, id: &str) -> Result<Value, String> {
    let p = env.profile(id)?;
    let idx = read_index(&inst_of(&p));
    let mut keys: Vec<&String> = idx.files.keys().collect();
    keys.sort();
    let mut content: Vec<Value> = Vec::new();
    for key in keys {
        let meta = &idx.files[key];
        if meta["source"].as_str() != Some("modrinth") || truthy(&meta["dependency"]) || truthy(&meta["managed"]) {
            continue;
        }
        content.push(json!({
            "type": if key.starts_with("resourcepacks/") { "resourcepack" } else { "mod" },
            "projectId": meta["projectId"],
            "versionId": meta["versionId"],
            "title": meta["title"]
        }));
    }
    Ok(json!({
        "format": "nova-profile",
        "v": 1,
        "name": p["name"],
        "mc": p["mc"],
        "loader": p["loader"],
        "nova": truthy(&p["nova"]),

        "perf": { "master": true, "off": [] },
        "content": content
    }))
}

pub fn export_to_file(env: &Env, id: &str, path: &Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&snapshot(env, id)?).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

pub fn share_code(env: &Env, id: &str) -> Result<String, String> {
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;
    let json = serde_json::to_vec(&snapshot(env, id)?).map_err(|e| e.to_string())?;
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&json).map_err(|e| e.to_string())?;
    let packed = z.finish().map_err(|e| e.to_string())?;
    Ok(format!("NOVA1:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(packed)))
}

pub fn parse_share(text: &str) -> Result<Value, String> {
    let t = text.trim().trim_start_matches('\u{feff}');
    let data: Value = if let Some(code) = t.strip_prefix("NOVA1:") {
        let broken = || "El código está incompleto o dañado.".to_string();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(code.trim().trim_end_matches('='))
            .map_err(|_| broken())?;
        let mut out = String::new();
        flate2::read::ZlibDecoder::new(&bytes[..]).read_to_string(&mut out).map_err(|_| broken())?;
        serde_json::from_str(&out).map_err(|_| broken())?
    } else {
        serde_json::from_str(t).map_err(|_| "Eso no parece un perfil de Trident Client.".to_string())?
    };
    if data["format"].as_str() != Some("nova-profile") || data["mc"].as_str().is_none_or(|m| m.is_empty()) {
        return Err("Eso no parece un perfil de Trident Client.".to_string());
    }
    Ok(data)
}

pub async fn import_data(env: &Env, data: &Value) -> Result<Value, String> {
    let created = profiles::create(
        &env.user_dir,
        &env.game_dir,
        &json!({ "name": data["name"], "mc": data["mc"], "loader": data["loader"], "nova": data["nova"], "origin": "pack" }),
    )?;
    let id = created["id"].as_str().unwrap_or("").to_string();
    let real = env.profile(&id)?;
    let content = data["content"].as_array().cloned().unwrap_or_default();
    let total = content.len();
    let mut failed: Vec<String> = Vec::new();
    for (i, c) in content.iter().enumerate() {
        let label = c["title"].as_str().filter(|s| !s.is_empty()).unwrap_or("contenido");
        (env.progress)(id.clone(), format!("Instalando {label} ({}/{total})…", i + 1));
        let project_id = c["projectId"].as_str().unwrap_or("");
        let kind = norm_kind(c["type"].as_str().unwrap_or("mod"));
        let step = async {
            let mc = real["mc"].as_str().unwrap_or("");
            let mut version: Option<Value> = None;
            if let Some(vid) = c["versionId"].as_str() {
                if let Ok(v) = mr_get(&format!("/version/{}", enc(vid)), &[]).await {
                    if v["game_versions"].as_array().is_some_and(|g| g.iter().any(|x| x.as_str() == Some(mc))) {
                        version = Some(v);
                    }
                }
            }
            let version = match version {
                Some(v) => v,
                None => best_version(&real, project_id, kind, false).await?.ok_or("sin versión compatible")?,
            };
            let (title, icon) = project_info(project_id).await?;
            install_version(&real, &version, kind, json!({ "title": title, "icon": icon })).await?;
            Ok::<(), String>(())
        };
        if let Err(e) = step.await {
            let name = c["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(project_id);
            failed.push(format!("{name} ({e})"));
        }
    }
    (env.progress)(id.clone(), String::new());
    Ok(json!({ "profile": env.profile(&id)?, "failed": failed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn versiones_de_otro_minecraft() {
        assert!(targets_other_mc("5.20.3+mc1.21.4", "1.21.8"));
        assert!(!targets_other_mc("5.20.3+mc1.21.8", "1.21.8"));
        assert!(targets_other_mc("modernfix-fabric-5.20.3+1.21.4.jar", "1.21.8"));
        assert!(!targets_other_mc("sodium-fabric-0.6.13+mc1.21.8.jar", "1.21.8"));
        assert!(targets_other_mc("mod-1.0-mc1.20.1", "1.21"));

        assert!(!targets_other_mc("0.5.13", "1.21.8"));
        assert!(!targets_other_mc("mod-1.2.345", "1.21.8"));
        assert!(!targets_other_mc("", "1.21.8"));
        assert!(!targets_other_mc("iris-fabric-1.11.7+mc26.3.jar", "26.3"));
        assert!(!targets_other_mc("1.11.7+26.3-fabric", "26.3"));
        assert!(!targets_other_mc("ImmediatelyFast-Fabric-1.17.1+26.3.jar", "26.3"));
        assert!(!targets_other_mc("entityculling-fabric-1.12.2-mc26.3.jar", "26.3"));
        assert!(!targets_other_mc("iris-fabric-1.8.8+mc1.21.4.jar", "1.21.4"));
        assert!(targets_other_mc("iris-fabric-1.11.7+mc26.2.jar", "26.3"));
        assert!(targets_other_mc("sodium-fabric-0.6.13+mc1.21.8.jar", "26.3"));
        assert!(!targets_other_mc("lithium-fabric-0.26.4-mc26.3.jar", "26.3"));
        assert!(targets_other_mc("lithium-fabric-0.26.4-mc26.3.jar", "1.21.8"));
        assert!(is_prerelease("mc26.3-0.9.4-beta.1-fabric"));
        assert!(is_prerelease("1.0.0-alpha.2"));
        assert!(!is_prerelease("mc26.3-0.9.3-fabric"));
        assert!(!is_prerelease("1.11.7+26.3-fabric"));
    }

    #[test]
    fn nombres_seguros() {
        assert_eq!(safe_name("a<b>c:d.jar"), "a_b_c_d.jar");
        assert_eq!(safe_name(""), "archivo");
        assert_eq!(base_name("C:\\x\\y\\mod.jar"), "mod.jar");
        assert_eq!(base_name("../../mod.jar"), "mod.jar");
    }

    #[test]
    fn codigo_de_compartir_ida_y_vuelta() {
        let data = json!({ "format": "nova-profile", "v": 1, "name": "X", "mc": "1.21.4", "loader": "fabric", "content": [] });
        use flate2::{write::ZlibEncoder, Compression};
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(data.to_string().as_bytes()).unwrap();
        let code = format!("NOVA1:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(z.finish().unwrap()));
        assert_eq!(parse_share(&code).unwrap()["mc"], "1.21.4");
        assert_eq!(parse_share(&data.to_string()).unwrap()["name"], "X");
        assert!(parse_share("NOVA1:@@@").is_err());
        assert!(parse_share("{\"format\":\"otra\"}").is_err());
        assert!(parse_share("hola").is_err());
    }

    #[test]
    fn lee_codigo_de_electron() {
        let code = "NOVA1:eJwljrFqw0AQRH8lTL0WKLjaOincuXJjVJxPe-TM3a1YrQ1GqPC35ceClHKGx5tZkNRqcDCaPsNhMk25CAhPcE9ooQoYXzKP8vFdJLppA6FGMPrus--OIBQNoxgYKdwsR9AuA7s9hDCJJfCCGmbfqP9WUwJfUbL_5EfFsBKiNpfm4OsCf03bcNURhMn0LtFPIxjn7ZvYnLXt-QKCZy8b_fvGOqx_F4dEPw";
        let d = parse_share(code).unwrap();
        assert_eq!(d["name"], "Desde Electron");
        assert_eq!(d["perf"]["off"][0], "lithium");
        assert_eq!(d["content"][0]["title"], "Á");
    }

    fn env(tmp: &Path) -> Env {
        Env {
            user_dir: tmp.join("user"),
            game_dir: tmp.join("game"),
            log: Arc::new(|_| {}),
            progress: Arc::new(|_, _| {}),
        }
    }

    fn jar(path: &Path, id: &str, ver: &str, mc: &str) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(format!(r#"{{"id":"{id}","name":"{id}","version":"{ver}","depends":{{"minecraft":"{mc}"}}}}"#).as_bytes()).unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn lista_activa_y_borra_contenido() {
        let tmp = std::env::temp_dir().join(format!("nova-c1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let e = env(&tmp);
        let p = profiles::create(&e.user_dir, &e.game_dir, &json!({ "name": "T", "mc": "1.21.4", "loader": "fabric" })).unwrap();
        let id = p["id"].as_str().unwrap().to_string();
        let dir = inst_of(&p);
        jar(&dir.join("mods/zeta.jar"), "zeta", "1.0", "~1.21.4");
        jar(&dir.join("mods/alfa.jar"), "alfa", "2.0", "1.20.1");
        jar(&dir.join("mods/fabric-api-0.1.jar"), "fabric-api", "0.1", "*");
        std::fs::create_dir_all(dir.join("resourcepacks/carpeta")).unwrap();
        std::fs::write(dir.join("resourcepacks/pack.zip"), b"x").unwrap();

        let l = list_content(&e, &id).unwrap();
        let names: Vec<&str> = l.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["alfa", "carpeta", "pack", "zeta"]);
        let alfa = &l[0];
        assert_eq!(alfa["compat"]["status"], "bad");
        assert_eq!(l[3]["compat"]["status"], "ok");
        assert_eq!(l[3]["source"], "Local");

        toggle_content(&e, &id, "zeta.jar", false).unwrap();
        assert!(dir.join("mods/zeta.jar.disabled").exists());
        let l = list_content(&e, &id).unwrap();
        let z = l.as_array().unwrap().iter().find(|x| x["name"] == "zeta").unwrap();
        assert_eq!(z["enabled"], false);
        toggle_content(&e, &id, "zeta.jar.disabled", true).unwrap();
        assert!(dir.join("mods/zeta.jar").exists());

        remove_content(&e, &id, "../../zeta.jar").unwrap();
        assert!(!dir.join("mods/zeta.jar").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn agrega_archivos_locales_y_avisa_incompatibles() {
        let tmp = std::env::temp_dir().join(format!("nova-c2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let e = env(&tmp);
        let p = profiles::create(&e.user_dir, &e.game_dir, &json!({ "mc": "1.21.4", "loader": "fabric" })).unwrap();
        let id = p["id"].as_str().unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        jar(&tmp.join("viejo.jar"), "viejo", "1", "1.19.2");
        jar(&tmp.join("bueno.jar"), "bueno", "1", "1.21.x");
        std::fs::write(tmp.join("nota.txt"), b"x").unwrap();
        let paths: Vec<String> = ["viejo.jar", "bueno.jar", "nota.txt"].iter().map(|n| tmp.join(n).to_string_lossy().to_string()).collect();
        let r = add_local(&e, id, &paths).unwrap();
        assert_eq!(r["n"], 2);
        assert_eq!(r["incompatible"].as_array().unwrap().len(), 1);
        assert_eq!(r["incompatible"][0]["name"], "viejo");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mundos_servidores_y_registro() {
        let tmp = std::env::temp_dir().join(format!("nova-c3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let e = env(&tmp);
        let p = profiles::create(&e.user_dir, &e.game_dir, &json!({ "mc": "1.21.4" })).unwrap();
        let id = p["id"].as_str().unwrap();
        let dir = inst_of(&p);
        std::fs::create_dir_all(dir.join("saves/Mundo1")).unwrap();
        std::fs::write(dir.join("saves/Mundo1/level.dat"), b"x").unwrap();
        std::fs::write(dir.join("saves/Mundo1/icon.png"), b"png").unwrap();
        std::fs::create_dir_all(dir.join("saves/SinNivel")).unwrap();
        let w = list_worlds(&e, id).unwrap();
        assert_eq!(w.as_array().unwrap().len(), 1);
        assert_eq!(w[0]["name"], "Mundo1");
        assert!(w[0]["icon"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(world_dir(&e, id, "../../x").unwrap().ends_with("saves/x"));

        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("logs/latest.log"), "hola\nmundo").unwrap();
        assert_eq!(read_log(&e, id).unwrap(), "hola\nmundo");
        std::fs::write(dir.join("logs/latest.log"), "x".repeat(300_000)).unwrap();
        assert_eq!(read_log(&e, id).unwrap().len(), 200 * 1024);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let routes = routes(&base);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    req.extend_from_slice(&buf[..n]);

                    if n == 0 || String::from_utf8_lossy(&req).contains("\r\n\r\n") {
                        break;
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                match routes.iter().find(|(p, _)| path.starts_with(p.as_str())) {
                    Some((_, body)) => {
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        base
    }

    fn sha1b(b: &[u8]) -> String {
        hex::encode(Sha1::digest(b))
    }

    #[tokio::test]
    async fn instala_actualiza_exporta_e_importa_con_un_modrinth_falso() {
        std::env::set_var("NO_PROXY", "127.0.0.1");
        std::env::set_var("no_proxy", "127.0.0.1");
        let tmp = std::env::temp_dir().join(format!("nova-c4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let (j1, j2, jd) = (tmp.join("v1.jar"), tmp.join("v2.jar"), tmp.join("dep.jar"));
        jar(&j1, "coolmod", "1.0", "~1.21.4");
        jar(&j2, "coolmod", "2.0", "~1.21.4");
        jar(&jd, "depmod", "1.0", "~1.21.4");
        let (b1, b2, bd) = (std::fs::read(&j1).unwrap(), std::fs::read(&j2).unwrap(), std::fs::read(&jd).unwrap());
        let (c1, c2, cd) = (b1.clone(), b2.clone(), bd.clone());

        let base = serve(move |base| {
            let ver = |vid: &str, pid: &str, num: &str, file: &str, body: &[u8], deps: Value| {
                json!({
                    "id": vid, "project_id": pid, "version_number": num, "version_type": "release", "game_versions": ["1.21.4"],
                    "files": [{ "primary": true, "filename": file, "url": format!("{base}/files/{file}"), "hashes": { "sha1": sha1b(body) } }],
                    "dependencies": deps
                })
            };
            let v1 = ver("V1", "COOL", "1.0", "cool-1.0.jar", &c1, json!([
                { "project_id": "DEP", "dependency_type": "required" },
                { "project_id": "fabric-api", "dependency_type": "required" },
                { "project_id": "OPT", "dependency_type": "optional" }
            ]));
            let v2 = ver("V2", "COOL", "2.0", "cool-2.0.jar", &c2, json!([]));
            let vd = ver("VD", "DEP", "1.0", "dep-1.0.jar", &cd, json!([]));
            let j = |v: Value| v.to_string().into_bytes();
            vec![
                ("/v2/project/COOL/version".into(), j(json!([v1.clone()]))),
                ("/v2/project/DEP/version".into(), j(json!([vd]))),
                ("/v2/project/COOL/members".into(), j(json!([{ "role": "Owner", "user": { "username": "Ana" } }]))),
                ("/v2/project/DEP/members".into(), b"[]".to_vec()),
                ("/v2/project/COOL".into(), j(json!({ "title": "Cool Mod", "icon_url": "https://x/i.png", "slug": "cool" }))),
                ("/v2/project/DEP".into(), j(json!({ "title": "Dep Mod", "icon_url": null, "slug": "dep" }))),
                ("/v2/version_files/update".into(), j(json!({ sha1b(&c1): v2 }))),
                ("/v2/version_files".into(), j(json!({ sha1b(&c1): v1 }))),
                ("/v2/projects".into(), j(json!([{ "id": "COOL", "title": "Cool Mod", "icon_url": null }]))),
                ("/v2/version/V1".into(), j(v1.clone())),
                ("/v2/search".into(), j(json!({ "total_hits": 1, "hits": [{ "project_id": "COOL", "slug": "cool", "title": "Cool Mod", "author": "Ana", "description": "d", "icon_url": null, "downloads": 5, "follows": 2, "date_modified": "2025-01-01" }] }))),
                ("/files/cool-1.0.jar".into(), c1.clone()),
                ("/files/cool-2.0.jar".into(), c2.clone()),
                ("/files/dep-1.0.jar".into(), cd.clone()),
            ]
        });
        std::env::set_var("NOVA_MODRINTH_API", format!("{base}/v2"));

        let logs = Arc::new(Mutex::new(Vec::<String>::new()));
        let prog = Arc::new(Mutex::new(Vec::<String>::new()));
        let (l2, p2) = (logs.clone(), prog.clone());
        let e = Env {
            user_dir: tmp.join("user"),
            game_dir: tmp.join("game"),
            log: Arc::new(move |m| l2.lock().unwrap().push(m)),
            progress: Arc::new(move |_, t| p2.lock().unwrap().push(t)),
        };
        let p = profiles::create(&e.user_dir, &e.game_dir, &json!({ "name": "Prueba", "mc": "1.21.4", "loader": "fabric" })).unwrap();
        let id = p["id"].as_str().unwrap().to_string();
        let dir = inst_of(&p);

        let r = search(&e, &id, &json!({ "query": "cool" })).await.unwrap();
        assert_eq!(r["total"], 1);
        assert_eq!(r["hits"][0]["installed"], false);
        let r = browse(&e, &json!({ "query": "cool", "type": "mod", "profileId": id, "limit": 500 })).await.unwrap();
        assert_eq!(r["hits"][0]["follows"], 2);

        let done = install_project(&e, &id, "COOL", "mod", json!({}), 0).await.unwrap();
        assert_eq!(done, ["Cool Mod", "Dep Mod"], "{:?}", logs.lock().unwrap());
        assert!(dir.join("mods/cool-1.0.jar").exists());
        assert!(dir.join("mods/dep-1.0.jar").exists());
        assert!(install_project(&e, &id, "COOL", "mod", json!({}), 0).await.unwrap().is_empty());
        let idx = read_index(&dir);
        assert_eq!(idx.files["mods/cool-1.0.jar"]["author"], "Ana");
        assert_eq!(idx.files["mods/dep-1.0.jar"]["dependency"], true);

        let l = list_content(&e, &id).unwrap();
        let cool = l.as_array().unwrap().iter().find(|x| x["name"] == "Cool Mod").unwrap();
        assert_eq!(cool["source"], "Modrinth");
        assert_eq!(cool["version"], "1.0");

        toggle_content(&e, &id, "cool-1.0.jar", false).unwrap();
        let ups = check_updates(&e, &id).await.unwrap();
        assert_eq!(ups.len(), 1);
        assert_eq!(ups[0].0, "mods/cool-1.0.jar");
        let r = update_content(&e, &id, "cool-1.0.jar.disabled").await.unwrap();
        assert_eq!(r["updated"], true);
        assert_eq!(r["version"], "2.0");
        assert!(!dir.join("mods/cool-1.0.jar.disabled").exists());
        assert!(dir.join("mods/cool-2.0.jar.disabled").exists());
        assert_eq!(read_index(&dir).files["mods/cool-2.0.jar"]["versionId"], "V2");

        let id2 = profiles::create(&e.user_dir, &e.game_dir, &json!({ "mc": "1.21.4", "loader": "fabric" })).unwrap();
        let id2 = id2["id"].as_str().unwrap().to_string();
        let dir2 = inst_of(&e.profile(&id2).unwrap());
        std::fs::write(dir2.join("mods/a-mano.jar"), &b1).unwrap();
        jar(&dir2.join("mods/otro.jar"), "otro", "1", "*");
        assert_eq!(identify_local(&e, &id2).await.unwrap(), 1);
        let i2 = read_index(&dir2);
        assert_eq!(i2.files["mods/a-mano.jar"]["projectId"], "COOL");
        assert_eq!(i2.files["mods/a-mano.jar"]["detected"], true);
        assert!(i2.unmatched.contains_key("mods/otro.jar"));
        assert_eq!(identify_local(&e, &id2).await.unwrap(), 0);

        assert_eq!(sync_all(&e, &id2).await, 1);
        assert!(dir2.join("mods/cool-2.0.jar").exists());
        assert!(!dir2.join("mods/a-mano.jar").exists());

        let snap = snapshot(&e, &id).unwrap();
        assert_eq!(snap["content"].as_array().unwrap().len(), 1);
        let code = share_code(&e, &id).unwrap();
        assert!(code.starts_with("NOVA1:"));
        let imported = import_data(&e, &parse_share(&code).unwrap()).await.unwrap();
        assert!(imported["failed"].as_array().unwrap().is_empty(), "{imported}");
        assert_eq!(imported["profile"]["name"], "Prueba");
        assert_eq!(imported["profile"]["origin"], "pack");
        let ndir = inst_of(&imported["profile"]);
        assert!(ndir.join("mods/cool-1.0.jar").exists());
        assert!(prog.lock().unwrap().iter().any(|t| t.starts_with("Instalando Cool Mod (1/1)")));
        assert_eq!(prog.lock().unwrap().last().unwrap(), "");

        ensure_perf(&e, &id).await.unwrap();
        let pf = get_perf(&e, &id).unwrap();
        assert_eq!(pf["mods"].as_array().unwrap().len(), PERF_MODS.len());
        assert!(logs.lock().unwrap().iter().any(|m| m.starts_with("[Rendimiento] Sodium")));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

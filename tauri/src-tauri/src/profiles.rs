use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PROFILE_KEYS: &[&str] = &[
    "name", "mc", "loader", "nova", "autoSync", "folder", "ramGB", "jvmArgs", "pinned", "fabricApi",
];

struct Store {
    raw: Map<String, Value>,
}

impl Store {
    fn load(user_dir: &Path) -> Store {
        let mut raw: Map<String, Value> = fs::read_to_string(user_dir.join("profiles.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        for k in ["folders", "profiles", "servers"] {
            if !raw.get(k).is_some_and(|v| v.is_array()) {
                raw.insert(k.to_string(), json!([]));
            }
        }
        Store { raw }
    }

    fn save(&self, user_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(user_dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&Value::Object(self.raw.clone())).map_err(|e| e.to_string())?;
        fs::write(user_dir.join("profiles.json"), text).map_err(|e| e.to_string())
    }

    fn arr(&self, key: &str) -> &Vec<Value> {
        self.raw
            .get(key)
            .and_then(|v| v.as_array())
            .expect("Store::load garantiza que existe")
    }

    fn arr_mut(&mut self, key: &str) -> &mut Vec<Value> {
        self.raw
            .get_mut(key)
            .and_then(|v| v.as_array_mut())
            .expect("Store::load garantiza que existe")
    }

    fn folders(&self) -> Vec<String> {
        self.arr("folders")
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }

    fn find(&self, id: &str) -> Result<usize, String> {
        self.arr("profiles")
            .iter()
            .position(|p| p["id"].as_str() == Some(id))
            .ok_or_else(|| "El perfil ya no existe.".to_string())
    }
}

pub fn new_id() -> String {
    let b: [u8; 6] = rand::random();
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

fn limit_chars(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn instance_dir(p: &Value, game_dir: &Path) -> PathBuf {
    if p["legacy"].as_bool().unwrap_or(false) {
        game_dir.to_path_buf()
    } else {
        game_dir.join("instances").join(p["id"].as_str().unwrap_or(""))
    }
}

fn view(p: &Value, game_dir: &Path) -> Value {
    let mut v = p.clone();
    if let Some(o) = v.as_object_mut() {
        let label = match p["loader"].as_str() {
            Some("fabric") => "Fabric",
            Some("bedrock") => "Bedrock",
            _ => "Vanilla",
        };
        o.insert("loaderLabel".to_string(), json!(label));
        o.insert(
            "dir".to_string(),
            json!(instance_dir(p, game_dir).to_string_lossy()),
        );
    }
    v
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn list(user_dir: &Path, game_dir: &Path) -> Value {
    let s = Store::load(user_dir);
    json!({
        "folders": s.arr("folders"),
        "profiles": s.arr("profiles").iter().map(|p| view(p, game_dir)).collect::<Vec<Value>>()
    })
}

pub fn get(user_dir: &Path, game_dir: &Path, id: &str) -> Result<Value, String> {
    let s = Store::load(user_dir);
    let i = s.find(id)?;
    Ok(view(&s.arr("profiles")[i], game_dir))
}

pub fn create(user_dir: &Path, game_dir: &Path, input: &Value) -> Result<Value, String> {
    let mut s = Store::load(user_dir);
    let bedrock = input["loader"].as_str() == Some("bedrock");
    let mc = if bedrock { "Bedrock".to_string() } else { input["mc"].as_str().unwrap_or("").trim().to_string() };
    if mc.is_empty() {
        return Err("Elegí una versión de Minecraft.".to_string());
    }
    let fabric = !bedrock && input["loader"].as_str() == Some("fabric");
    let loader = if bedrock { "bedrock" } else if fabric { "fabric" } else { "vanilla" };
    let typed = input["name"].as_str().unwrap_or("").trim().to_string();
    let name = if !typed.is_empty() {
        typed
    } else if bedrock {
        "Minecraft Bedrock".to_string()
    } else {
        format!("{} {}", if fabric { "Fabric" } else { "Minecraft" }, mc)
    };
    let folder = input["folder"]
        .as_str()
        .filter(|f| s.folders().iter().any(|x| x == f))
        .unwrap_or("")
        .to_string();
    let nova = fabric && input["nova"].as_bool() != Some(false);
    let auto_sync = input["autoSync"].as_bool() != Some(false);
    let origin = if input["origin"].as_str() == Some("pack") { "pack" } else { "custom" };
    let short_name = limit_chars(&name, 60);
    let p = json!({
        "id": new_id(),
        "name": short_name,
        "mc": mc,
        "loader": loader,
        "nova": nova,
        "autoSync": auto_sync,
        "fabricApi": true,
        "folder": folder,
        "pinned": false,
        "legacy": false,
        "origin": origin,
        "created": now_ms(),
        "lastPlayed": 0,
        "playtimeSec": 0,
        "perf": { "master": false, "off": [] }
    });
    s.arr_mut("profiles").push(p.clone());
    s.save(user_dir)?;
    let _ = fs::create_dir_all(instance_dir(&p, game_dir).join("mods"));
    Ok(view(&p, game_dir))
}

pub fn update(user_dir: &Path, game_dir: &Path, id: &str, patch: &Value) -> Result<Value, String> {
    let mut s = Store::load(user_dir);
    let i = s.find(id)?;
    let folders = s.folders();
    {
        let p = &mut s.arr_mut("profiles")[i];
        let is_bedrock = p["loader"].as_str() == Some("bedrock");
        for k in PROFILE_KEYS {
            let v = match patch.get(*k) {
                Some(v) => v.clone(),
                None => continue,
            };
            if is_bedrock && matches!(*k, "mc" | "loader" | "nova") {
                continue;
            }
            let new_value = match *k {
                "name" => {
                    let t = limit_chars(v.as_str().unwrap_or(""), 60);
                    if t.is_empty() { p["name"].clone() } else { json!(t) }
                }
                "ramGB" => {
                    if truthy(&v) {
                        let n = v
                            .as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
                            .filter(|n| n.is_finite())
                            .unwrap_or(0.0)
                            .clamp(1.0, 64.0);
                        if n.fract() == 0.0 { json!(n as i64) } else { json!(n) }
                    } else {
                        Value::Null
                    }
                }
                "folder" => {
                    let f = v.as_str().unwrap_or("");
                    if folders.iter().any(|x| x == f) { json!(f) } else { json!("") }
                }
                "loader" => {
                    let l = if v.as_str() == Some("fabric") { "fabric" } else { "vanilla" };
                    json!(l)
                }
                "nova" | "autoSync" | "pinned" | "fabricApi" => json!(truthy(&v)),
                _ => v,
            };
            p[*k] = new_value;
        }
        if p["loader"].as_str() != Some("fabric") {
            p["nova"] = json!(false);
        }
        if is_bedrock {
            p["loader"] = json!("bedrock");
        }
    }
    s.save(user_dir)?;
    Ok(view(&s.arr("profiles")[i], game_dir))
}

pub fn remove(user_dir: &Path, game_dir: &Path, id: &str) -> Result<(), String> {
    let mut s = Store::load(user_dir);
    let i = s.find(id)?;
    let p = s.arr_mut("profiles").remove(i);
    s.save(user_dir)?;

    if !p["legacy"].as_bool().unwrap_or(false) {
        let _ = fs::remove_dir_all(instance_dir(&p, game_dir));
    }
    Ok(())
}

pub fn duplicate(user_dir: &Path, game_dir: &Path, id: &str) -> Result<Value, String> {
    let p = {
        let s = Store::load(user_dir);
        let i = s.find(id)?;
        s.arr("profiles")[i].clone()
    };
    let created = create(
        user_dir,
        game_dir,
        &json!({
            "name": format!("{} (copia)", p["name"].as_str().unwrap_or("")),
            "mc": p["mc"],
            "loader": p["loader"],
            "nova": p["nova"],
            "autoSync": p["autoSync"],
            "folder": p["folder"]
        }),
    )?;
    let new_id_str = created["id"].as_str().unwrap_or("").to_string();

    let mut s = Store::load(user_dir);
    let ci = s.find(&new_id_str)?;
    {
        let c = &mut s.arr_mut("profiles")[ci];
        c["perf"] = if p["perf"].is_null() { json!({ "master": false, "off": [] }) } else { p["perf"].clone() };
        if !p["ramGB"].is_null() {
            c["ramGB"] = p["ramGB"].clone();
        }
        if !p["jvmArgs"].is_null() {
            c["jvmArgs"] = p["jvmArgs"].clone();
        }
    }
    s.save(user_dir)?;

    let from = instance_dir(&p, game_dir);
    let to = instance_dir(&s.arr("profiles")[ci], game_dir);
    for sub in ["mods", "resourcepacks", "config", "shaderpacks"] {
        let _ = copy_dir_all(&from.join(sub), &to.join(sub));
    }
    for f in ["options.txt", "nova-content.json"] {
        if from.join(f).exists() {
            let _ = fs::copy(from.join(f), to.join(f));
        }
    }
    Ok(view(&s.arr("profiles")[ci], game_dir))
}

pub fn touch_played(user_dir: &Path, id: &str, seconds: f64) {
    let mut s = Store::load(user_dir);
    if let Ok(i) = s.find(id) {
        let p = &mut s.arr_mut("profiles")[i];
        let total = p["playtimeSec"].as_f64().unwrap_or(0.0) + seconds.max(0.0).round();
        p["lastPlayed"] = json!(now_ms());
        p["playtimeSec"] = json!(total as u64);
        let _ = s.save(user_dir);
    }
}

pub fn set_folders(user_dir: &Path, action: &str, name: &str) -> Result<Value, String> {
    let mut s = Store::load(user_dir);
    let n = limit_chars(name, 40);
    if action == "add" {
        if n.is_empty() {
            return Err("Escribí un nombre para la carpeta.".to_string());
        }
        if !s.folders().contains(&n) {
            s.arr_mut("folders").push(json!(n));
        }
    } else if action == "remove" {
        s.arr_mut("folders").retain(|f| f.as_str() != Some(n.as_str()));
        for p in s.arr_mut("profiles").iter_mut() {
            if p["folder"].as_str() == Some(n.as_str()) {
                p["folder"] = json!("");
            }
        }
    }
    s.save(user_dir)?;
    Ok(json!(s.arr("folders")))
}

pub fn dir_of(user_dir: &Path, game_dir: &Path, id: &str) -> Result<PathBuf, String> {
    let s = Store::load(user_dir);
    let i = s.find(id)?;
    Ok(instance_dir(&s.arr("profiles")[i], game_dir))
}

pub fn ids(user_dir: &Path) -> Vec<String> {
    Store::load(user_dir)
        .arr("profiles")
        .iter()
        .filter_map(|p| p["id"].as_str().map(|s| s.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfil_bedrock() {
        let base = std::env::temp_dir().join(format!("trident-prof-bedrock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (u, g) = (base.join("u"), base.join("g"));
        fs::create_dir_all(&u).unwrap();
        let p = create(&u, &g, &json!({ "loader": "bedrock", "mc": "1.21.4", "nova": true })).unwrap();
        assert_eq!(p["loader"], "bedrock");
        assert_eq!(p["mc"], "Bedrock");
        assert_eq!(p["nova"], false);
        assert_eq!(p["name"], "Minecraft Bedrock");
        assert_eq!(p["loaderLabel"], "Bedrock");
        let id = p["id"].as_str().unwrap().to_string();
        let up = update(&u, &g, &id, &json!({ "loader": "fabric", "mc": "1.21.4", "name": "Mi Bedrock" })).unwrap();
        assert_eq!(up["loader"], "bedrock");
        assert_eq!(up["mc"], "Bedrock");
        assert_eq!(up["name"], "Mi Bedrock");
        let _ = fs::remove_dir_all(&base);
    }
}

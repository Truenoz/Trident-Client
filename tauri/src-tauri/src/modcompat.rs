use regex::Regex;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

type Ver = [Option<i64>; 3];

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex válida"))
}

fn parse_ver(s: &str) -> Option<Ver> {
    static R: OnceLock<Regex> = OnceLock::new();
    let t = s.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    let c = re(&R, r"^(\d+)(?:\.(\d+|[xX*]))?(?:\.(\d+|[xX*]))?").captures(t)?;
    let n = |i: usize| -> Option<i64> { c.get(i).and_then(|m| m.as_str().parse::<i64>().ok()) };
    Some([n(1), n(2), n(3)])
}

fn cmp(a: &Ver, b: &Ver) -> i32 {
    for i in 0..3 {
        let x = a[i].unwrap_or(0);
        let y = b[i].unwrap_or(0);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

fn match_atom(atom: &str, v: &Ver) -> Option<bool> {
    static R: OnceLock<Regex> = OnceLock::new();
    let atom = atom.trim();
    if atom.is_empty() || atom == "*" || atom == "x" {
        return Some(true);
    }
    let c = re(&R, r"^(>=|<=|>|<|=|~|\^)?\s*(.+)$").captures(atom)?;
    let op = c.get(1).map_or("=", |m| m.as_str());
    let t = parse_ver(c.get(2)?.as_str())?;
    let wild = t[1].is_none() || t[2].is_none();
    Some(match op {
        "=" => cmp(&[t[0], t[1].or(v[1]), t[2].or(v[2])], v) == 0,
        ">=" => cmp(v, &t) >= 0,
        ">" => {
            if wild {
                cmp(v, &[t[0], Some(t[1].unwrap_or(0) + 1), Some(0)]) >= 0
            } else {
                cmp(v, &t) > 0
            }
        }
        "<=" => cmp(v, &t) <= 0 || (wild && cmp(v, &[t[0], Some(t[1].unwrap_or(99999)), Some(99999)]) <= 0),
        "<" => cmp(v, &t) < 0,
        "~" => v[0] == t[0] && (t[1].is_none() || v[1] == t[1]) && cmp(v, &t) >= 0,
        "^" => v[0] == t[0] && cmp(v, &t) >= 0,
        _ => return None,
    })
}

pub fn match_range(range: &Value, mc: &str) -> Option<bool> {
    static TAIL: OnceLock<Regex> = OnceLock::new();
    let v = parse_ver(mc)?;
    let rest = re(&TAIL, r"^\d+(\.\d+)*").replace(mc, "").to_string();
    if rest.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let items: Vec<String> = match range {
        Value::Array(a) => a.iter().map(|x| x.as_str().map(|s| s.to_string()).unwrap_or_else(|| x.to_string())).collect(),
        Value::String(s) => vec![s.clone()],
        other => vec![other.to_string()],
    };
    let mut unknown = false;
    for alt in items.iter().flat_map(|r| r.split("||")) {
        let mut all = true;
        for a in alt.split_whitespace() {
            match match_atom(a, &v) {
                None => {
                    unknown = true;
                    all = false;
                    break;
                }
                Some(false) => {
                    all = false;
                    break;
                }
                Some(true) => {}
            }
        }
        if all {
            return Some(true);
        }
    }
    if unknown { None } else { Some(false) }
}

pub fn match_maven(range: &str, mc: &str) -> Option<bool> {
    static R: OnceLock<Regex> = OnceLock::new();
    let v = parse_ver(mc)?;
    let parts: Vec<&str> = re(&R, r"[\[(][^\])]*[\])]").find_iter(range).map(|m| m.as_str()).collect();
    if parts.is_empty() {
        return parse_ver(range).map(|t| cmp(&t, &v) == 0);
    }
    for p in parts {
        let lo = p.starts_with('[');
        let hi = p.ends_with(']');
        let inner = &p[1..p.len() - 1];
        let pieces: Vec<&str> = inner.split(',').map(|x| x.trim()).collect();
        if pieces.len() < 2 {
            if let Some(t) = parse_ver(pieces[0]) {
                if cmp(&t, &v) == 0 {
                    return Some(true);
                }
            }
            continue;
        }
        let a = if pieces[0].is_empty() { None } else { parse_ver(pieces[0]) };
        let b = if pieces[1].is_empty() { None } else { parse_ver(pieces[1]) };
        let ok_lo = a.is_none_or(|a| if lo { cmp(&v, &a) >= 0 } else { cmp(&v, &a) > 0 });
        let ok_hi = b.is_none_or(|b| if hi { cmp(&v, &b) <= 0 } else { cmp(&v, &b) < 0 });
        if ok_lo && ok_hi {
            return Some(true);
        }
    }
    Some(false)
}

fn read_toml_dep(text: &str) -> Option<String> {
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    static MOD: OnceLock<Regex> = OnceLock::new();
    static RANGE: OnceLock<Regex> = OnceLock::new();
    for block in re(&SPLIT, r"\[\[dependencies[^\]]*\]\]").split(text).skip(1) {
        if re(&MOD, r#"modId\s*=\s*["']minecraft["']"#).is_match(block) {
            if let Some(c) = re(&RANGE, r#"versionRange\s*=\s*["']([^"']+)["']"#).captures(block) {
                return Some(c[1].to_string());
            }
        }
    }
    None
}

#[derive(Clone, Debug, Default)]
pub struct JarInfo {
    pub loader: Option<String>,
    pub mc_range: Option<Value>,
    pub maven_range: bool,
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
}

fn entry_text<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).trim_start_matches('\u{feff}').to_string())
}

fn non_null(v: &Value) -> Option<Value> {
    if v.is_null() { None } else { Some(v.clone()) }
}

pub fn read_mod_jar(file: &Path) -> JarInfo {
    let mut info = JarInfo::default();
    let Ok(f) = std::fs::File::open(file) else { return info };
    let Ok(mut zip) = zip::ZipArchive::new(f) else { return info };
    let fab = entry_text(&mut zip, "fabric.mod.json");
    let quilt = entry_text(&mut zip, "quilt.mod.json");
    let neo = entry_text(&mut zip, "META-INF/neoforge.mods.toml");
    let forge = entry_text(&mut zip, "META-INF/mods.toml");
    if let Some(t) = fab {
        let Ok(j) = serde_json::from_str::<Value>(&t) else { return info };
        info.loader = Some("fabric".into());
        info.name = j["name"].as_str().filter(|s| !s.is_empty()).or_else(|| j["id"].as_str()).map(|s| s.to_string());
        info.version = j["version"].as_str().map(|s| s.to_string());
        let a = &j["authors"][0];
        info.author = a.as_str().or_else(|| a["name"].as_str()).map(|s| s.to_string());
        info.mc_range = non_null(&j["depends"]["minecraft"]);
    } else if let Some(t) = quilt {
        let Ok(j) = serde_json::from_str::<Value>(&t) else { return info };
        info.loader = Some("quilt".into());
        let dep = j["quilt_loader"]["depends"]
            .as_array()
            .and_then(|a| a.iter().find(|d| d.as_str() == Some("minecraft") || d["id"].as_str() == Some("minecraft")));
        info.mc_range = dep.filter(|d| d.is_object()).and_then(|d| non_null(&d["versions"]));
    } else if let Some(t) = neo {
        info.loader = Some("neoforge".into());
        info.mc_range = read_toml_dep(&t).map(Value::String);
        info.maven_range = true;
    } else if let Some(t) = forge {
        info.loader = Some("forge".into());
        info.mc_range = read_toml_dep(&t).map(Value::String);
        info.maven_range = true;
    }
    info
}

fn loader_name(l: &str) -> &str {
    match l {
        "fabric" => "Fabric",
        "quilt" => "Quilt",
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        "vanilla" => "Vanilla",
        other => other,
    }
}

pub fn check_compat(info: &JarInfo, loader: &str, mc: &str) -> Value {
    let Some(il) = info.loader.as_deref() else {
        return json!({ "status": "unknown", "reason": "No se pudo leer la información del mod." });
    };
    if loader == "fabric" && il != "fabric" && il != "quilt" {
        return json!({ "status": "bad", "reason": format!("Es un mod de {}; este perfil usa Fabric.", loader_name(il)) });
    }
    if loader == "vanilla" {
        return json!({ "status": "bad", "reason": "Este perfil es Vanilla y no carga mods." });
    }
    let Some(range) = &info.mc_range else {
        return json!({ "status": "unknown", "reason": "El mod no indica con qué versión de Minecraft funciona." });
    };
    let res = if info.maven_range {
        match_maven(range.as_str().unwrap_or(""), mc)
    } else {
        match_range(range, mc)
    };
    let req = match range {
        Value::Array(a) => a.iter().map(|x| x.as_str().map(|s| s.to_string()).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" o "),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match res {
        Some(true) => json!({ "status": "ok", "reason": format!("Compatible con Minecraft {mc}."), "requires": req }),
        Some(false) => json!({ "status": "bad", "reason": format!("Pide Minecraft {req}; este perfil usa {mc}."), "requires": req }),
        None => json!({ "status": "unknown", "reason": format!("Pide Minecraft {req}; no se pudo comprobar contra {mc}."), "requires": req }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rangos_de_fabric() {
        assert_eq!(match_range(&json!("~1.21.4"), "1.21.4"), Some(true));
        assert_eq!(match_range(&json!("~1.21.4"), "1.21.8"), Some(true));
        assert_eq!(match_range(&json!("~1.21.9"), "1.21.8"), Some(false));
        assert_eq!(match_range(&json!(">=1.20 <1.21"), "1.20.4"), Some(true));
        assert_eq!(match_range(&json!(">=1.20 <1.21"), "1.21"), Some(false));
        assert_eq!(match_range(&json!(["1.20.x", "1.21.x"]), "1.21.4"), Some(true));
        assert_eq!(match_range(&json!("1.21.x"), "1.20.6"), Some(false));
        assert_eq!(match_range(&json!("1.20 || 1.21.4"), "1.21.4"), Some(true));
        assert_eq!(match_range(&json!("*"), "1.19.2"), Some(true));

        assert_eq!(match_range(&json!(">1.20"), "1.20.4"), Some(false));
        assert_eq!(match_range(&json!(">1.20"), "1.21"), Some(true));
        assert_eq!(match_range(&json!("^1.20"), "1.21.1"), Some(true));
        assert_eq!(match_range(&json!(">=1.21"), "26.1"), Some(true));
        assert_eq!(match_range(&json!(">=1.21"), "1.21-pre1"), None);
    }

    #[test]
    fn rangos_de_maven() {
        assert_eq!(match_maven("[1.21,1.22)", "1.21.4"), Some(true));
        assert_eq!(match_maven("[1.21,1.22)", "1.22"), Some(false));
        assert_eq!(match_maven("[1.21.1]", "1.21.1"), Some(true));
        assert_eq!(match_maven("[1.21.1]", "1.21.4"), Some(false));
        assert_eq!(match_maven("1.21.1", "1.21.1"), Some(true));
        assert_eq!(match_maven("(,1.21]", "1.20.4"), Some(true));
    }

    #[test]
    fn toml_de_forge() {
        let t = "[[dependencies.foo]]\nmodId=\"forge\"\nversionRange=\"[47,)\"\n[[dependencies.foo]]\nmodId=\"minecraft\"\nversionRange=\"[1.20.1,1.21)\"\n";
        assert_eq!(read_toml_dep(t).as_deref(), Some("[1.20.1,1.21)"));
    }

    #[test]
    fn lee_jars_de_fabric_y_forge() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("nova-mc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, entry: &str, body: &str| {
            let p = dir.join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            z.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
            z.finish().unwrap();
            p
        };
        let fab = mk("a.jar", "fabric.mod.json", r#"{"id":"x","name":"Mod X","version":"1.0","authors":[{"name":"Ana"}],"depends":{"minecraft":"~1.21.4"}}"#);
        let info = read_mod_jar(&fab);
        assert_eq!(info.name.as_deref(), Some("Mod X"));
        assert_eq!(info.author.as_deref(), Some("Ana"));
        assert_eq!(check_compat(&info, "fabric", "1.21.4")["status"], "ok");
        assert_eq!(check_compat(&info, "fabric", "1.20.1")["status"], "bad");
        assert_eq!(check_compat(&info, "vanilla", "1.21.4")["status"], "bad");
        let forge = mk("b.jar", "META-INF/mods.toml", "[[dependencies.m]]\nmodId=\"minecraft\"\nversionRange=\"[1.20.1]\"\n");
        let fi = read_mod_jar(&forge);
        assert_eq!(fi.loader.as_deref(), Some("forge"));
        assert_eq!(check_compat(&fi, "fabric", "1.20.1")["status"], "bad");
        assert_eq!(check_compat(&JarInfo::default(), "fabric", "1.20.1")["status"], "unknown");
        let _ = std::fs::remove_dir_all(dir);
    }
}

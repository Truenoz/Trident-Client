use crate::account::{self, Session};
use crate::launcher::{self, WINDOWS};
use crate::{content, discord, fabric, java, net, nova, profiles, settings, AppState};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub struct GameInfo {
    pub profile_id: String,

    pub kill: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Default)]
pub struct Runtime {
    pub session: Mutex<Option<Session>>,
    pub game: Mutex<Option<GameInfo>>,
    pub active_profile: Mutex<Option<String>>,
    pub log_buf: Mutex<Vec<String>>,

    pub presence: Mutex<Option<discord::Presence>>,
}

pub fn log(app: &AppHandle, msg: impl Into<String>) {
    let msg = msg.into();
    let rt = &app.state::<AppState>().rt;
    if let Ok(mut buf) = rt.log_buf.lock() {
        buf.push(msg.clone());
        if buf.len() > 3000 {
            let cut = buf.len() - 2000;
            buf.drain(0..cut);
        }
    }
    let _ = app.emit("game:log", msg);
}

pub fn status(app: &AppHandle, state: &str, text: &str, percent: Option<f64>) {
    let rt = &app.state::<AppState>().rt;
    let profile = if state == "idle" { None } else { rt.active_profile.lock().ok().and_then(|g| g.clone()) };
    let _ = app.emit("game:status", json!({ "state": state, "text": text, "percent": percent, "profileId": profile }));
}

pub fn is_running(app: &AppHandle) -> Option<String> {
    app.state::<AppState>().rt.game.lock().ok().and_then(|g| g.as_ref().map(|x| x.profile_id.clone()))
}

fn set_session(app: &AppHandle, s: Option<Session>) {
    if let Ok(mut g) = app.state::<AppState>().rt.session.lock() {
        *g = s;
    }
}

pub fn get_session(app: &AppHandle) -> Option<Session> {
    app.state::<AppState>().rt.session.lock().ok().and_then(|g| g.clone())
}

static SESSION_FRESH: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn establish(app: &AppHandle, s: Session) -> Value {
    if !s.offline {
        *SESSION_FRESH.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    }
    let user_dir = app.state::<AppState>().user_dir.clone();

    match &s.refresh_token {
        Some(t) => account::save_refresh(&user_dir, t),
        None => account::clear_refresh(&user_dir),
    }
    if s.offline {
        account::save_offline(&user_dir, &s.name);
    } else {
        account::clear_offline(&user_dir);
    }
    let info = s.info();
    set_session(app, Some(s));
    info
}

pub async fn login(app: &AppHandle) -> Result<Value, String> {
    let s = account::login(app).await?;
    Ok(establish(app, s))
}

pub fn login_offline(app: &AppHandle, name: &str) -> Result<Value, String> {
    let s = account::offline_session(name)?;
    Ok(establish(app, s))
}

pub fn logout(app: &AppHandle) {
    set_session(app, None);
    let user_dir = app.state::<AppState>().user_dir.clone();
    account::clear_refresh(&user_dir);
    account::clear_offline(&user_dir);
}

pub async fn restore(app: &AppHandle) -> Result<Value, String> {
    let user_dir = app.state::<AppState>().user_dir.clone();
    let token = match account::load_refresh(&user_dir) {
        Some(t) => t,
        None => {
            return Ok(match account::load_offline(&user_dir).and_then(|n| account::offline_session(&n).ok()) {
                Some(s) => establish(app, s),
                None => Value::Null,
            });
        }
    };
    match account::refresh(&token).await {
        Ok(s) => Ok(establish(app, s)),
        Err(e) => {
            eprintln!("No se pudo restaurar la sesión: {e}");
            if account::is_rejected(&e) {
                account::clear_refresh(&user_dir);
            }
            Ok(Value::Null)
        }
    }
}

fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

fn remove_nova_jars(mods_dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(mods_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if (n.starts_with("nova-client") || n.starts_with("trident-client")) && n.ends_with(".jar") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

async fn prepare_profile(app: &AppHandle, profile: &Value, inst: &Path, root: &Path) -> Result<(String, bool), String> {
    let mc = profile["mc"].as_str().ok_or("El perfil no tiene versión de Minecraft.")?.to_string();
    let progress = |t: String, p: Option<f64>| status(app, "preparing", &t, p);
    let mods_dir = inst.join("mods");
    std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

    if profile["loader"].as_str() != Some("fabric") {

        remove_nova_jars(&mods_dir);
        return Ok((mc, false));
    }

    let custom = fabric::ensure_fabric(root, &mc, &progress).await?;
    if profile["fabricApi"].as_bool() != Some(false) {
        fabric::ensure_fabric_api(&mc, &mods_dir, &progress).await?;
    }

    let env = content::Env {
        user_dir: app.state::<AppState>().user_dir.clone(),
        game_dir: root.to_path_buf(),
        log: { let a = app.clone(); std::sync::Arc::new(move |m| log(&a, m)) },
        progress: std::sync::Arc::new(|_, _| {}),
    };
    let pid = profile["id"].as_str().unwrap_or("");
    if profile["autoSync"].as_bool().unwrap_or(false) {
        status(app, "preparing", "Sincronizando contenido…", None);
        let n = content::sync_all(&env, pid).await;
        if n > 0 {
            log(app, format!("[Sincronización] {n} elemento(s) actualizado(s)."));
        }
    }

    status(app, "preparing", "Preparando mods…", None);
    remove_nova_jars(&mods_dir);
    let perf = async {
        if let Err(e) = content::ensure_perf(&env, pid).await {
            log(app, format!("[Rendimiento] {e}"));
        }
    };
    let trident = async {
        if !profile["nova"].as_bool().unwrap_or(false) {
            return false;
        }
        let user_dir = app.state::<AppState>().user_dir.clone();
        let logger = |m: String| log(app, m);
        match nova::ensure_jar(&user_dir, &mc, &logger).await {
            Ok(Some(jar)) => match nova::install_jar(&jar, &mods_dir.join("trident-client.jar"), &logger) {
                Ok(()) => true,
                Err(e) => { log(app, format!("[Trident] No se pudo copiar el mod: {e}")); false }
            },
            Ok(None) => { log(app, format!("[Trident] No hay mod de Trident para Minecraft {mc}. Se inicia solo con Fabric.")); false }
            Err(e) => { log(app, format!("[Trident] Error al preparar el mod: {e}")); false }
        }
    };
    let ((), with_nova) = tokio::join!(perf, trident);
    Ok((custom, with_nova))
}

pub fn supports_quick_play(mc: &str) -> bool {
    match mc.strip_prefix("1.").and_then(|r| r.split('.').next()).and_then(|n| n.parse::<u32>().ok()) {
        Some(minor) => minor >= 20,
        None => true,
    }
}

fn memory_args(ram_gb: f64) -> [String; 2] {
    let max = ((ram_gb * 1024.0).round() as u64).max(512);
    let min = (((ram_gb.clamp(1.0, 2.0)) * 1024.0).round() as u64).min(max);
    [format!("-Xms{min}M"), format!("-Xmx{max}M")]
}

pub fn perf_jvm_flags(user_jvm: &[String]) -> Vec<String> {
    let own_gc = user_jvm.iter().any(|a| a.starts_with("-XX:+Use") && a.ends_with("GC"));
    if own_gc {
        return Vec::new();
    }
    [
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:+UseG1GC",
        "-XX:G1NewSizePercent=20",
        "-XX:G1ReservePercent=20",
        "-XX:MaxGCPauseMillis=50",
        "-XX:G1HeapRegionSize=16M",
        "-XX:+ParallelRefProcEnabled",
        "-XX:+DisableExplicitGC",
        "-XX:+PerfDisableSharedMem",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn default_options(inst: &Path) {
    let file = inst.join("options.txt");
    if file.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(inst);
    let _ = std::fs::write(file, "renderDistance:10\nsimulationDistance:8\nenableVsync:false\nmaxFps:144\n");
}

#[allow(clippy::too_many_arguments)]
pub fn build_command(
    prep: &launcher::Prepared,
    root: &Path,
    inst: &Path,
    launch_id: &str,
    version_type: &str,
    session: &Session,
    settings: &serde_json::Map<String, Value>,
    ram_gb: f64,
    user_jvm: &[String],
    app_version: &str,
    server: Option<&str>,
    mc: &str,
    nova_net: Option<&str>,
) -> Vec<String> {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let classpath = prep.classpath.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(sep);

    let width = settings.get("winWidth").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = settings.get("winHeight").and_then(|v| v.as_u64()).unwrap_or(0);
    let fullscreen = settings.get("fullscreen").and_then(|v| v.as_bool()).unwrap_or(false);
    let custom_window = fullscreen || (width > 0 && height > 0);

    let mut features: HashSet<String> = HashSet::new();
    if custom_window {
        features.insert("has_custom_resolution".to_string());
    }
    let quick = server.filter(|_| supports_quick_play(mc));
    if quick.is_some() {
        features.insert("is_quick_play_multiplayer".to_string());
    }

    let s = |x: &str| x.to_string();
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert(s("auth_player_name"), session.name.clone());
    vars.insert(s("auth_uuid"), session.uuid_plain());
    vars.insert(s("auth_access_token"), session.access_token.clone());
    vars.insert(s("auth_session"), session.access_token.clone());
    vars.insert(s("auth_xuid"), session.xuid.clone());
    vars.insert(s("clientid"), session.uuid_plain());
    vars.insert(s("user_type"), session.user_type.clone());
    vars.insert(s("user_properties"), s("{}"));
    vars.insert(s("version_name"), s(launch_id));
    vars.insert(s("version_type"), s(version_type));
    vars.insert(s("game_directory"), inst.to_string_lossy().to_string());
    vars.insert(s("assets_root"), root.join("assets").to_string_lossy().to_string());
    vars.insert(s("game_assets"), root.join("assets").join("virtual").join("legacy").to_string_lossy().to_string());
    vars.insert(s("assets_index_name"), prep.asset_index.clone());
    vars.insert(s("natives_directory"), prep.natives_dir.to_string_lossy().to_string());
    vars.insert(s("library_directory"), root.join("libraries").to_string_lossy().to_string());
    vars.insert(s("launcher_name"), s("minecraft-launcher"));
    vars.insert(s("launcher_version"), s(app_version));
    vars.insert(s("classpath"), classpath);
    vars.insert(s("classpath_separator"), s(sep));
    vars.insert(s("resolution_width"), if width > 0 { width.to_string() } else { s("1280") });
    vars.insert(s("resolution_height"), if height > 0 { height.to_string() } else { s("720") });
    if let Some(q) = quick {
        vars.insert(s("quickPlayMultiplayer"), s(q));
    }

    let (json_jvm, mut game) = launcher::build_args(&prep.version, &WINDOWS, &features, &vars);

    let mut args: Vec<String> = Vec::new();
    args.extend(memory_args(ram_gb));
    args.extend(perf_jvm_flags(user_jvm));
    args.extend(user_jvm.iter().cloned());
    args.extend(json_jvm);
    if let Some(l) = &prep.logging_arg {
        args.push(l.clone());
    }
    if let Some(url) = nova_net {
        args.push(format!("-Dtridentclient.net={url}"));
    }

    args.push(prep.version["mainClass"].as_str().unwrap_or("net.minecraft.client.main.Main").to_string());

    if fullscreen {
        game.push(s("--fullscreen"));
    }
    if let (Some(srv), None) = (server, quick) {

        let (host, port) = match srv.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h, Some(p)),
            _ => (srv, None),
        };
        game.push(s("--server"));
        game.push(s(host));
        if let Some(p) = port {
            game.push(s("--port"));
            game.push(s(p));
        }
    }
    args.extend(game);
    args
}

pub async fn launch(app: &AppHandle, profile_id: &str, opts: &Value) -> Result<(), String> {
    let st = app.state::<AppState>();
    {
        let mut g = st.rt.game.lock().map_err(|_| "Estado interno bloqueado.".to_string())?;
        if g.is_some() {
            return Err("El juego ya está en ejecución.".to_string());
        }
        *g = Some(GameInfo { profile_id: profile_id.to_string(), kill: None });
    }
    if let Ok(mut a) = st.rt.active_profile.lock() {
        *a = Some(profile_id.to_string());
    }
    let result = launch_inner(app, profile_id, opts).await;
    if result.is_err() {
        if let Ok(mut g) = st.rt.game.lock() {
            *g = None;
        }
        if let Ok(mut a) = st.rt.active_profile.lock() {
            *a = None;
        }
        crate::updater::on_game_exit(app);
    }
    result
}

async fn launch_inner(app: &AppHandle, profile_id: &str, opts: &Value) -> Result<(), String> {
    let st = app.state::<AppState>();
    let (user_dir, game_dir) = (st.user_dir.clone(), st.game_dir.clone());
    if get_session(app).is_none() {
        return Err("Iniciá sesión con Microsoft o elegí Jugar sin cuenta.".to_string());
    }

    let profile = profiles::get(&user_dir, &game_dir, profile_id)?;
    let mc = profile["mc"].as_str().ok_or("El perfil no tiene versión de Minecraft.")?.to_string();
    if !net::is_allowed(&mc) {
        return Err(format!("La versión {mc} no está disponible en Trident Client"));
    }
    let settings = settings::write(&user_dir, &json!({ "lastProfile": profile_id, "lastVersion": mc }))?;
    let inst = PathBuf::from(profile["dir"].as_str().ok_or("El perfil no tiene carpeta.")?);
    std::fs::create_dir_all(&inst).map_err(|e| e.to_string())?;

    status(app, "preparing", "Preparando…", None);
    let progress_prep = |t: String, p: Option<f64>| status(app, "preparing", &t, p);
    let progress_dl = |t: String, p: Option<f64>| status(app, "downloading", &t, p);

    let version_type = launcher::ensure_vanilla_json(&game_dir, &mc).await?;
    let major = launcher::required_java_major(&game_dir, &mc).await?;
    let java_setting = settings.get("javaPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let java_path = java::ensure_java(&game_dir, &java_setting, major, &progress_prep).await?;

    let stale = SESSION_FRESH.lock().unwrap_or_else(|e| e.into_inner()).is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(12 * 3600));
    if stale {
        if let Some(t) = get_session(app).and_then(|s| s.refresh_token) {
            if let Ok(fresh) = account::refresh(&t).await {
                establish(app, fresh);
            }
        }
    }
    let session = get_session(app).ok_or("La sesión se cerró mientras se preparaba el juego.")?;
    crate::social::game_link(&user_dir, &inst, &session);
    let menu_dir = inst.join("config").join("tridentclient");
    let menu = if session.offline {
        tokio::time::timeout(std::time::Duration::from_secs(5), crate::skins::menu_skin(&user_dir, &session.name)).await.ok().flatten()
    } else {
        None
    };
    crate::skins::write_menu_skin_files(&menu_dir, menu.as_ref());

    let (launch_id, with_nova) = prepare_profile(app, &profile, &inst, &game_dir).await?;

    status(app, "downloading", "Verificando archivos del juego…", None);
    let prep = launcher::prepare(&game_dir, &launch_id, &WINDOWS, &progress_dl).await?;

    log(app, format!("[Autenticación] Modo: {}", if session.offline { "OFFLINE (sin cuenta Microsoft)" } else { "MICROSOFT" }));
    log(app, format!("[Autenticación] Perfil: {}", session.name));
    log(app, format!("[Autenticación] UUID: {}", session.uuid_plain()));
    log(app, format!("[Autenticación] Tipo: {}", session.user_type));

    let ram = profile["ramGB"]
        .as_f64()
        .filter(|n| *n > 0.0)
        .or_else(|| settings.get("ramGB").and_then(|v| v.as_f64()))
        .unwrap_or(4.0);
    let mut user_jvm = split_args(settings.get("jvmArgs").and_then(|v| v.as_str()).unwrap_or(""));
    user_jvm.extend(split_args(profile["jvmArgs"].as_str().unwrap_or("")));

    if java_setting.trim().is_empty() && major >= 21 && !user_jvm.iter().any(|a| a.contains("SharedArchiveFile")) {
        let cds_dir = game_dir.join("cache").join("cds");
        if std::fs::create_dir_all(&cds_dir).is_ok() {
            user_jvm.insert(0, format!("-XX:SharedArchiveFile={}", cds_dir.join(format!("{launch_id}.jsa")).display()));
            user_jvm.insert(0, "-XX:+AutoCreateSharedArchive".to_string());
        }
    }
    let net_url = nova::net_url(&settings);
    let nova_net = (with_nova && net_url.starts_with("http") && !net_url.contains(char::is_whitespace)).then_some(net_url.as_str());
    let server = opts["server"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty());

    let plan = build_command(
        &prep, &game_dir, &inst, &launch_id, &version_type, &session, &settings, ram, &user_jvm,
        &app.package_info().version.to_string(), server, &mc, nova_net,
    );

    default_options(&inst);
    let mut cmd = tokio::process::Command::new(&java_path);
    cmd.args(&plan)
        .current_dir(&inst)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000 | 0x0000_8000);
    let mut child = cmd.spawn().map_err(|e| {
        format!("No se pudo iniciar el juego ({}): {e}. Revisá el registro para más detalles.", java_path.display())
    })?;

    if let Some(out) = child.stdout.take() {
        pump(app.clone(), out);
    }
    if let Some(err) = child.stderr.take() {
        pump(app.clone(), err);
    }

    let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();
    if let Ok(mut g) = st.rt.game.lock() {
        if let Some(info) = g.as_mut() {
            info.kill = Some(kill_tx);
        }
    }
    status(app, "running", "Minecraft en ejecución", None);
    show_presence(app, &settings, &mc, with_nova);

    let started = std::time::Instant::now();
    let (app2, pid) = (app.clone(), profile_id.to_string());
    tokio::spawn(async move {
        let mut stopped = false;
        let code = tokio::select! {
            r = child.wait() => r.ok().and_then(|s| s.code()),
            _ = kill_rx => {
                stopped = true;
                let _ = child.kill().await;
                child.wait().await.ok().and_then(|s| s.code())
            }
        };
        on_exit(&app2, &pid, started.elapsed().as_secs_f64(), code, stopped);
    });

    if settings.get("closeOnLaunch").and_then(|v| v.as_bool()).unwrap_or(false) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    }
    Ok(())
}

fn show_presence(app: &AppHandle, settings: &serde_json::Map<String, Value>, mc: &str, with_nova: bool) {
    if !settings.get("discord").and_then(|v| v.as_bool()).unwrap_or(true) {
        return;
    }
    let client_id = discord::client_id(settings);
    if !discord::is_valid_id(&client_id) {
        log(app, "[Discord] Falta el ID de aplicación de Discord (ver README → Discord). No se muestra la actividad.");
        return;
    }
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut activity = json!({
        "details": format!("Minecraft {mc}"),
        "state": if with_nova { "Jugando con Trident Client" } else { "Jugando Minecraft" },
        "timestamps": { "start": started },
        "instance": false
    });
    let image = discord::large_image(settings);
    if !image.is_empty() {
        activity["assets"] = json!({ "large_image": image, "large_text": "Trident Client" });
    }
    let app2 = app.clone();
    let logger: discord::Log = Arc::new(move |m: String| log(&app2, m));
    hide_presence(app);
    let presence = discord::Presence::start(client_id, activity, logger);
    if let Ok(mut g) = app.state::<AppState>().rt.presence.lock() {
        *g = Some(presence);
    }
}

pub fn hide_presence(app: &AppHandle) {
    let old = app.state::<AppState>().rt.presence.lock().ok().and_then(|mut g| g.take());
    if let Some(p) = old {
        p.stop();
    }
}

fn pump<R: AsyncRead + Unpin + Send + 'static>(app: AppHandle, stream: R) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\r', '\n']);
                    if !line.is_empty() {
                        log(&app, line.to_string());
                    }
                }
            }
        }
    });
}

fn on_exit(app: &AppHandle, profile_id: &str, seconds: f64, code: Option<i32>, stopped: bool) {
    let st = app.state::<AppState>();
    if let Ok(mut g) = st.rt.game.lock() {
        *g = None;
    }
    if let Ok(mut a) = st.rt.active_profile.lock() {
        *a = None;
    }
    hide_presence(app);
    profiles::touch_played(&st.user_dir, profile_id, seconds);
    let text = match code {
        Some(0) => "Juego cerrado.".to_string(),
        _ if stopped => "Juego cerrado.".to_string(),
        Some(c) => format!("El juego se cerró (código {c})."),
        None => "El juego se cerró.".to_string(),
    };
    status(app, "idle", &text, None);
    crate::updater::on_game_exit(app);
    if let Some(w) = app.get_webview_window("main") {
        if !w.is_visible().unwrap_or(true) {
            let _ = w.show();
        }
    }
}

pub fn stop(app: &AppHandle) {
    let tx = app
        .state::<AppState>()
        .rt
        .game
        .lock()
        .ok()
        .and_then(|mut g| g.as_mut().and_then(|i| i.kill.take()));
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_flags_respetan_el_recolector_del_jugador() {
        assert!(perf_jvm_flags(&[]).contains(&"-XX:+UseG1GC".to_string()));
        assert!(perf_jvm_flags(&["-XX:+UseZGC".to_string()]).is_empty());
        assert!(!perf_jvm_flags(&["-Dfoo=1".to_string()]).is_empty());
    }

    #[test]
    fn default_options_no_pisa_lo_del_jugador() {
        let dir = std::env::temp_dir().join(format!("trident-opts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        default_options(&dir);
        assert!(std::fs::read_to_string(dir.join("options.txt")).unwrap().contains("simulationDistance:8"));
        std::fs::write(dir.join("options.txt"), "renderDistance:32\n").unwrap();
        default_options(&dir);
        assert_eq!(std::fs::read_to_string(dir.join("options.txt")).unwrap(), "renderDistance:32\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_play_por_version() {
        assert!(!supports_quick_play("1.19.2"));
        assert!(!supports_quick_play("1.19.4"));
        assert!(supports_quick_play("1.20.1"));
        assert!(supports_quick_play("1.21.11"));
        assert!(supports_quick_play("26.1"));
    }

    #[test]
    fn memoria_en_mb() {
        assert_eq!(memory_args(4.0), ["-Xms2048M".to_string(), "-Xmx4096M".to_string()]);
        assert_eq!(memory_args(1.0), ["-Xms1024M".to_string(), "-Xmx1024M".to_string()]);
        assert_eq!(memory_args(4.5)[1], "-Xmx4608M");
        assert_eq!(memory_args(0.1)[1], "-Xmx512M");
    }

    use std::collections::HashMap as Map;
    use std::io::{Read as _, Write as _};

    fn serve(files: Map<String, Vec<u8>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = match stream { Ok(s) => s, Err(_) => continue };
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                match files.get(&path) {
                    Some(body) => {
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                        let _ = s.write_all(body);
                    }
                    None => { let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); }
                }
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn sha1(b: &[u8]) -> String { use sha1::{Digest, Sha1}; hex::encode(Sha1::digest(b)) }

    #[tokio::test]
    async fn descarga_prepara_y_ejecuta_una_jvm_real() {
        std::env::set_var("NO_PROXY", "127.0.0.1");
        std::env::set_var("no_proxy", "127.0.0.1");
        let work = std::env::temp_dir().join(format!("nova-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();

        std::fs::write(work.join("Main.java"), r#"
            public class Main { public static void main(String[] a) {
                System.out.println("ARGS=" + String.join("|", a));
                System.out.println("LIBPATH=" + System.getProperty("java.library.path"));
                System.out.println("CP=" + System.getProperty("java.class.path"));
                System.out.println("MARK=" + System.getProperty("nova.mark"));
            } }"#).unwrap();
        assert!(std::process::Command::new("javac").arg("-d").arg(&work).arg(work.join("Main.java")).status().unwrap().success());
        assert!(std::process::Command::new("jar").current_dir(&work).args(["cf", "client.jar", "Main.class"]).status().unwrap().success());
        let client = std::fs::read(work.join("client.jar")).unwrap();

        let lib = b"PK-lib-falsa".to_vec();
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("lwjgl64.dll", o).unwrap(); z.write_all(b"dll").unwrap();
        z.start_file("META-INF/MANIFEST.MF", o).unwrap(); z.write_all(b"m").unwrap();
        let natives = z.finish().unwrap().into_inner();

        let idx = br#"{"objects":{}}"#.to_vec();
        let mut files = Map::new();
        files.insert("/client.jar".to_string(), client.clone());
        files.insert("/lib.jar".to_string(), lib.clone());
        files.insert("/nat.jar".to_string(), natives.clone());
        files.insert("/idx.json".to_string(), idx.clone());
        let base = serve(files);

        let root = work.join("root");
        let vdir = root.join("versions").join("test-1");
        std::fs::create_dir_all(&vdir).unwrap();
        let version = json!({
            "id": "test-1", "type": "release", "mainClass": "Main", "assets": "idx",
            "assetIndex": { "id": "idx", "url": format!("{base}/idx.json"), "sha1": sha1(&idx) },
            "downloads": { "client": { "url": format!("{base}/client.jar"), "sha1": sha1(&client), "size": client.len() } },
            "libraries": [
                { "name": "x:lib:1", "downloads": { "artifact": { "path": "x/lib/1/lib-1.jar", "url": format!("{base}/lib.jar"), "sha1": sha1(&lib), "size": lib.len() } } },
                { "name": "x:solo-osx:1", "rules": [{ "action": "allow", "os": { "name": "osx" } }],
                  "downloads": { "artifact": { "path": "x/osx/1/osx-1.jar", "url": format!("{base}/no-existe.jar"), "sha1": "0", "size": 1 } } },
                { "name": "x:nat:1", "natives": { "windows": "natives-windows" }, "extract": { "exclude": ["META-INF/"] },
                  "downloads": { "classifiers": { "natives-windows": { "path": "x/nat/1/nat-1-natives-windows.jar", "url": format!("{base}/nat.jar"), "sha1": sha1(&natives), "size": natives.len() } } } }
            ],
            "arguments": {
                "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"],
                "game": ["--username", "${auth_player_name}", "--uuid", "${auth_uuid}", "--userType", "${user_type}", "--gameDir", "${game_directory}",
                         { "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }], "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"] }]
            }
        });
        std::fs::write(vdir.join("test-1.json"), serde_json::to_string(&version).unwrap()).unwrap();

        let steps = std::sync::Mutex::new(Vec::<String>::new());
        let progress = |t: String, _p: Option<f64>| steps.lock().unwrap().push(t);
        let prep = launcher::prepare(&root, "test-1", &WINDOWS, &progress).await.expect("prepare");

        assert!(root.join("libraries/x/lib/1/lib-1.jar").exists());
        assert!(!root.join("libraries/x/osx/1/osx-1.jar").exists(), "no debe bajar librerías de otro sistema");
        assert!(prep.natives_dir.join("lwjgl64.dll").exists(), "nativos extraídos");
        assert!(!prep.natives_dir.join("MANIFEST.MF").exists(), "META-INF excluido");
        assert!(root.join("versions/test-1/test-1.jar").exists());
        assert!(root.join("assets/indexes/idx.json").exists());
        assert_eq!(prep.classpath.len(), 2, "lib normal + jar del cliente (los nativos no van al classpath)");
        assert!(steps.lock().unwrap().iter().any(|s| s.contains("librerías")));

        let again = launcher::prepare(&root, "test-1", &WINDOWS, &progress).await.expect("prepare 2");
        assert_eq!(again.classpath, prep.classpath);

        let session = account::offline_session("Tester_1").unwrap();
        let mut settings = serde_json::Map::new();
        settings.insert("winWidth".into(), json!(1000));
        settings.insert("winHeight".into(), json!(600));
        let inst = work.join("inst");
        std::fs::create_dir_all(&inst).unwrap();
        let args = build_command(&prep, &root, &inst, "test-1", "release", &session, &settings, 1.0,
            &["-Dnova.mark=hola".to_string()], "1.1.0", None, "1.21.4", None);
        assert_eq!(args[0], "-Xms1024M");
        assert_eq!(args[1], "-Xmx1024M");

        let out = std::process::Command::new("java").args(&args).current_dir(&inst).output().expect("ejecutar java");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(out.status.success(), "java falló: {}\n{}", stdout, String::from_utf8_lossy(&out.stderr));
        let uuid = account::offline_uuid("Tester_1").replace('-', "");
        assert!(stdout.contains(&format!("ARGS=--username|Tester_1|--uuid|{uuid}|--userType|mojang|--gameDir|{}|--width|1000|--height|600", inst.display())), "{stdout}");
        assert!(stdout.contains("MARK=hola"), "{stdout}");
        assert!(stdout.contains(&format!("LIBPATH={}", prep.natives_dir.display())), "{stdout}");
        let _ = std::fs::remove_dir_all(&work);
    }
}

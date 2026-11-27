#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
mod account;
mod bedrock;
mod content;
mod discord;
mod dl;
mod fabric;
mod game;
mod java;
mod launcher;
mod modcompat;
mod net;
mod nova;
mod profiles;
mod settings;
mod skins;
mod social;
mod system;
mod updater;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

type Res = Result<Value, String>;

pub struct AppState {
    pub user_dir: PathBuf,
    pub game_dir: PathBuf,
    pub rt: game::Runtime,
    pub updates: updater::LiveUpdate,
}

static MAXIMIZED: AtomicBool = AtomicBool::new(false);

fn migrate_dir(new: PathBuf, old: PathBuf) -> (PathBuf, bool) {
    if new.exists() || !old.exists() {
        return (new, false);
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => (new, true),
        Err(e) => {
            eprintln!("No se pudo renombrar {} → {}: {e}", old.display(), new.display());
            (old, false)
        }
    }
}

fn data_dirs(roaming: &Path) -> (PathBuf, PathBuf) {
    let (user_dir, _) = migrate_dir(roaming.join("Trident Client"), roaming.join("Nova Client"));
    let old_game = roaming.join(".novalauncher");
    let (game_dir, moved) = migrate_dir(roaming.join(".tridentclient"), old_game.clone());

    if moved {
        let s = settings::read(&user_dir);
        if let Some(rest) = s.get("javaPath").and_then(|v| v.as_str()).and_then(|p| p.strip_prefix(&*old_game.to_string_lossy())) {
            let fixed = format!("{}{}", game_dir.to_string_lossy(), rest);
            let _ = settings::write(&user_dir, &serde_json::json!({ "javaPath": fixed }));
        }
    }
    (user_dir, game_dir)
}

fn open_folder(app: &tauri::AppHandle, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

fn nova_versions(user_dir: &Path) -> Vec<String> {
    std::fs::read_dir(user_dir.join("mod-cache"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.strip_prefix("nova-client-").and_then(|r| r.strip_suffix(".jar")).map(|v| v.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn app_info(app: tauri::AppHandle, state: State<'_, AppState>) -> Res {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total_gb = sys.total_memory() / 1024 / 1024 / 1024;
    let nova_versions = nova_versions(&state.user_dir);
    let discord_ok = discord::is_valid_id(&discord::client_id(&settings::read(&state.user_dir)));
    Ok(json!({
        "version": app.package_info().version.to_string(),
        "gameDir": state.game_dir.to_string_lossy(),
        "dataDir": state.user_dir.to_string_lossy(),
        "totalRamGB": total_gb,
        "platform": if cfg!(target_os = "macos") { "darwin" } else if cfg!(target_os = "linux") { "linux" } else { "win32" },
        "engine": format!("Tauri {}", tauri::VERSION),
        "novaVersions": nova_versions,
        "novaJar": !nova_versions.is_empty(),
        "discordConfigured": discord_ok,
        "canAutoUpdate": updater::supported(),
        "running": game::is_running(&app).map(|id| json!({ "profileId": id }))
    }))
}

#[tauri::command]
fn settings_get(state: State<'_, AppState>) -> Res {
    Ok(Value::Object(settings::read(&state.user_dir)))
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, state: State<'_, AppState>, patch: Value) -> Res {
    let next = settings::write(&state.user_dir, &patch)?;

    if patch.get("discord").and_then(|v| v.as_bool()) == Some(false) {
        game::hide_presence(&app);
    }
    if patch.get("uiScale").is_some() {
        system::apply_zoom(&app, &next);
    }
    if patch.get("openAtLogin").is_some() || patch.get("startMode").is_some() {
        system::apply_login_item(&next);
    }
    Ok(Value::Object(next))
}

#[tauri::command]
fn report_error(state: State<'_, AppState>, kind: String, detail: String) {
    let kind: String = kind.chars().take(40).collect();
    let detail: String = detail.chars().take(8000).collect();
    system::report_error(&state.user_dir, &kind, &detail);
}

#[tauri::command]
async fn versions_list(app: tauri::AppHandle) -> Res {
    net::list_versions(&app.package_info().version.to_string()).await
}

#[tauri::command]
async fn news_list(app: tauri::AppHandle) -> Res {
    net::news(&app.package_info().version.to_string()).await
}

#[tauri::command]
async fn version_art(app: tauri::AppHandle) -> Res {
    art::version_art(&app).await
}

fn bedrock_roots(app: &tauri::AppHandle) -> Vec<bedrock::Root> {
    match (app.path().data_dir(), app.path().local_data_dir()) {
        (Ok(roaming), Ok(local)) => bedrock::roots(&roaming, &local),
        _ => Vec::new(),
    }
}

#[tauri::command]
async fn bedrock_status(app: tauri::AppHandle) -> Res {
    let roots = bedrock_roots(&app);
    blocking(move || {
        let installed = bedrock::protocol_registered() || !roots.is_empty();
        Ok(json!({ "installed": installed, "canImport": bedrock::import_target(&roots).is_some(), "worlds": bedrock::worlds(&roots) }))
    })
    .await
}

#[tauri::command]
fn bedrock_play(app: tauri::AppHandle, state: State<'_, AppState>, id: Option<String>) -> Res {
    if !bedrock::protocol_registered() {
        return Err("Minecraft Bedrock no está instalado. Descargalo con el botón «Descargar Bedrock».".to_string());
    }
    app.opener()
        .open_url("minecraft://", None::<&str>)
        .map_err(|e| format!("No se pudo abrir Minecraft Bedrock: {e}"))?;
    if let Some(id) = id.filter(|s| !s.is_empty()) {
        profiles::touch_played(&state.user_dir, &id, 0.0);
    }
    Ok(Value::Null)
}

#[tauri::command]
fn bedrock_install(app: tauri::AppHandle) -> Res {
    app.opener()
        .open_url(bedrock::STORE_URL, None::<&str>)
        .map_err(|e| format!("No se pudo abrir la Microsoft Store: {e}"))?;
    Ok(Value::Null)
}

#[tauri::command]
async fn bedrock_backup(app: tauri::AppHandle, path: String) -> Res {
    let world = bedrock::world_dir(&bedrock_roots(&app), &path)?;
    let stamp = {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (y, m, d) = civil_from_days((secs / 86_400) as i64);
        format!("{y:04}-{m:02}-{d:02}")
    };
    let file_name = format!("{} {stamp}.mcworld", bedrock::safe_file_name(&bedrock::display_name(&world)));
    let downloads = app.path().download_dir().ok();
    let a = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = a.dialog().file().set_title("Guardar copia del mundo").set_file_name(file_name).add_filter("Mundo de Minecraft Bedrock", &["mcworld"]);
        if let Some(d) = downloads {
            dlg = dlg.set_directory(d);
        }
        dlg.blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(dest) = file.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };
    let out = dest.clone();
    blocking(move || bedrock::backup(&world, &out)).await?;
    let _ = app.opener().reveal_item_in_dir(&dest);
    Ok(json!(dest.to_string_lossy()))
}

#[tauri::command]
async fn bedrock_import(app: tauri::AppHandle) -> Res {
    let roots = bedrock_roots(&app);
    let Some(target) = bedrock::import_target(&roots).map(|r| r.dir.clone()) else {
        return Err("Abrí Minecraft Bedrock una vez para que cree su carpeta de mundos y volvé a intentar.".to_string());
    };
    let a = app.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        a.dialog().file().set_title("Importar mundos de Bedrock").add_filter("Mundo de Minecraft Bedrock", &["mcworld"]).blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;
    let paths = picked(files.unwrap_or_default());
    if paths.is_empty() {
        return Ok(Value::Null);
    }
    blocking(move || {
        let mut names = Vec::new();
        for p in paths {
            names.push(bedrock::import(&p, &target)?);
        }
        Ok(json!(names))
    })
    .await
}

#[tauri::command]
fn bedrock_open(app: tauri::AppHandle, path: String) -> Res {
    let world = bedrock::world_dir(&bedrock_roots(&app), &path)?;
    open_folder(&app, &world)?;
    Ok(Value::Null)
}

#[tauri::command]
fn shell_external(app: tauri::AppHandle, url: String) -> Res {
    if !url.starts_with("https://") {
        return Err("Enlace no permitido.".to_string());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

#[tauri::command]
fn folder_open(app: tauri::AppHandle, state: State<'_, AppState>) -> Res {
    open_folder(&app, &state.game_dir)?;
    Ok(Value::Null)
}

#[tauri::command]
fn data_open(app: tauri::AppHandle, state: State<'_, AppState>) -> Res {
    open_folder(&app, &state.user_dir)?;
    Ok(Value::Null)
}

#[tauri::command]
fn profiles_list(state: State<'_, AppState>) -> Res {
    Ok(profiles::list(&state.user_dir, &state.game_dir))
}

#[tauri::command]
fn profiles_create(state: State<'_, AppState>, input: Value) -> Res {
    profiles::create(&state.user_dir, &state.game_dir, &input)
}

#[tauri::command]
fn profiles_update(state: State<'_, AppState>, id: String, patch: Value) -> Res {
    profiles::update(&state.user_dir, &state.game_dir, &id, &patch)
}

#[tauri::command]
fn profiles_delete(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Res {
    not_running(&app, &id)?;
    profiles::remove(&state.user_dir, &state.game_dir, &id)?;
    Ok(Value::Null)
}

#[tauri::command]
fn profiles_duplicate(state: State<'_, AppState>, id: String) -> Res {
    profiles::duplicate(&state.user_dir, &state.game_dir, &id)
}

#[tauri::command]
fn profiles_folders(state: State<'_, AppState>, action: String, name: String) -> Res {
    profiles::set_folders(&state.user_dir, &action, &name)
}

#[tauri::command]
fn profiles_open_folder(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Res {
    let dir = profiles::dir_of(&state.user_dir, &state.game_dir, &id)?;
    open_folder(&app, &dir)?;
    Ok(Value::Null)
}

fn not_running(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    if game::is_running(app).as_deref() == Some(id) {
        return Err("Cerrá el juego antes de hacer eso con este perfil.".to_string());
    }
    Ok(())
}

fn env(app: &tauri::AppHandle) -> content::Env {
    let st = app.state::<AppState>();
    let (a, b) = (app.clone(), app.clone());
    content::Env {
        user_dir: st.user_dir.clone(),
        game_dir: st.game_dir.clone(),
        log: Arc::new(move |m| game::log(&a, m)),
        progress: Arc::new(move |id, text| {
            let _ = b.emit("profile:progress", json!({ "id": id, "text": text }));
        }),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn picked(paths: Vec<tauri_plugin_dialog::FilePath>) -> Vec<PathBuf> {
    paths.into_iter().filter_map(|p| p.into_path().ok()).collect()
}

#[tauri::command]
async fn profiles_copy_from(app: tauri::AppHandle, id: String, source_id: String) -> Res {
    not_running(&app, &id)?;
    let e = env(&app);
    blocking(move || content::copy_from(&e, &id, &source_id)).await
}

#[tauri::command]
async fn profiles_export(app: tauri::AppHandle, id: String) -> Res {
    let e = env(&app);
    let name = profiles::get(&e.user_dir, &e.game_dir, &id)?["name"].as_str().unwrap_or("perfil").to_string();
    let a = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        a.dialog()
            .file()
            .set_title("Exportar perfil")
            .set_file_name(format!("{}.novaprofile", content::safe_name(&name)))
            .add_filter("Perfil de Trident Client", &["novaprofile"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = file.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };
    content::export_to_file(&e, &id, &path)?;
    Ok(json!(path.to_string_lossy()))
}

#[tauri::command]
fn profiles_share(app: tauri::AppHandle, id: String) -> Res {
    Ok(json!(content::share_code(&env(&app), &id)?))
}

#[tauri::command]
async fn profiles_import_file(app: tauri::AppHandle) -> Res {
    let a = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        a.dialog()
            .file()
            .set_title("Importar perfil")
            .add_filter("Perfil de Trident Client", &["novaprofile", "json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = file.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("No se pudo leer el archivo: {e}"))?;
    content::import_data(&env(&app), &content::parse_share(&text)?).await
}

#[tauri::command]
async fn profiles_import_code(app: tauri::AppHandle, code: String) -> Res {
    content::import_data(&env(&app), &content::parse_share(&code)?).await
}

#[tauri::command]
async fn content_list(app: tauri::AppHandle, id: String) -> Res {
    let e = env(&app);
    blocking(move || content::list_content(&e, &id)).await
}

#[tauri::command]
async fn content_toggle(app: tauri::AppHandle, id: String, file: String, enabled: bool) -> Res {
    not_running(&app, &id)?;
    let e = env(&app);
    blocking(move || content::toggle_content(&e, &id, &file, enabled)).await?;
    Ok(Value::Null)
}

#[tauri::command]
async fn content_remove(app: tauri::AppHandle, id: String, file: String) -> Res {
    not_running(&app, &id)?;
    let e = env(&app);
    blocking(move || content::remove_content(&e, &id, &file)).await?;
    Ok(Value::Null)
}

#[tauri::command]
async fn content_search(app: tauri::AppHandle, id: String, q: Value) -> Res {
    content::search(&env(&app), &id, &q).await
}

#[tauri::command]
async fn content_browse(app: tauri::AppHandle, q: Value) -> Res {
    content::browse(&env(&app), &q).await
}

#[tauri::command]
async fn content_install(app: tauri::AppHandle, id: String, project_id: String, kind: Option<String>) -> Res {
    not_running(&app, &id)?;
    let titles = content::install_project(&env(&app), &id, &project_id, kind.as_deref().unwrap_or("mod"), json!({}), 0).await?;
    Ok(json!(titles))
}

#[tauri::command]
async fn content_update(app: tauri::AppHandle, id: String, file: String) -> Res {
    not_running(&app, &id)?;
    content::update_content(&env(&app), &id, &file).await
}

#[tauri::command]
async fn content_identify(app: tauri::AppHandle, id: String) -> Res {
    Ok(json!(content::identify_local(&env(&app), &id).await?))
}

#[tauri::command]
async fn content_check_updates(app: tauri::AppHandle, id: String) -> Res {
    let keys: Vec<String> = content::check_updates(&env(&app), &id).await?.into_iter().map(|(k, _)| k).collect();
    Ok(json!(keys))
}

#[tauri::command]
async fn content_pick_local(app: tauri::AppHandle, id: String) -> Res {
    not_running(&app, &id)?;
    let a = app.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        a.dialog()
            .file()
            .set_title("Añadir contenido")
            .add_filter("Mods y paquetes de recursos", &["jar", "zip"])
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;
    let list: Vec<String> = picked(files.unwrap_or_default()).iter().map(|p| p.to_string_lossy().to_string()).collect();
    Ok(json!(list))
}

#[tauri::command]
async fn content_add_local(app: tauri::AppHandle, id: String, paths: Vec<String>) -> Res {
    not_running(&app, &id)?;
    let e = env(&app);
    blocking(move || content::add_local(&e, &id, &paths)).await
}

#[tauri::command]
fn perf_get(app: tauri::AppHandle, id: String) -> Res {
    content::get_perf(&env(&app), &id)
}

#[tauri::command]
async fn worlds_list(app: tauri::AppHandle, id: String) -> Res {
    let e = env(&app);
    blocking(move || content::list_worlds(&e, &id)).await
}

#[tauri::command]
fn worlds_open(app: tauri::AppHandle, id: String, name: String) -> Res {
    let dir = content::world_dir(&env(&app), &id, &name)?;
    if !dir.is_dir() {
        return Err("Ese mundo ya no existe.".to_string());
    }
    open_folder(&app, &dir)?;
    Ok(Value::Null)
}

#[tauri::command]
async fn logs_read(app: tauri::AppHandle, id: String) -> Res {
    let e = env(&app);
    Ok(json!(blocking(move || content::read_log(&e, &id)).await?))
}

#[tauri::command]
async fn skin_get(state: State<'_, AppState>, query: String, force: Option<bool>) -> Res {
    skins::get_skin(&state.user_dir.join("skin-cache"), &query, force.unwrap_or(false), None).await
}

#[tauri::command]
async fn skin_mine(app: tauri::AppHandle, force: Option<bool>) -> Res {
    let session = game::get_session(&app).ok_or("No hay sesión iniciada.")?;
    let dir = app.state::<AppState>().user_dir.join("skin-cache");
    let force = force.unwrap_or(false);
    if session.offline {
        if let Some(v) = skins::offline_skin(&app.state::<AppState>().user_dir, &session.name) {
            return Ok(v);
        }
        return skins::get_skin(&dir, &session.name, force, None).await;
    }
    let list = session.skins.as_array().cloned().unwrap_or_default();
    let active = list.iter().find(|s| s["state"].as_str() == Some("ACTIVE")).or_else(|| list.first());
    let known = skins::Known {
        name: session.name.clone(),
        skin_url: active.and_then(|s| s["url"].as_str()).map(|s| s.to_string()),
        slim: active.is_some_and(|s| s["variant"].as_str().is_some_and(|v| v.eq_ignore_ascii_case("slim"))),
    };
    skins::get_skin(&dir, &session.id, force, Some(&known)).await
}

#[tauri::command]
fn skin_offline_set(app: tauri::AppHandle, data: String, model: String) -> Res {
    let session = game::get_session(&app).ok_or("No hay sesión iniciada.")?;
    if !session.offline {
        return Err("Tu cuenta es premium: la skin se cambia en minecraft.net.".into());
    }
    skins::set_offline_skin(&app.state::<AppState>().user_dir, &session.name, &data, &model)
}

#[tauri::command]
fn skin_offline_clear(app: tauri::AppHandle) -> Res {
    let session = game::get_session(&app).ok_or("No hay sesión iniciada.")?;
    skins::clear_offline_skin(&app.state::<AppState>().user_dir, &session.name);
    Ok(Value::Null)
}

#[tauri::command]
async fn debug_export(app: tauri::AppHandle) -> Res {
    use std::io::Write;
    let st = app.state::<AppState>();
    let mut settings = settings::read(&st.user_dir);
    settings.remove("discordClientId");
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let versions = nova_versions(&st.user_dir);
    let info = [
        format!("Trident Client {}", app.package_info().version),
        "Tauri 2".to_string(),
        format!(
            "{} · {} GB de RAM",
            sysinfo::System::long_os_version().unwrap_or_else(|| "sistema desconocido".to_string()),
            sys.total_memory() / 1024 / 1024 / 1024
        ),
        format!("Carpeta del juego: {}", st.game_dir.to_string_lossy()),
        format!("Mods de Trident compilados: {}", if versions.is_empty() { "ninguno".to_string() } else { versions.join(", ") }),
    ]
    .join("\n");
    let log_text = st.rt.log_buf.lock().map(|b| b.join("\n")).unwrap_or_default();
    let profiles_text = std::fs::read_to_string(st.user_dir.join("profiles.json")).ok();
    let e = env(&app);
    let ids = profiles::ids(&st.user_dir);
    let settings_text = serde_json::to_string_pretty(&Value::Object(settings)).map_err(|e| e.to_string())?;

    let bytes = blocking(move || {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opt = zip::write::SimpleFileOptions::default();
        let mut add = |name: &str, body: &[u8]| -> Result<(), String> {
            z.start_file(name, opt).map_err(|e| e.to_string())?;
            z.write_all(body).map_err(|e| e.to_string())
        };
        add("info.txt", info.as_bytes())?;
        add("settings.json", settings_text.as_bytes())?;
        add("launcher-log.txt", log_text.as_bytes())?;
        if let Some(t) = &profiles_text {
            add("profiles.json", t.as_bytes())?;
        }
        for id in &ids {
            let log = content::read_log(&e, id).unwrap_or_default();
            if !log.is_empty() {
                add(&format!("perfiles/{id}-latest.log"), log.as_bytes())?;
            }
        }
        Ok(z.finish().map_err(|e| e.to_string())?.into_inner())
    })
    .await?;

    let desktop = app.path().desktop_dir().ok();
    let stamp = {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (y, m, d) = civil_from_days((secs / 86_400) as i64);
        format!("{y:04}-{m:02}-{d:02}")
    };
    let a = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = a.dialog().file().set_title("Exportar depuración").set_file_name(format!("nova-depuracion-{stamp}.zip")).add_filter("Zip", &["zip"]);
        if let Some(d) = desktop {
            dlg = dlg.set_directory(d);
        }
        dlg.blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = file.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };
    std::fs::write(&path, bytes).map_err(|e| format!("No se pudo guardar el archivo: {e}"))?;
    let _ = app.opener().reveal_item_in_dir(&path);
    Ok(json!(path.to_string_lossy()))
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasa_los_datos_de_nova_a_trident() {
        let root = std::env::temp_dir().join(format!("trident-migra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Nova Client")).unwrap();
        std::fs::create_dir_all(root.join(".novalauncher/runtime/jre-21/bin")).unwrap();
        std::fs::write(root.join("Nova Client/profiles.json"), "{}").unwrap();
        let java = root.join(".novalauncher/runtime/jre-21/bin/javaw.exe");
        settings::write(&root.join("Nova Client"), &serde_json::json!({ "javaPath": java.to_string_lossy() })).unwrap();

        let (u, g) = data_dirs(&root);
        assert_eq!(u, root.join("Trident Client"));
        assert_eq!(g, root.join(".tridentclient"));
        assert!(u.join("profiles.json").exists() && !root.join("Nova Client").exists());
        assert!(g.join("runtime/jre-21/bin").exists() && !root.join(".novalauncher").exists());
        let jp = settings::read(&u)["javaPath"].as_str().unwrap().to_string();
        assert!(jp.starts_with(&*g.to_string_lossy()), "{jp}");

        assert_eq!(data_dirs(&root), (root.join("Trident Client"), root.join(".tridentclient")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fechas_civiles() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_500), (2026, 2, 16));
    }
}

#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Res {
    Ok(updater::check(&app, true).await)
}

#[tauri::command]
fn update_hold(app: tauri::AppHandle) -> Res {
    updater::hold(&app);
    Ok(Value::Null)
}

#[tauri::command]
fn update_start(app: tauri::AppHandle) -> Res {
    updater::start(&app)?;
    Ok(Value::Null)
}

#[tauri::command]
fn danger_clear_cache(state: State<'_, AppState>) -> Res {
    let _ = std::fs::remove_dir_all(state.user_dir.join("mod-cache"));
    Ok(Value::Null)
}

#[tauri::command]
fn danger_reset_settings(state: State<'_, AppState>) -> Res {
    Ok(Value::Object(settings::reset(&state.user_dir)))
}

#[tauri::command]
fn online_get() -> Res {
    Ok(json!(social::online_count()))
}

#[tauri::command]
async fn tray_menu_action(app: tauri::AppHandle, action: String) -> Res {
    let handle = app.clone();
    app.run_on_main_thread(move || system::tray_action(&handle, &action)).map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

#[tauri::command]
async fn auth_restore(app: tauri::AppHandle) -> Res {
    game::restore(&app).await
}

#[tauri::command]
async fn auth_login(app: tauri::AppHandle) -> Res {
    game::login(&app).await
}

#[tauri::command]
fn auth_offline(app: tauri::AppHandle, name: String) -> Res {
    game::login_offline(&app, &name)
}

#[tauri::command]
async fn social_sync(app: tauri::AppHandle, after: Option<f64>) -> Res {
    social::sync(&app, after.unwrap_or(0.0)).await
}

#[tauri::command]
async fn social_action(app: tauri::AppHandle, action: String, name: String) -> Res {
    social::action(&app, &action, &name).await
}

#[tauri::command]
async fn shop_call(app: tauri::AppHandle, op: String, id: Option<String>, on: Option<bool>) -> Res {
    social::shop(&app, &op, id.as_deref().unwrap_or(""), on.unwrap_or(false)).await
}

#[tauri::command]
fn net_offline_status(app: tauri::AppHandle) -> Res {
    Ok(social::offline_status(&app))
}

#[tauri::command]
async fn net_offline_link(app: tauri::AppHandle, password: String, register: Option<bool>) -> Res {
    social::offline_link(&app, &password, register.unwrap_or(false)).await
}

#[tauri::command]
fn net_offline_unlink(app: tauri::AppHandle) -> Res {
    social::offline_unlink(&app);
    Ok(Value::Null)
}

#[tauri::command]
async fn social_send(app: tauri::AppHandle, to: String, text: String) -> Res {
    social::send(&app, &to, &text).await
}

#[tauri::command]
fn auth_logout(app: tauri::AppHandle) -> Res {
    game::logout(&app);
    Ok(Value::Null)
}

#[tauri::command]
async fn game_launch(app: tauri::AppHandle, profile_id: String, opts: Value) -> Res {
    game::launch(&app, &profile_id, &opts).await?;
    Ok(Value::Null)
}

#[tauri::command]
fn game_stop(app: tauri::AppHandle) -> Res {
    game::stop(&app);
    Ok(Value::Null)
}

#[tauri::command]
fn danger_clear_java(app: tauri::AppHandle, state: State<'_, AppState>) -> Res {
    if game::is_running(&app).is_some() {
        return Err("Cerrá Minecraft primero.".to_string());
    }
    let _ = std::fs::remove_dir_all(state.game_dir.join("runtime"));
    Ok(Value::Null)
}

#[tauri::command]
fn win_minimize(window: tauri::WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
fn win_maximize(window: tauri::WebviewWindow) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

#[tauri::command]
fn win_close(window: tauri::WebviewWindow) {
    if window.label() == "main" && system::hides_to_tray(window.app_handle()) {
        let _ = window.hide();
    } else {
        let _ = window.close();
    }
}

#[tauri::command]
fn win_drag(window: tauri::WebviewWindow) {
    let _ = window.start_dragging();
}

fn main() {
    dl::tls_init();
    tauri::Builder::default()

        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .register_asynchronous_uri_scheme_protocol(art::SCHEME, |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(art::serve(app, req).await);
            });
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let roaming = app.path().data_dir().map_err(|e| e.to_string())?;
            let (user_dir, game_dir) = data_dirs(&roaming);
            let state = AppState {
                user_dir,
                game_dir,
                rt: game::Runtime::default(),
                updates: updater::LiveUpdate::default(),
            };
            dl::set_checks_file(state.user_dir.join("checks.json"));
            let current = settings::read(&state.user_dir);
            system::install_panic_reporter(state.user_dir.clone());
            app.manage(state);

            let win_cfg = app.config().app.windows.first().cloned().ok_or("Falta la ventana en tauri.conf.json")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &win_cfg)?
                .additional_browser_args(system::session_browser_args(&current))
                .build()?;

            let handle = app.handle().clone();
            let args: Vec<String> = std::env::args().collect();
            let hidden = system::start_hidden(&args, &current);
            let custom_menu = cfg!(windows) && system::create_tray_menu(&handle, system::session_browser_args(&current));
            let tray_ok = match system::create_tray(&handle, custom_menu) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("No se pudo crear el ícono de la bandeja: {e}");
                    false
                }
            };

            if current.get("openAtLogin").and_then(|v| v.as_bool()).unwrap_or(false) {
                system::apply_login_item(&current);
            }
            system::apply_zoom(&handle, &current);

            if !(hidden && tray_ok) {
                system::show_main_window(&handle);
            }
            updater::schedule(&handle);
            social::start_online(handle.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == system::TRAY_MENU {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && system::hides_to_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
            }

            if let tauri::WindowEvent::Resized(_) = event {
                if let Ok(max) = window.is_maximized() {
                    if MAXIMIZED.swap(max, Ordering::Relaxed) != max {
                        let _ = window.emit("win:maximized", max);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            online_get,
            tray_menu_action,
            settings_get,
            settings_set,
            report_error,
            versions_list,
            news_list,
            version_art,
            shell_external,
            folder_open,
            data_open,
            profiles_list,
            profiles_create,
            profiles_update,
            profiles_delete,
            profiles_duplicate,
            profiles_folders,
            profiles_open_folder,
            profiles_copy_from,
            profiles_export,
            profiles_share,
            profiles_import_file,
            profiles_import_code,
            content_list,
            content_toggle,
            content_remove,
            content_search,
            content_browse,
            content_install,
            content_update,
            content_identify,
            content_check_updates,
            content_pick_local,
            content_add_local,
            perf_get,
            worlds_list,
            worlds_open,
            logs_read,
            skin_get,
            skin_mine,
            skin_offline_set,
            skin_offline_clear,
            debug_export,
            update_check,
            update_hold,
            update_start,
            danger_clear_cache,
            danger_reset_settings,
            danger_clear_java,
            auth_restore,
            auth_login,
            auth_offline,
            auth_logout,
            social_sync,
            bedrock_status,
            bedrock_play,
            bedrock_install,
            bedrock_backup,
            bedrock_import,
            bedrock_open,
            social_action,
            social_send,
            shop_call,
            net_offline_status,
            net_offline_link,
            net_offline_unlink,
            game_launch,
            game_stop,
            win_minimize,
            win_maximize,
            win_close,
            win_drag
        ])
        .build(tauri::generate_context!())
        .expect("No se pudo iniciar Trident Client")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                system::show_main_window(app);
            }
            let _ = (app, event);
        });
}

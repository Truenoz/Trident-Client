use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{game, nova, AppState};

const COUNTDOWN_SECS: u32 = 10;
const FIRST_CHECK: Duration = Duration::from_secs(3);
const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
const INSTALL_DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug, PartialEq)]
pub enum Step {
    Nothing,

    Emit(Value),

    Countdown(Value, u64),
    Download,
    Install,
}

#[derive(Default)]
pub struct Flow {
    version: Option<String>,
    downloading: bool,
    ready: bool,
    install_on_exit: bool,

    cancelled_for: Option<String>,

    countdown: Option<u64>,
    turn: u64,
    seconds: u32,
}

fn launcher(state: &str, extra: Value) -> Value {
    let mut v = json!({ "kind": "launcher", "state": state });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    v
}

impl Flow {
    fn pending(&self) -> bool {
        self.version.is_some() && !self.ready
    }

    pub fn busy(&self) -> bool {
        self.ready || self.downloading
    }

    fn available(&self) -> Value {
        launcher("available", json!({ "version": self.version }))
    }

    pub fn checking(&self) -> Step {
        if self.pending() || self.downloading {
            Step::Nothing
        } else {
            Step::Emit(launcher("checking", json!({})))
        }
    }

    pub fn nothing_new(&self) -> Step {
        if self.pending() || self.downloading {
            Step::Nothing
        } else {
            Step::Emit(launcher("none", json!({})))
        }
    }

    pub fn check_failed(&self, message: &str) -> Step {
        if self.pending() || self.downloading {
            Step::Nothing
        } else {
            Step::Emit(launcher("error", json!({ "message": message })))
        }
    }

    pub fn found(&mut self, version: &str, game_running: bool) -> Step {
        if self.ready || self.downloading || self.countdown.is_some() {
            return Step::Nothing;
        }
        self.version = Some(version.to_string());
        if game_running || self.cancelled_for.as_deref() == Some(version) {
            return Step::Emit(self.available());
        }
        self.turn += 1;
        self.countdown = Some(self.turn);
        self.seconds = COUNTDOWN_SECS;
        Step::Countdown(launcher("countdown", json!({ "version": self.version, "seconds": self.seconds })), self.turn)
    }

    pub fn tick(&mut self, turn: u64, game_running: bool) -> Step {
        if self.countdown != Some(turn) {
            return Step::Nothing;
        }
        if game_running {
            self.countdown = None;
            return Step::Emit(self.available());
        }
        self.seconds = self.seconds.saturating_sub(1);
        if self.seconds == 0 {
            self.countdown = None;
            return self.begin_download();
        }
        Step::Emit(launcher("countdown", json!({ "version": self.version, "seconds": self.seconds })))
    }

    fn begin_download(&mut self) -> Step {
        if self.downloading || self.ready {
            return Step::Nothing;
        }
        self.countdown = None;
        self.downloading = true;
        Step::Download
    }

    pub fn hold(&mut self) -> Step {
        if !self.pending() || self.downloading {
            return Step::Nothing;
        }
        self.countdown = None;
        self.cancelled_for = self.version.clone();
        Step::Emit(self.available())
    }

    pub fn start(&mut self, game_running: bool) -> Result<Step, String> {
        if self.version.is_none() {
            return Err("No hay ninguna actualización disponible.".to_string());
        }
        if self.ready {
            if game_running {
                self.install_on_exit = true;
                return Ok(Step::Nothing);
            }
            return Ok(Step::Install);
        }
        Ok(self.begin_download())
    }

    pub fn progress(&self, percent: u32) -> Value {
        launcher("downloading", json!({ "version": self.version, "percent": percent.min(100) }))
    }

    pub fn downloaded(&mut self, game_running: bool) -> Step {
        self.downloading = false;
        self.ready = true;
        if game_running {
            self.install_on_exit = true;
            Step::Emit(launcher("ready", json!({ "version": self.version, "waitingGame": true })))
        } else {
            Step::Install
        }
    }

    pub fn download_failed(&mut self) -> Value {
        self.downloading = false;
        self.available()
    }

    pub fn installing(&self) -> Value {
        launcher("ready", json!({ "version": self.version, "installing": true }))
    }

    pub fn game_exited(&mut self) -> Step {
        if self.ready && self.install_on_exit {
            self.install_on_exit = false;
            Step::Install
        } else {
            Step::Nothing
        }
    }
}

#[derive(Default)]
pub struct LiveUpdate {
    flow: Mutex<Flow>,
    update: Mutex<Option<Update>>,
    bytes: Mutex<Option<Vec<u8>>>,
}

pub fn supported() -> bool {
    !cfg!(debug_assertions) && nova::repo().is_some()
}

fn live(app: &AppHandle) -> &LiveUpdate {
    &app.state::<AppState>().inner().updates
}

fn emit(app: &AppHandle, payload: Value) {
    let _ = app.emit("update:status", payload);
}

fn note(app: &AppHandle, msg: impl AsRef<str>) {
    game::log(app, format!("[Actualizador] {}", msg.as_ref()));
}

fn running(app: &AppHandle) -> bool {
    game::is_running(app).is_some()
}

fn with_flow<T>(app: &AppHandle, f: impl FnOnce(&mut Flow) -> T) -> Option<T> {
    live(app).flow.lock().ok().map(|mut g| f(&mut g))
}

fn run(app: &AppHandle, step: Step) {
    match step {
        Step::Nothing => {}
        Step::Emit(v) => emit(app, v),
        Step::Countdown(v, turn) => {
            emit(app, v);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let step = with_flow(&app, |f| f.tick(turn, running(&app))).unwrap_or(Step::Nothing);
                    let last = !matches!(step, Step::Emit(ref v) if v["state"] == "countdown");
                    run(&app, step);
                    if last {
                        break;
                    }
                }
            });
        }
        Step::Download => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { download(&app).await });
        }
        Step::Install => install(app),
    }
}

fn endpoint() -> Option<tauri::Url> {
    let repo = nova::repo()?;
    tauri::Url::parse(&format!("https://github.com/{repo}/releases/latest/download/latest.json")).ok()
}

async fn check_launcher(app: &AppHandle) -> Result<&'static str, String> {
    if !supported() {
        return Ok("unsupported");
    }
    if with_flow(app, |f| f.busy()).unwrap_or(true) {
        return Ok("skipped");
    }
    let url = endpoint().ok_or("Repositorio de GitHub no configurado.")?;
    run(app, with_flow(app, |f| f.checking()).unwrap_or(Step::Nothing));
    let current = app.package_info().version.to_string();
    let found = async {
        app.updater_builder()
            .endpoints(vec![url])
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            note(app, format!("Versión instalada {current}; hay una nueva: {version}."));
            if let Ok(mut u) = live(app).update.lock() {
                *u = Some(update);
            }
            run(app, with_flow(app, |f| f.found(&version, running(app))).unwrap_or(Step::Nothing));
        }
        Ok(None) => {
            note(app, format!("Versión instalada {current}: es la última."));
            run(app, with_flow(app, |f| f.nothing_new()).unwrap_or(Step::Nothing));
        }
        Err(e) => {
            note(app, format!("No se pudo buscar ({e})."));
            run(app, with_flow(app, |f| f.check_failed(&e)).unwrap_or(Step::Nothing));
            return Err(e);
        }
    }
    Ok("checked")
}

async fn download(app: &AppHandle) {
    let update = live(app).update.lock().ok().and_then(|u| u.clone());
    let Some(update) = update else {
        let v = with_flow(app, |f| f.download_failed());
        emit(app, launcher("error", json!({ "message": "No hay ninguna actualización para descargar." })));
        if let Some(v) = v {
            emit(app, v);
        }
        return;
    };
    if let Some(v) = with_flow(app, |f| f.progress(0)) {
        emit(app, v);
    }
    let mut got: u64 = 0;
    let mut last = 0u32;
    let app2 = app.clone();
    let result = update
        .download(
            move |chunk, total| {
                got += chunk as u64;
                if let Some(t) = total.filter(|t| *t > 0) {
                    let p = (got * 100 / t) as u32;
                    if p != last {
                        last = p;
                        if let Some(v) = with_flow(&app2, |f| f.progress(p)) {
                            emit(&app2, v);
                        }
                    }
                }
            },
            || {},
        )
        .await;
    match result {
        Ok(bytes) => {
            if let Ok(mut b) = live(app).bytes.lock() {
                *b = Some(bytes);
            }
            run(app, with_flow(app, |f| f.downloaded(running(app))).unwrap_or(Step::Nothing));
        }
        Err(e) => {
            note(app, format!("No se pudo descargar ({e})."));
            emit(app, launcher("error", json!({ "message": format!("No se pudo descargar la actualización: {e}") })));
            if let Some(v) = with_flow(app, |f| f.download_failed()) {
                emit(app, v);
            }
        }
    }
}

fn install(app: &AppHandle) {
    if let Some(v) = with_flow(app, |f| f.installing()) {
        emit(app, v);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INSTALL_DELAY).await;
        let update = live(&app).update.lock().ok().and_then(|u| u.clone());
        let bytes = live(&app).bytes.lock().ok().and_then(|b| b.clone());
        if let (Some(update), Some(bytes)) = (update, bytes) {
            if let Err(e) = update.install(bytes) {
                note(&app, format!("No se pudo instalar ({e})."));
                emit(&app, launcher("error", json!({ "message": format!("No se pudo instalar la actualización: {e}") })));
            }
        }
    });
}

pub async fn check(app: &AppHandle, manual: bool) -> Value {
    let launcher_state = check_launcher(app).await.unwrap_or("error");
    if manual && launcher_state == "unsupported" {
        emit(app, launcher("unsupported", json!({})));
    }
    let user_dir = app.state::<AppState>().user_dir.clone();
    let app2 = app.clone();
    let logger = move |m: String| game::log(&app2, m);
    let mods = match nova::refresh_cached(&user_dir, &logger).await {
        Ok((tag, updated)) => {
            if !updated.is_empty() {
                emit(app, json!({ "kind": "mods", "state": "updated", "tag": tag, "versions": updated }));
            }
            updated
        }
        Err(e) => {
            if manual {
                game::log(app, format!("[Actualizaciones] No se pudo buscar el mod: {e}"));
            }
            Vec::new()
        }
    };
    json!({ "launcher": launcher_state, "mods": mods })
}

pub fn schedule(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(&app, false).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

pub fn hold(app: &AppHandle) {
    run(app, with_flow(app, |f| f.hold()).unwrap_or(Step::Nothing));
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    let step = with_flow(app, |f| f.start(running(app))).ok_or("Estado interno bloqueado.")??;
    run(app, step);
    Ok(())
}

pub fn on_game_exit(app: &AppHandle) {
    run(app, with_flow(app, |f| f.game_exited()).unwrap_or(Step::Nothing));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(s: &Step) -> String {
        match s {
            Step::Emit(v) | Step::Countdown(v, _) => v["state"].as_str().unwrap_or("").to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn sin_minecraft_cuenta_regresiva_descarga_e_instala() {
        let mut f = Flow::default();
        assert_eq!(state(&f.checking()), "checking");
        let s = f.found("1.2.0", false);
        let Step::Countdown(v, turn) = s else { panic!("{s:?}") };
        assert_eq!(v["seconds"], 10);
        assert_eq!(v["version"], "1.2.0");

        assert_eq!(f.found("1.2.0", false), Step::Nothing);
        assert_eq!(f.checking(), Step::Nothing);
        for left in (1..10).rev() {
            let s = f.tick(turn, false);
            let Step::Emit(v) = s else { panic!("{s:?}") };
            assert_eq!(v["seconds"], left);
        }
        assert_eq!(f.tick(turn, false), Step::Download);
        assert_eq!(f.progress(140)["percent"], 100);
        assert_eq!(f.downloaded(false), Step::Install);
        assert_eq!(f.installing()["installing"], true);
    }

    #[test]
    fn con_minecraft_abierto_es_manual_y_se_instala_al_cerrar() {
        let mut f = Flow::default();
        assert_eq!(state(&f.found("1.2.0", true)), "available");
        assert_eq!(f.game_exited(), Step::Nothing);
        assert_eq!(f.start(true).unwrap(), Step::Download);
        let s = f.downloaded(true);
        assert_eq!(state(&s), "ready");
        let Step::Emit(v) = s else { unreachable!() };
        assert_eq!(v["waitingGame"], true);
        assert_eq!(f.game_exited(), Step::Install);
        assert_eq!(f.game_exited(), Step::Nothing, "solo una vez");
    }

    #[test]
    fn si_se_abre_minecraft_durante_la_cuenta_pasa_a_manual() {
        let mut f = Flow::default();
        let Step::Countdown(_, turn) = f.found("2.0.0", false) else { panic!() };
        assert_eq!(state(&f.tick(turn, true)), "available");
        assert_eq!(f.tick(turn, false), Step::Nothing, "la cuenta quedó cancelada");
    }

    #[test]
    fn frenar_el_cartel_no_vuelve_a_empezar_solo() {
        let mut f = Flow::default();
        let Step::Countdown(_, turn) = f.found("2.0.0", false) else { panic!() };
        assert_eq!(state(&f.hold()), "available");
        assert_eq!(f.tick(turn, false), Step::Nothing);

        assert_eq!(state(&f.found("2.0.0", false)), "available");

        assert!(matches!(f.found("2.0.1", false), Step::Countdown(..)));
    }

    #[test]
    fn errores_y_reintentos() {
        let mut f = Flow::default();
        assert!(f.start(false).is_err(), "sin actualización no hay nada que continuar");
        assert_eq!(state(&f.check_failed("sin conexión")), "error");
        f.found("3.0.0", true);
        assert_eq!(f.check_failed("sin conexión"), Step::Nothing, "con una pendiente no se muestra el error");
        assert_eq!(f.start(false).unwrap(), Step::Download);
        assert_eq!(f.start(false).unwrap(), Step::Nothing, "no se baja dos veces");
        assert_eq!(f.download_failed()["state"], "available");
        assert_eq!(f.start(false).unwrap(), Step::Download, "se puede reintentar");
        assert_eq!(f.downloaded(true), Step::Emit(launcher("ready", json!({ "version": "3.0.0", "waitingGame": true }))));
        assert!(f.busy());

        assert_eq!(f.start(false).unwrap(), Step::Install);
    }
}

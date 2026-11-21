// Discord Rich Presence sin dependencias: habla directo con el IPC local de Discord
// (named pipe en Windows, socket Unix en Mac/Linux). Si Discord no está abierto, reintenta solo.

use serde_json::{json, Map, Value};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

const OP_HANDSHAKE: i32 = 0;
const OP_FRAME: i32 = 1;
const OP_CLOSE: i32 = 2;
const OP_PING: i32 = 3;
const OP_PONG: i32 = 4;
const RETRY: Duration = Duration::from_secs(15);

pub type Log = Arc<dyn Fn(String) + Send + Sync>;

/// ID de la aplicación de Discord: el de Ajustes manda, si no el de trident.config.json.
pub fn client_id(settings: &Map<String, Value>) -> String {
    let own = settings.get("discordClientId").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if !own.is_empty() {
        return own;
    }
    crate::nova::config_string("discordClientId")
}

/// Clave (o URL https) de la imagen grande. Vacío = sin imagen.
pub fn large_image(settings: &Map<String, Value>) -> String {
    let own = settings.get("discordLargeImage").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if !own.is_empty() {
        return own;
    }
    crate::nova::config_string("discordLargeImage")
}

/// Un ID de aplicación de Discord son 15 a 25 dígitos.
pub fn is_valid_id(id: &str) -> bool {
    let n = id.len();
    (15..=25).contains(&n) && id.bytes().all(|b| b.is_ascii_digit())
}

// ───────────────────────── Protocolo ─────────────────────────

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, op: i32, data: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut buf = Vec::with_capacity(8 + body.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&(body.len() as i32).to_le_bytes());
    buf.extend_from_slice(&body);
    w.write_all(&buf).await?;
    w.flush().await
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(i32, Value)> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head).await?;
    let op = i32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let len = i32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    if !(0..=(1 << 20)).contains(&len) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "mensaje de Discord demasiado grande"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

fn nonce() -> String {
    format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>())
}

/// SET_ACTIVITY con la actividad, o sin ella para quitarla.
async fn send_activity<W: AsyncWrite + Unpin>(w: &mut W, activity: &Option<Value>) -> io::Result<()> {
    let mut args = json!({ "pid": std::process::id() });
    if let Some(a) = activity {
        args["activity"] = a.clone();
    }
    write_frame(w, OP_FRAME, &json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce() })).await
}

// ───────────────────────── Conexión ─────────────────────────

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

#[cfg(windows)]
async fn connect() -> Option<Box<dyn Io>> {
    use tokio::net::windows::named_pipe::ClientOptions;
    for i in 0..10 {
        if let Ok(pipe) = ClientOptions::new().open(format!(r"\\?\pipe\discord-ipc-{i}")) {
            return Some(Box::new(pipe));
        }
    }
    None
}

#[cfg(unix)]
async fn connect() -> Option<Box<dyn Io>> {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "/tmp".to_string());
    let base = std::path::PathBuf::from(base);
    for i in 0..10 {
        let candidates = [
            base.join(format!("discord-ipc-{i}")),
            base.join("app").join("com.discordapp.Discord").join(format!("discord-ipc-{i}")),
            base.join("snap.discord").join(format!("discord-ipc-{i}")),
        ];
        for path in candidates {
            if let Ok(sock) = tokio::net::UnixStream::connect(path).await {
                return Some(Box::new(sock));
            }
        }
    }
    None
}

/// Una conexión con Discord: saludo, espera READY, publica la actividad y la mantiene al día.
/// Termina cuando Discord cierra, hay un error o se suelta el emisor de la actividad.
async fn session<C: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    conn: C,
    client_id: &str,
    rx: &mut watch::Receiver<Option<Value>>,
    log: &Log,
) {
    let (mut rd, mut wr) = tokio::io::split(conn);
    if write_frame(&mut wr, OP_HANDSHAKE, &json!({ "v": 1, "client_id": client_id })).await.is_err() {
        return;
    }

    // La lectura va en su propia tarea: `read_exact` no se puede cancelar a mitad de un mensaje.
    let (tx, mut frames) = mpsc::channel::<(i32, Value)>(16);
    let reader = tokio::spawn(async move {
        while let Ok(frame) = read_frame(&mut rd).await {
            if tx.send(frame).await.is_err() {
                break;
            }
        }
    });

    let mut ready = false;
    loop {
        tokio::select! {
            frame = frames.recv() => {
                let Some((op, msg)) = frame else { break };
                match op {
                    OP_PING => {
                        if write_frame(&mut wr, OP_PONG, &msg).await.is_err() {
                            break;
                        }
                    }
                    OP_CLOSE => {
                        let why = msg["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default();
                        log(format!("[Discord] Conexión cerrada por Discord{why}"));
                        break;
                    }
                    OP_FRAME => match msg["evt"].as_str() {
                        Some("READY") => {
                            ready = true;
                            let user = msg["data"]["user"]["username"].as_str().unwrap_or("usuario");
                            log(format!("[Discord] Conectado como {user}."));
                            let current = rx.borrow_and_update().clone();
                            if send_activity(&mut wr, &current).await.is_err() {
                                break;
                            }
                        }
                        Some("ERROR") => {
                            let why = msg["data"]["message"].as_str().unwrap_or("desconocido");
                            log(format!("[Discord] Error: {why}"));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            changed = rx.changed() => {
                if changed.is_err() {
                    break; // se soltó el emisor: «stop»
                }
                let current = rx.borrow_and_update().clone();
                if ready && send_activity(&mut wr, &current).await.is_err() {
                    break;
                }
            }
        }
    }
    reader.abort();
}

async fn run(client_id: String, mut rx: watch::Receiver<Option<Value>>, log: Log) {
    loop {
        if let Some(conn) = connect().await {
            session(conn, &client_id, &mut rx, &log).await;
        }
        // Espera para reintentar, pero sale en cuanto se suelta el emisor.
        tokio::select! {
            _ = tokio::time::sleep(RETRY) => {}
            changed = rx.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

// ───────────────────────── Interfaz pública ─────────────────────────

/// Actividad en marcha. `stop()` la quita de Discord y cierra la conexión.
pub struct Presence {
    tx: watch::Sender<Option<Value>>,
}

impl Presence {
    pub fn start(client_id: String, activity: Value, log: Log) -> Presence {
        let (tx, rx) = watch::channel(Some(activity));
        tauri::async_runtime::spawn(run(client_id, rx, log));
        Presence { tx }
    }

    pub fn stop(self) {
        let tx = self.tx;
        tauri::async_runtime::spawn(async move {
            let _ = tx.send(None);
            // Da tiempo a que salga el mensaje que quita la actividad antes de cerrar.
            tokio::time::sleep(Duration::from_millis(300)).await;
            drop(tx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn id_de_aplicacion() {
        assert!(is_valid_id("1543151181990469702"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("12345"));
        assert!(!is_valid_id("15431511819904697a2"));
        assert!(!is_valid_id(&"1".repeat(26)));
    }

    #[test]
    fn el_id_de_ajustes_manda_sobre_la_config() {
        let mut s = Map::new();
        assert!(is_valid_id(&client_id(&s)), "debe usar el de trident.config.json");
        s.insert("discordClientId".into(), json!(" 111111111111111111 "));
        assert_eq!(client_id(&s), "111111111111111111");
    }

    #[tokio::test]
    async fn conversacion_completa_con_un_discord_falso() {
        let (client, mut server) = tokio::io::duplex(8192);
        let logs: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = logs.clone();
        let log: Log = Arc::new(move |m: String| sink.lock().unwrap().push(m));

        let (tx, mut rx) = watch::channel(Some(json!({ "details": "Minecraft 1.21.4", "state": "Jugando" })));
        let task = tokio::spawn(async move { session(client, "1543151181990469702", &mut rx, &log).await });

        // 1. Saludo con el ID de la aplicación.
        let (op, hello) = read_frame(&mut server).await.unwrap();
        assert_eq!(op, OP_HANDSHAKE);
        assert_eq!(hello["client_id"], "1543151181990469702");
        assert_eq!(hello["v"], 1);

        // 2. READY → publica la actividad inicial.
        write_frame(&mut server, OP_FRAME, &json!({ "evt": "READY", "data": { "user": { "username": "alex" } } })).await.unwrap();
        let (op, set) = read_frame(&mut server).await.unwrap();
        assert_eq!(op, OP_FRAME);
        assert_eq!(set["cmd"], "SET_ACTIVITY");
        assert_eq!(set["args"]["activity"]["details"], "Minecraft 1.21.4");
        assert_eq!(set["args"]["pid"], std::process::id());

        // 3. PING → PONG con el mismo contenido.
        write_frame(&mut server, OP_PING, &json!({ "n": 7 })).await.unwrap();
        let (op, pong) = read_frame(&mut server).await.unwrap();
        assert_eq!(op, OP_PONG);
        assert_eq!(pong["n"], 7);

        // 4. Cambia la actividad.
        tx.send(Some(json!({ "details": "Otra", "state": "Jugando" }))).unwrap();
        let (_, set) = read_frame(&mut server).await.unwrap();
        assert_eq!(set["args"]["activity"]["details"], "Otra");

        // 5. Quitar la actividad: SET_ACTIVITY sin «activity».
        tx.send(None).unwrap();
        let (_, clear) = read_frame(&mut server).await.unwrap();
        assert_eq!(clear["cmd"], "SET_ACTIVITY");
        assert!(clear["args"].get("activity").is_none());

        // 6. Soltar el emisor cierra la sesión.
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), task).await.expect("la sesión debía terminar").unwrap();

        let logs = logs.lock().unwrap();
        assert!(logs.iter().any(|l| l.contains("Conectado como alex")), "{logs:?}");
    }

    #[tokio::test]
    async fn si_discord_cierra_la_sesion_termina() {
        let (client, mut server) = tokio::io::duplex(8192);
        let log: Log = Arc::new(|_: String| {});
        let (_tx, mut rx) = watch::channel(Some(json!({})));
        let task = tokio::spawn(async move { session(client, "1543151181990469702", &mut rx, &log).await });
        let _ = read_frame(&mut server).await.unwrap();
        write_frame(&mut server, OP_CLOSE, &json!({ "message": "Invalid Client ID" })).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), task).await.expect("debía terminar").unwrap();
    }

    #[tokio::test]
    async fn mensaje_enorme_se_rechaza() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[1, 0, 0, 0, 0xff, 0xff, 0xff, 0x7f]).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
    }
}

//! Tempo de jogo e última vez jogado, por jogo.
//!
//! O processo principal mede: entrar num jogo do catálogo (navegação do frame
//! principal, ver `client.rs`) abre uma sessão; ir para a Biblioteca, para
//! `about:blank` (jogo estacionado, ver `shell.rs`) ou fechar o browser a encerra.
//! Os totais ficam em `~/.local/share/web-avenger/playtime.json`, que a Biblioteca lê
//! por `avengerStats()`.

use crate::{catalog, profiles};
use cef::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Intervalo do heartbeat que grava o parcial de uma sessão em andamento.
const HEARTBEAT_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Tempo total de jogo, em segundos.
    #[serde(default)]
    pub seconds: u64,
    /// Última vez jogado (segundos Unix).
    #[serde(default)]
    pub last_played: u64,
}

pub type AllStats = BTreeMap<String, Stats>;

struct Session {
    game_id: String,
    browser_id: i32,
    /// Desde quando o tempo ainda não foi gravado.
    since: Instant,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// Geração da sessão atual: heartbeats de sessões antigas param sozinhos.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn path() -> PathBuf {
    crate::data_dir().join("playtime.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn load() -> AllStats {
    load_from(&path())
}

fn load_from(path: &Path) -> AllStats {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Soma `seconds` ao jogo e marca `last_played = at`, gravando o arquivo.
fn record(path: &Path, game_id: &str, seconds: u64, at: u64) -> io::Result<()> {
    let mut all = load_from(path);
    let stats = all.entry(game_id.to_string()).or_default();
    stats.seconds += seconds;
    stats.last_played = stats.last_played.max(at);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&all)?)?;
    fs::rename(tmp, path)
}

/// Grava o tempo desde o último registro da sessão (e reinicia a contagem).
fn flush(session: &mut Session) {
    let seconds = session.since.elapsed().as_secs();
    session.since = Instant::now();
    if let Err(err) = record(&path(), &session.game_id, seconds, now()) {
        eprintln!("[web-avenger] falha ao gravar o tempo de jogo: {err}");
    }
}

/// Jogo do catálogo (manual ou importado) para o qual a URL navega.
pub fn catalog_game_for(url: &str) -> Option<catalog::Game> {
    catalog_game_in(catalog::load(), url)
}

fn catalog_game_in(games: Vec<catalog::Game>, url: &str) -> Option<catalog::Game> {
    games.into_iter().find(|game| profiles::matches(&game.url, url))
}

/// Navegação do frame principal de um browser (`on_address_change`).
pub fn on_navigation(browser_id: i32, url: &str) {
    let game = catalog_game_for(url);
    let Ok(mut session) = SESSION.lock() else {
        return;
    };
    match (session.as_ref(), &game) {
        // Navegação dentro do jogo em andamento.
        (Some(current), Some(game)) if current.browser_id == browser_id && current.game_id == game.id => return,
        // Outro browser saindo de um jogo que não é o da sessão (ex.: o estacionado).
        (Some(current), None) if current.browser_id != browser_id => return,
        _ => {}
    }
    if let Some(mut finished) = session.take() {
        flush(&mut finished);
    }
    if let Some(game) = game {
        let mut started = Session { game_id: game.id, browser_id, since: Instant::now() };
        // Marca "último jogado" já na entrada (com 0 s).
        if let Err(err) = record(&path(), &started.game_id, 0, now()) {
            eprintln!("[web-avenger] falha ao gravar o tempo de jogo: {err}");
        }
        started.since = Instant::now();
        *session = Some(started);
        drop(session);
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        post_delayed_task(ThreadId::UI, Some(&mut HeartbeatTask::new(generation)), HEARTBEAT_MS);
    }
}

/// O browser fechou: encerra a sessão dele, se houver.
pub fn on_browser_closed(browser_id: i32) {
    let Ok(mut session) = SESSION.lock() else {
        return;
    };
    if session.as_ref().is_some_and(|current| current.browser_id == browser_id) {
        if let Some(mut finished) = session.take() {
            flush(&mut finished);
        }
    }
}

/// Encerra qualquer sessão (janela fechando).
pub fn end_all() {
    if let Some(mut finished) = SESSION.lock().ok().and_then(|mut session| session.take()) {
        flush(&mut finished);
    }
}

wrap_task! {
    struct HeartbeatTask {
        generation: u64,
    }

    impl Task {
        fn execute(&self) {
            if self.generation != GENERATION.load(Ordering::Relaxed) {
                return; // outra sessão começou e tem o próprio heartbeat
            }
            let Ok(mut session) = SESSION.lock() else {
                return;
            };
            let Some(current) = session.as_mut() else {
                return; // sessão acabou: o heartbeat para
            };
            flush(current);
            drop(session);
            post_delayed_task(ThreadId::UI, Some(&mut HeartbeatTask::new(self.generation)), HEARTBEAT_MS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: &str, url: &str, source: Option<&str>) -> catalog::Game {
        serde_json::from_value(serde_json::json!({ "id": id, "name": id, "url": url, "source": source })).unwrap()
    }

    #[test]
    fn acha_o_jogo_pela_url() {
        let games = || {
            vec![
                game("colorful-path", "https://html-classic.itch.zone/html/6182212/index.html", None),
                game("polytrack", "https://html-classic.itch.zone/html/19260993/index.html", Some("itch.io")),
            ]
        };
        let id = |url| catalog_game_in(games(), url).map(|g| g.id);
        assert_eq!(id("https://html-classic.itch.zone/html/19260993/index.html").as_deref(), Some("polytrack"));
        assert_eq!(id("https://html-classic.itch.zone/html/6182212/Build/a.js").as_deref(), Some("colorful-path"));
        assert_eq!(id("about:blank"), None);
    }

    #[test]
    fn soma_sessoes_e_guarda_ultima_vez() {
        let dir = std::env::temp_dir().join(format!("web-avenger-playtime-{}", std::process::id()));
        let path = dir.join("playtime.json");
        record(&path, "jogo", 30, 1000).unwrap();
        record(&path, "jogo", 45, 2000).unwrap();
        record(&path, "outro", 0, 1500).unwrap();
        // Um registro mais antigo não volta o "último jogado".
        record(&path, "jogo", 5, 1200).unwrap();
        let all = load_from(&path);
        assert_eq!(all["jogo"], Stats { seconds: 80, last_played: 2000 });
        assert_eq!(all["outro"], Stats { seconds: 0, last_played: 1500 });
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn arquivo_ausente_ou_invalido_vira_vazio() {
        assert!(load_from(Path::new("/nao/existe/playtime.json")).is_empty());
    }
}

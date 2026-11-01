//! Perfil próprio (e persistente) para cada jogo manual.
//!
//! Jogos manuais (sem `source`, ver `catalog::Game`) rodam num `RequestContext` com
//! pasta própria em `~/.local/share/web-avenger/game-<id>/`: cookies, localStorage,
//! IndexedDB e cache isolados do resto do app. A Biblioteca e os jogos importados do
//! itch.io continuam no perfil principal (precisam da sessão do itch.io).

use crate::catalog::Game;
use cef::*;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
};

thread_local! {
    /// Um contexto por jogo, criado uma vez: o CEF não aceita duas instâncias da mesma pasta.
    static CONTEXTS: RefCell<HashMap<String, RequestContext>> = RefCell::new(HashMap::new());
}

/// URL sem query, fragmento e `/` final.
fn strip(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/')
}

/// Se a navegação para `url` é o jogo cadastrado em `game_url`: a mesma URL, ou algo
/// dentro da pasta dela (ex.: `…/html/6182212/` para `…/html/6182212/index.html`).
/// Compara por URL, não por origem: jogos diferentes podem dividir a mesma origem
/// (todos os jogos HTML5 do itch.io ficam em `html-classic.itch.zone`).
pub fn matches(game_url: &str, url: &str) -> bool {
    let (game, url) = (strip(game_url), strip(url));
    if game.eq_ignore_ascii_case(url) {
        return true;
    }
    let after_scheme = game.find("://").map_or(0, |i| i + 3);
    let dir = match game[after_scheme..].rfind('/') {
        // Último segmento com extensão (`index.html`): a pasta é o que vem antes.
        Some(i) if game[after_scheme + i + 1..].contains('.') => &game[..after_scheme + i + 1],
        _ => &game[..],
    };
    let dir = dir.trim_end_matches('/');
    url.len() > dir.len() && url[..dir.len()].eq_ignore_ascii_case(dir) && url.as_bytes()[dir.len()] == b'/'
}

pub type OnReady = Box<dyn FnOnce(RequestContext)>;

/// Entrega o contexto (perfil) do jogo a `on_ready`; na primeira vez cria a pasta,
/// migra os saves e espera o CEF inicializar o perfil (no estilo Chrome isso é
/// assíncrono, e um browser criado antes disso falha).
pub fn with_context(game: &Game, on_ready: OnReady) {
    if let Some(context) = CONTEXTS.with(|contexts| contexts.borrow().get(&game.id).cloned()) {
        return on_ready(context);
    }
    // O CEF (estilo Chrome) só aceita perfis diretamente dentro do `root_cache_path`.
    let dir = crate::data_dir().join(format!("game-{}", game.id));
    if !dir.exists() {
        if let Err(err) = migrate_indexeddb(&game.url, &crate::data_dir().join("Default"), &dir) {
            eprintln!("[web-avenger] saves de {} não migrados: {err}", game.id);
        }
    }
    let settings = RequestContextSettings {
        cache_path: CefString::from(dir.to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        ..Default::default()
    };
    let mut handler = ContextReady::new(game.id.clone(), Arc::new(Mutex::new(Some(on_ready))));
    if request_context_create_context(Some(&settings), Some(&mut handler)).is_none() {
        eprintln!("[web-avenger] não foi possível criar o perfil de {}", game.id);
    }
}

wrap_request_context_handler! {
    struct ContextReady {
        game_id: String,
        on_ready: Arc<Mutex<Option<OnReady>>>,
    }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, request_context: Option<&mut RequestContext>) {
            let Some(context) = request_context.map(|context| context.clone()) else {
                return;
            };
            CONTEXTS.with(|contexts| contexts.borrow_mut().insert(self.game_id.clone(), context.clone()));
            let on_ready = self.on_ready.lock().ok().and_then(|mut on_ready| on_ready.take());
            if let Some(on_ready) = on_ready {
                on_ready(context);
            }
        }
    }
}

/// Solta os contextos guardados: o `shutdown()` do CEF trava se ainda houver referências.
pub fn release_all() {
    CONTEXTS.with(|contexts| contexts.borrow_mut().clear());
}

/// Nome que o Chromium dá ao IndexedDB de uma origem: `https_exemplo.com_0`.
fn indexeddb_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host_port = rest.split(['/', '?', '#']).next()?;
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => (host, port),
        _ => (host_port, "0"),
    };
    Some(format!("{}_{}_{}", scheme.to_lowercase(), host.to_lowercase(), port))
}

/// Copia o IndexedDB da origem do jogo do perfil principal para a pasta nova, para
/// os saves feitos antes do perfil próprio continuarem lá. O localStorage não dá
/// para migrar: é um banco único para todas as origens.
fn migrate_indexeddb(game_url: &str, main_profile: &Path, game_dir: &Path) -> io::Result<()> {
    let Some(origin) = indexeddb_origin(game_url) else {
        return Ok(());
    };
    for suffix in ["indexeddb.leveldb", "indexeddb.blob"] {
        let name = format!("{origin}.{suffix}");
        let source = main_profile.join("IndexedDB").join(&name);
        if source.is_dir() {
            copy_dir(&source, &game_dir.join("IndexedDB").join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if entry.file_name() != "LOCK" {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: &str, url: &str, source: Option<&str>) -> Game {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "url": url, "source": source,
        }))
        .unwrap()
    }

    #[test]
    fn casa_url_do_jogo() {
        assert!(matches("https://skate.aaddpp.lol", "https://skate.aaddpp.lol/"));
        assert!(matches("https://skate.aaddpp.lol", "https://skate.aaddpp.lol/play?x=1"));
        let unity = "https://html-classic.itch.zone/html/6182212/index.html";
        assert!(matches(unity, "https://html-classic.itch.zone/html/6182212/index.html?v=2"));
        assert!(matches(unity, "https://html-classic.itch.zone/html/6182212/Build/x.js"));
        assert!(!matches(unity, "https://html-classic.itch.zone/html/19260993/index.html"));
        assert!(!matches(unity, "https://html-classic.itch.zone/html/61822120/index.html"));
        assert!(!matches("https://a.com/jogo", "https://a.com/jogo2"));
    }

    #[test]
    fn nome_do_indexeddb_da_origem() {
        assert_eq!(indexeddb_origin("https://html-classic.itch.zone/html/1/index.html").unwrap(), "https_html-classic.itch.zone_0");
        assert_eq!(indexeddb_origin("http://127.0.0.1:8080/x").unwrap(), "http_127.0.0.1_8080");
    }
}

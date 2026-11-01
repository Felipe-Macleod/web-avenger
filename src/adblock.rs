//! Bloqueio de anúncios e rastreadores nas janelas de jogo (motor do Brave, crate
//! `adblock`, com as listas EasyList e EasyPrivacy).
//!
//! Só bloqueia requisições de rede; a página principal nunca é bloqueada. As listas ficam
//! em `~/.local/share/web-avenger/adblock/` e são baixadas de novo a cada 24 h. A
//! Biblioteca (e os logins do itch.io/DMM, que navegam nela) fica sem bloqueio, e cada
//! jogo pode desligá-lo (`no_adblock`, ver `catalog::Game`).

use crate::http;
use adblock::{lists::ParseOptions, request::Request, Engine, FilterSet};
use cef::ResourceType;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};

const LISTS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
];
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

static ENGINE: RwLock<Option<Arc<Engine>>> = RwLock::new(None);
/// Browsers (janelas de jogo) com o bloqueio ligado.
static ENABLED: Mutex<Option<HashSet<i32>>> = Mutex::new(None);

fn dir() -> PathBuf {
    crate::data_dir().join("adblock")
}

/// Monta o motor com as listas em cache e atualiza as que estiverem velhas. Roda na
/// thread de UI do processo principal (os downloads usam `http::fetch`).
pub fn init() {
    std::thread::spawn(rebuild);
    for (file, url) in LISTS {
        let path = dir().join(file);
        let fresh = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| SystemTime::now().duration_since(modified).is_ok_and(|age| age < MAX_AGE));
        if !fresh {
            download(path, url);
        }
    }
}

fn download(path: PathBuf, url: &'static str) {
    http::fetch(
        http::Request::get(url),
        Box::new(move |result| {
            let saved = match result {
                Ok(response) if response.ok() => fs::create_dir_all(dir())
                    .and_then(|()| fs::write(&path, &response.body))
                    .map_err(|e| e.to_string()),
                Ok(response) => Err(format!("HTTP {}", response.status)),
                Err(error) => Err(error),
            };
            match saved {
                Ok(()) => {
                    std::thread::spawn(rebuild);
                }
                Err(error) => eprintln!("[web-avenger] adblock: falha ao baixar {url}: {error}"),
            }
        }),
    );
}

/// Remonta o motor com as listas que estiverem em disco (fora da thread de UI: são
/// dezenas de milhares de regras).
fn rebuild() {
    let lists: Vec<String> = LISTS.iter().filter_map(|(file, _)| fs::read_to_string(dir().join(file)).ok()).collect();
    if lists.is_empty() {
        return;
    }
    let engine = engine_from(&lists);
    if let Ok(mut current) = ENGINE.write() {
        *current = Some(Arc::new(engine));
    }
}

fn engine_from(lists: &[String]) -> Engine {
    let mut filters = FilterSet::new(false);
    for list in lists {
        filters.add_filter_list(list.clone(), ParseOptions::default());
    }
    Engine::new_with_filter_set(filters)
}

/// Liga ou desliga o bloqueio no browser (chamado quando a janela do jogo é criada).
pub fn set_enabled(browser_id: i32, enabled: bool) {
    if let Ok(mut set) = ENABLED.lock() {
        let set = set.get_or_insert_with(HashSet::new);
        if enabled {
            set.insert(browser_id);
        } else {
            set.remove(&browser_id);
        }
    }
}

/// Se a requisição do browser deve ser cancelada. Roda na thread de IO do CEF.
pub fn should_block(browser_id: i32, url: &str, source_url: &str, kind: ResourceType) -> bool {
    let Some(kind) = request_type(kind) else {
        return false;
    };
    let enabled = ENABLED.lock().is_ok_and(|set| set.as_ref().is_some_and(|set| set.contains(&browser_id)));
    if !enabled {
        return false;
    }
    let Some(engine) = ENGINE.read().ok().and_then(|engine| engine.clone()) else {
        return false;
    };
    blocks(&engine, url, source_url, kind)
}

fn blocks(engine: &Engine, url: &str, source_url: &str, kind: &str) -> bool {
    Request::new(url, source_url, kind, "GET").is_ok_and(|request| engine.check_network_request(&request).should_block())
}

/// Tipo da requisição no vocabulário das listas; `None` para a página principal.
fn request_type(kind: ResourceType) -> Option<&'static str> {
    Some(match kind {
        ResourceType::MAIN_FRAME => return None,
        ResourceType::SUB_FRAME => "sub_frame",
        ResourceType::STYLESHEET => "stylesheet",
        ResourceType::SCRIPT => "script",
        ResourceType::IMAGE | ResourceType::FAVICON => "image",
        ResourceType::FONT_RESOURCE => "font",
        ResourceType::MEDIA => "media",
        ResourceType::OBJECT => "object",
        ResourceType::XHR => "xmlhttprequest",
        ResourceType::PING => "ping",
        ResourceType::CSP_REPORT => "csp_report",
        _ => "other",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloqueia_pelas_regras() {
        let engine = engine_from(&["||ads.exemplo.com^\n@@||ads.exemplo.com/liberado.js".into()]);
        let page = "https://jogo.exemplo.org/";
        assert!(blocks(&engine, "https://ads.exemplo.com/banner.js", page, "script"));
        assert!(!blocks(&engine, "https://ads.exemplo.com/liberado.js", page, "script"));
        assert!(!blocks(&engine, "https://jogo.exemplo.org/build.wasm", page, "other"));
    }

    #[test]
    fn pagina_principal_nunca_e_bloqueada() {
        assert_eq!(request_type(ResourceType::MAIN_FRAME), None);
        assert_eq!(request_type(ResourceType::XHR), Some("xmlhttprequest"));
        assert_eq!(request_type(ResourceType::FAVICON), Some("image"));
    }
}

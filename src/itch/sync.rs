//! Importa para a Biblioteca os jogos de navegador comprados e das coleções do itch.io.
//!
//! Tudo roda na thread de UI do processo principal, encadeando requisições: primeiro
//! lista os jogos (`/profile/owned-keys` e `/collections/{id}/collection-games`, todas
//! as páginas) e depois, um jogo por vez, lê a página de cada jogo novo para achar a
//! URL do jogo HTML5 (mesma detecção do cadastro manual, com os cookies do perfil para
//! jogos pagos).

use crate::{catalog, http, library, page_info};
use serde::Serialize;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    rc::Rc,
};

const API: &str = "https://api.itch.io";
/// Limite de páginas por coleção (proteção contra paginação que nunca termina).
const MAX_COLLECTION_PAGES: u64 = 50;

#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    /// Jogos de navegador adicionados à Biblioteca.
    pub imported: usize,
    /// Já estavam na Biblioteca (inclusive os que você removeu).
    pub existing: usize,
    /// Só para download (sem versão de navegador).
    pub not_web: usize,
    /// De navegador, mas a URL do jogo não foi encontrada (ex.: exige login no site).
    pub failed: Vec<String>,
    /// Coleções cuja página não pôde ser lida.
    pub skipped_collections: Vec<String>,
}

/// Jogo vindo da API, com a chave de compra (necessária para listar uploads de jogos pagos).
struct Candidate {
    game: Value,
    download_key_id: Option<u64>,
}

struct Job {
    token: String,
    seen: HashSet<u64>,
    known: HashSet<u64>,
    /// Páginas de jogos já na Biblioteca (pega jogos do itch.io cadastrados à mão).
    known_pages: HashSet<String>,
    candidates: VecDeque<Candidate>,
    collections: VecDeque<(u64, String)>,
    new_games: Vec<catalog::Game>,
    report: SyncReport,
    done: Option<Box<dyn FnOnce(Result<SyncReport, String>)>>,
}

type Shared = Rc<RefCell<Job>>;

pub fn run(token: String, done: Box<dyn FnOnce(Result<SyncReport, String>)>) {
    let job = Rc::new(RefCell::new(Job {
        token,
        seen: HashSet::new(),
        known: catalog::known_itch_ids(),
        known_pages: catalog::load_all_pages().into_iter().map(|page| normalize_page(&page)).collect(),
        candidates: VecDeque::new(),
        collections: VecDeque::new(),
        new_games: Vec::new(),
        report: SyncReport::default(),
        done: Some(done),
    }));
    fetch_owned_keys(job, 1);
}

fn fail(job: &Shared, error: String) {
    let done = job.borrow_mut().done.take();
    if let Some(done) = done {
        done(Err(error));
    }
}

fn get_json(job: &Shared, url: String, next: impl FnOnce(Value) + 'static) {
    let token = job.borrow().token.clone();
    let job = job.clone();
    let request = http::Request::get(&url).bearer(&token);
    let url = url.clone();
    http::fetch(
        request,
        Box::new(move |result| match result {
            Ok(response) if response.ok() => match serde_json::from_slice(&response.body) {
                Ok(json) => next(json),
                Err(_) => fail(&job, format!("resposta inválida da API do itch.io ({url})")),
            },
            Ok(response) => {
                let error = serde_json::from_slice::<Value>(&response.body)
                    .ok()
                    .and_then(|json| json["errors"][0].as_str().map(str::to_string))
                    .unwrap_or_else(|| format!("HTTP {}", response.status));
                // Caminho sem o host, para a mensagem dizer qual chamada falhou.
                let endpoint = url.strip_prefix(API).unwrap_or(&url);
                eprintln!("[web-avenger] itch.io {endpoint}: HTTP {} {}", response.status, response.text());
                fail(&job, format!("O itch.io recusou {endpoint} (HTTP {}): {error}", response.status));
            }
            Err(error) => fail(&job, error),
        }),
    );
}

/// A página seguinte existe se esta veio cheia (`per_page` itens).
fn has_next_page(json: &Value, items: usize) -> bool {
    let per_page = json["per_page"].as_u64().unwrap_or(50) as usize;
    items > 0 && items >= per_page
}

fn add_candidate(job: &Shared, game: &Value, download_key_id: Option<u64>) {
    let Some(id) = game["id"].as_u64() else {
        return;
    };
    let mut job = job.borrow_mut();
    if job.seen.insert(id) {
        job.candidates.push_back(Candidate { game: game.clone(), download_key_id });
    }
}

fn fetch_owned_keys(job: Shared, page: u64) {
    let url = format!("{API}/profile/owned-keys?page={page}");
    get_json(&job.clone(), url, move |json| {
        let keys = json["owned_keys"].as_array().cloned().unwrap_or_default();
        for key in &keys {
            add_candidate(&job, &key["game"], key["id"].as_u64());
        }
        if has_next_page(&json, keys.len()) {
            fetch_owned_keys(job, page + 1);
        } else {
            fetch_collections(job);
        }
    });
}

fn fetch_collections(job: Shared) {
    get_json(&job.clone(), format!("{API}/profile/collections"), move |json| {
        let collections = json["collections"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|c| Some((c["id"].as_u64()?, c["title"].as_str().unwrap_or("").to_string())))
                    .collect()
            })
            .unwrap_or_default();
        job.borrow_mut().collections = collections;
        next_collection(job);
    });
}

fn next_collection(job: Shared) {
    let next = job.borrow_mut().collections.pop_front();
    match next {
        Some((id, title)) => fetch_collection_page(job, id, title, 1, HashSet::new()),
        None => next_candidate(job),
    }
}

/// Lê os jogos de uma coleção pela página dela no itch.io, com os cookies do perfil.
/// (O endpoint `/collections/{id}/collection-games` exige o escopo `collection:view`,
/// que o OAuth não concede. Coleções privadas precisam do login feito no app.)
fn fetch_collection_page(job: Shared, collection: u64, title: String, page: u64, mut seen: HashSet<u64>) {
    let url = format!("https://itch.io/c/{collection}?page={page}");
    let mut request = http::Request::get(&url);
    request.cookies = true;
    http::fetch(
        request,
        Box::new(move |result| {
            let html = match result {
                Ok(response) if response.ok() => response.text(),
                _ => {
                    let name = if title.is_empty() { format!("coleção {collection}") } else { title };
                    job.borrow_mut().report.skipped_collections.push(name);
                    return next_collection(job);
                }
            };
            // Para quando a página não traz jogos novos (fim da coleção).
            let games: Vec<Value> = parse_collection_page(&html)
                .into_iter()
                .filter(|game| game["id"].as_u64().is_some_and(|id| seen.insert(id)))
                .collect();
            for game in &games {
                add_candidate(&job, game, None);
            }
            if !games.is_empty() && page < MAX_COLLECTION_PAGES {
                fetch_collection_page(job, collection, title, page + 1, seen);
            } else {
                next_collection(job);
            }
        }),
    );
}

/// Jogos de uma página de coleção (`div.game_cell`), no formato dos jogos da API
/// (`id`, `title`, `url`, `cover_url`, `short_text`, `type`). O selo "Play in browser"
/// (`web_flag`) vira `type: "html"`.
fn parse_collection_page(html: &str) -> Vec<Value> {
    // Só a classe `game_cell` em si (não `game_cell_data`, `game_cell_tools`...).
    let starts: Vec<usize> = html
        .match_indices("class=\"game_cell")
        .filter(|(i, m)| matches!(html.as_bytes().get(i + m.len()), Some(b'"' | b' ')))
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(n, &start)| {
            let tag_start = html[..start].rfind('<')?;
            let end = starts.get(n + 1).map_or(html.len(), |&next| html[..next].rfind('<').unwrap_or(next));
            let cell = &html[tag_start..end];
            let id: u64 = attr_after(cell, "data-game_id=\"")?.parse().ok()?;
            let title_at = cell.find("class=\"title game_link\"").or_else(|| cell.find("class=\"title"))?;
            let link_start = cell[..title_at].rfind("<a ")?;
            let url = attr_after(&cell[link_start..], "href=\"")?;
            let title_end = cell[title_at..].find('>')? + title_at + 1;
            let title = cell[title_end..].split("</a>").next()?.trim().to_string();
            let cover = attr_after(cell, "data-lazy_src=\"").or_else(|| attr_after(cell, "src=\""));
            let text = cell.find("class=\"game_text\"").and_then(|at| {
                let start = cell[at..].find('>')? + at + 1;
                Some(cell[start..].split("</div>").next()?.trim().to_string())
            });
            let web = cell.contains("class=\"web_flag\"");
            Some(serde_json::json!({
                "id": id,
                "title": decode_html(&title),
                "url": url,
                "cover_url": cover,
                "short_text": text.map(|t| decode_html(&t)),
                "type": if web { "html" } else { "default" },
            }))
        })
        .collect()
}

/// Página de jogo comparável: sem esquema, `www.`, query e barra final, em minúsculas.
fn normalize_page(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let url = url.split_once("://").map_or(url, |(_, rest)| rest);
    url.trim_start_matches("www.").trim_end_matches('/').to_lowercase()
}

fn attr_after(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)? + prefix.len();
    let value = &text[start..start + text[start..].find('"')?];
    Some(decode_html(value))
}

fn decode_html(text: &str) -> String {
    text.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// Se o jogo roda no navegador: `Some(true/false)` pelo tipo do projeto, `None` se a
/// API não informou (aí os uploads decidem).
fn is_web_game(game: &Value) -> Option<bool> {
    game["type"].as_str().map(|kind| kind == "html")
}

fn next_candidate(job: Shared) {
    let next = job.borrow_mut().candidates.pop_front();
    let Some(candidate) = next else {
        return finish(job);
    };
    let Some(id) = candidate.game["id"].as_u64() else {
        return next_candidate(job);
    };
    let page = candidate.game["url"].as_str().map(normalize_page).unwrap_or_default();
    if job.borrow().known.contains(&id) || job.borrow().known_pages.contains(&page) {
        job.borrow_mut().report.existing += 1;
        return next_candidate(job);
    }
    match is_web_game(&candidate.game) {
        Some(true) => import(job, candidate),
        Some(false) => {
            job.borrow_mut().report.not_web += 1;
            next_candidate(job);
        }
        None => {
            // Sem o tipo na resposta: procura um upload "html" (jogos pagos pedem a chave).
            let key = candidate.download_key_id.map(|k| format!("?download_key_id={k}")).unwrap_or_default();
            let url = format!("{API}/games/{id}/uploads{key}");
            get_json(&job.clone(), url, move |json| {
                let html = json["uploads"]
                    .as_array()
                    .is_some_and(|uploads| uploads.iter().any(|u| u["type"].as_str() == Some("html")));
                if html {
                    import(job, candidate);
                } else {
                    job.borrow_mut().report.not_web += 1;
                    next_candidate(job);
                }
            });
        }
    }
}

fn import(job: Shared, candidate: Candidate) {
    let game = candidate.game;
    let text = |key: &str| game[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let title = text("title").unwrap_or_else(|| "Jogo do itch.io".into());
    let (short_text, cover, itch_id) = (text("short_text"), text("cover_url"), game["id"].as_u64());
    let Some(page_url) = text("url") else {
        job.borrow_mut().report.failed.push(title);
        return next_candidate(job);
    };

    page_info::fetch(
        page_url.clone(),
        true,
        Box::new(move |info| {
            match info.game_url.clone() {
                Some(game_url) => {
                    let new_game = catalog::Game {
                        id: String::new(),
                        name: info.title.clone().unwrap_or(title),
                        description: short_text.or(info.description.clone()).unwrap_or_default(),
                        url: game_url,
                        homepage: Some(page_url),
                        thumbnail: info.thumbnail.clone().or(cover),
                        tags: info.tags.clone(),
                        removed: false,
                        source: Some(catalog::ITCH_SOURCE.into()),
                        itch_id,
                    };
                    let mut job = job.borrow_mut();
                    job.report.imported += 1;
                    job.new_games.push(new_game);
                }
                None => job.borrow_mut().report.failed.push(title),
            }
            next_candidate(job);
        }),
    );
}

fn finish(job: Shared) {
    let new_games = std::mem::take(&mut job.borrow_mut().new_games);
    let saved = if new_games.is_empty() {
        Ok(())
    } else {
        catalog::add_itch_games(new_games)
            .and_then(|_| library::write(&catalog::load()).map(|_| ()))
            .map_err(|e| e.to_string())
    };
    let (done, report) = {
        let mut job = job.borrow_mut();
        (job.done.take(), std::mem::take(&mut job.report))
    };
    if let Some(done) = done {
        done(saved.map(|()| report));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tipo_do_projeto_decide_se_e_web() {
        assert_eq!(is_web_game(&json!({"id": 1, "type": "html"})), Some(true));
        assert_eq!(is_web_game(&json!({"id": 1, "type": "default"})), Some(false));
        assert_eq!(is_web_game(&json!({"id": 1})), None);
    }

    #[test]
    fn le_jogos_da_pagina_da_colecao() {
        let html = r#"<div class="grid"><div data-game_id="3052719" class="game_cell has_cover" dir="auto"><div class="game_thumb"><a href="https://scrwl.itch.io/high-on-track" class="thumb_link game_link"><img data-lazy_src="https://img.itch.zone/a/315x250%23c/x.png" src="https://img.itch.zone/a/315x250%23c/x.png"></a></div><div class="game_cell_data"><div class="game_title"><a href="https://scrwl.itch.io/high-on-track" data-action="game_grid" class="title game_link" data-label="game:3052719:title">High On Track</a></div><div title="t" class="game_text">Have a blast &amp; race!</div><div class="game_platform"><span class="web_flag">Play in browser</span></div></div></div><div data-game_id="42" class="game_cell"><div class="game_cell_data"><div class="game_title"><a href="https://a.itch.io/b" class="title game_link">Só download</a></div></div></div></div>"#;
        let games = parse_collection_page(html);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0]["id"], 3052719);
        assert_eq!(games[0]["title"], "High On Track");
        assert_eq!(games[0]["url"], "https://scrwl.itch.io/high-on-track");
        assert_eq!(games[0]["cover_url"], "https://img.itch.zone/a/315x250%23c/x.png");
        assert_eq!(games[0]["short_text"], "Have a blast & race!");
        assert_eq!(games[0]["type"], "html");
        assert_eq!(games[1]["type"], "default");
        assert_eq!(games[1]["title"], "Só download");
    }

    #[test]
    fn normaliza_paginas() {
        assert_eq!(normalize_page("https://Autor.itch.io/Jogo/?x=1"), "autor.itch.io/jogo");
        assert_eq!(normalize_page("http://www.itch.io/a#b"), "itch.io/a");
    }

    #[test]
    fn paginacao() {
        assert!(has_next_page(&json!({"per_page": 2}), 2));
        assert!(!has_next_page(&json!({"per_page": 2}), 1));
        assert!(!has_next_page(&json!({}), 0));
        assert!(has_next_page(&json!({}), 50));
    }
}

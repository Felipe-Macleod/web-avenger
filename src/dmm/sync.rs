//! Importa para a Biblioteca os jogos de navegador de "マイゲーム" (Meus jogos) do DMM.
//!
//! O DMM não tem API pública: a página `library.games.dmm.com`, lida com os cookies do
//! perfil principal (login feito no app), traz a lista no `__NEXT_DATA__` (estado do
//! Apollo do Next.js), com os jogos do DMM GAMES e do FANZA GAMES juntos.

use crate::{catalog, http, itch::sync::normalize_page, library};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

pub const MY_GAMES_URL: &str = "https://library.games.dmm.com/";

#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    /// Jogos de navegador adicionados à Biblioteca.
    pub imported: usize,
    /// Já estavam na Biblioteca (inclusive os que você removeu).
    pub existing: usize,
    /// Apagados da Biblioteca por terem saído de "Meus jogos" no DMM.
    pub removed: usize,
    /// Exigem o DMM GAMES PLAYER, são de celular ou de sites parceiros.
    pub not_web: usize,
    /// De navegador, mas sem endereço de jogo reconhecido.
    pub failed: Vec<String>,
}

/// Um jogo de "Meus jogos", como vem no estado do Apollo.
#[derive(Debug, Clone, PartialEq)]
pub struct MyGame {
    /// Ex.: `"710892-social-pc"` (`social-pc` = navegador no PC).
    pub id: String,
    pub name: String,
    pub description: String,
    pub thumbnail: Option<String>,
    /// Página do jogo na loja (`games.dmm.com/detail/<slug>`).
    pub link: String,
    pub genre: Option<String>,
    /// `siteType: "ADULT"` (FANZA GAMES).
    pub adult: bool,
    pub browser: bool,
}

/// Conteúdo da página "Meus jogos".
#[derive(Debug, PartialEq)]
pub struct MyGamesPage {
    /// `false` se a página veio sem login (sessão expirada ou desconectada).
    pub logged_in: bool,
    pub nickname: Option<String>,
    pub games: Vec<MyGame>,
}

pub fn run(done: Box<dyn FnOnce(Result<(SyncReport, Option<String>), String>)>) {
    fetch_my_games(Box::new(move |result| {
        let page = match result {
            Ok(page) if page.logged_in => page,
            Ok(_) => return done(Err("A sessão do DMM expirou. Clique em Conectar e faça login de novo.".into())),
            Err(error) => return done(Err(error)),
        };
        let nickname = page.nickname.clone();
        done(import(page.games).map(|report| (report, nickname)));
    }));
}

/// Lê "Meus jogos" com os cookies do perfil principal.
pub fn fetch_my_games(done: Box<dyn FnOnce(Result<MyGamesPage, String>)>) {
    let mut request = http::Request::get(MY_GAMES_URL);
    request.cookies = true;
    http::fetch(
        request,
        Box::new(move |result| {
            let page = match result {
                Ok(response) if response.ok() => parse_my_games(&response.text()),
                Ok(response) => Err(format!("o DMM respondeu HTTP {}", response.status)),
                Err(error) => Err(error),
            };
            // Fora do Japão o DMM devolve uma página de bloqueio, sem a lista.
            done(page.map_err(|error| {
                eprintln!("[web-avenger] DMM: {error}");
                "Não foi possível ler seus jogos no DMM. Fora do Japão, ative a VPN e tente de novo.".to_string()
            }))
        }),
    );
}

fn import(games: Vec<MyGame>) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    let known_ids = catalog::known_dmm_ids();
    let known_pages: HashSet<String> = catalog::load_all_pages().iter().map(|page| normalize_page(page)).collect();
    let mut new_games = Vec::new();
    // Lista vazia com login feito é mais provável ser a página que mudou do que a
    // biblioteca vazia: não apaga nada nesse caso.
    let seen: Option<HashSet<String>> = (!games.is_empty()).then(|| games.iter().map(|game| game.id.clone()).collect());
    for game in games {
        if known_ids.contains(&game.id) || known_pages.contains(&normalize_page(&game.link)) {
            report.existing += 1;
            continue;
        }
        if !game.browser {
            report.not_web += 1;
            continue;
        }
        match to_catalog_game(&game) {
            Some(new_game) => {
                report.imported += 1;
                new_games.push(new_game);
            }
            None => report.failed.push(game.name),
        }
    }
    let imported = !new_games.is_empty();
    (|| {
        if imported {
            catalog::add_imported_games(catalog::dmm_catalog_path(), new_games)?;
        }
        if let Some(seen) = &seen {
            report.removed = catalog::prune_imported_games(catalog::dmm_catalog_path(), |game| {
                game.dmm_id.as_ref().is_none_or(|id| seen.contains(id))
            })?;
        }
        if imported || report.removed > 0 {
            library::write(&catalog::load())?;
        }
        Ok::<(), std::io::Error>(())
    })()
    .map_err(|e| e.to_string())?;
    Ok(report)
}

/// Endereço do jogo: a página de detalhes (`games.dmm.com/detail/<slug>`) vira a de
/// jogar (`play.games.dmm.com/game/<slug>`); o mesmo no FANZA (`.co.jp`).
pub fn play_url(link: &str) -> Option<String> {
    let link = link.split(['?', '#']).next()?.trim_end_matches('/');
    let (host, slug) = link.strip_prefix("https://")?.split_once("/detail/")?;
    if slug.is_empty() || slug.contains('/') || !matches!(host, "games.dmm.com" | "games.dmm.co.jp") {
        return None;
    }
    Some(format!("https://play.{host}/game/{slug}"))
}

pub(crate) fn to_catalog_game(game: &MyGame) -> Option<catalog::Game> {
    let mut tags: Vec<String> = game.genre.iter().cloned().collect();
    if game.adult {
        tags.push("18+".into()); // `ADULT_TAGS` da Biblioteca: respeita o filtro +18
    }
    tags.push(if game.adult { "fanza" } else { "dmm" }.into());
    let url = play_url(&game.link)?;
    // Base do id (ver `catalog::assign_ids`): o slug da URL (`chaos_smash_860791`).
    let slug = url.rsplit('/').next().unwrap_or_default().to_string();
    Some(catalog::Game {
        id: slug,
        name: game.name.clone(),
        description: game.description.clone(),
        url,
        homepage: Some(game.link.clone()),
        thumbnail: game.thumbnail.clone(),
        tags,
        removed: false,
        no_adblock: false,
        source: Some(catalog::DMM_SOURCE.into()),
        itch_id: None,
        dmm_id: Some(game.id.clone()),
        added_at: None, // preenchido em `catalog::add_imported_games`
    })
}

/// Lê "Meus jogos" do `__NEXT_DATA__` da página.
pub fn parse_my_games(html: &str) -> Result<MyGamesPage, String> {
    let data = next_data(html)?;
    let props = &data["props"]["pageProps"];
    let state = props["__APOLLO_STATE__"].as_object().ok_or("página sem __APOLLO_STATE__")?;
    let logged_in = props["memberStatus"].as_str().is_some_and(|status| status != "GUEST") && !props["userId"].is_null();

    let nickname = state
        .iter()
        .find(|(key, value)| key.starts_with("Profile:") && value["nickname"].is_string())
        .and_then(|(_, profile)| profile["nickname"].as_str())
        .map(str::to_string);

    // `ROOT_QUERY["myGames({…})"]` → `MyGame:…` → `Game:…`.
    let root = state.get("ROOT_QUERY").ok_or("estado sem ROOT_QUERY")?;
    let my_games = root
        .as_object()
        .and_then(|root| root.iter().find(|(key, _)| key.starts_with("myGames(")))
        .and_then(|(_, list)| list.as_array());
    let Some(my_games) = my_games else {
        return Ok(MyGamesPage { logged_in: false, nickname, games: Vec::new() });
    };
    let resolve = |value: &Value| value["__ref"].as_str().and_then(|key| state.get(key)).cloned();
    let games = my_games.iter().filter_map(|entry| parse_game(&resolve(&resolve(entry)?["game"])?)).collect();
    Ok(MyGamesPage { logged_in, nickname, games })
}

/// JSON do `__NEXT_DATA__` de uma página do DMM.
pub(crate) fn next_data(html: &str) -> Result<Value, String> {
    const START: &str = r#"<script id="__NEXT_DATA__" type="application/json">"#;
    let start = html.find(START).ok_or("página sem __NEXT_DATA__")? + START.len();
    let end = html[start..].find("</script>").ok_or("__NEXT_DATA__ incompleto")? + start;
    serde_json::from_str(&html[start..end]).map_err(|e| format!("__NEXT_DATA__ inválido: {e}"))
}

/// Um jogo no formato do DMM (o mesmo em "Meus jogos" e nas listas da loja).
pub(crate) fn parse_game(game: &Value) -> Option<MyGame> {
    let text = |key: &str| game[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let id = text("id")?;
    Some(MyGame {
        browser: game["type"].as_str() == Some("BROWSER") && id.ends_with("-social-pc"),
        name: text("name")?,
        description: text("description").unwrap_or_default(),
        thumbnail: text("thumbnail"),
        link: text("link")?,
        genre: text("genre"),
        adult: game["siteType"].as_str() == Some("ADULT"),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(props: Value) -> String {
        let data = json!({ "props": { "pageProps": props } });
        format!(r#"<html><body><script id="__NEXT_DATA__" type="application/json">{data}</script></body></html>"#)
    }

    fn sample() -> String {
        page(json!({
            "memberStatus": "MEMBER",
            "userId": "37232718",
            "__APOLLO_STATE__": {
                "Profile:1": { "nickname": "Mascote007" },
                "ROOT_QUERY": {
                    "myGames({\"input\":{\"viewerDevice\":\"PC\"}})": [
                        { "__ref": "MyGame:MyGame:710892" },
                        { "__ref": "MyGame:MyGame:688044" },
                        { "__ref": "MyGame:MyGame:5" }
                    ]
                },
                "MyGame:MyGame:710892": { "game": { "__ref": "Game:710892-social-pc" } },
                "MyGame:MyGame:688044": { "game": { "__ref": "Game:688044-social-pc" } },
                "MyGame:MyGame:5": { "game": { "__ref": "Game:5-client-pc" } },
                "Game:710892-social-pc": {
                    "id": "710892-social-pc", "name": "カオス★スマッシュ", "description": "ド派手に決めろ",
                    "thumbnail": "https://media.games.dmm.com/freegame/app/710892/200.gif",
                    "link": "https://games.dmm.com/detail/chaos_smash_860791",
                    "type": "BROWSER", "genre": "ストラテジー", "siteType": "GENERAL"
                },
                "Game:688044-social-pc": {
                    "id": "688044-social-pc", "name": "モンスター娘TD X", "description": "",
                    "link": "https://games.dmm.co.jp/detail/monmusutdx",
                    "type": "BROWSER", "genre": "シミュレーション", "siteType": "ADULT"
                },
                "Game:5-client-pc": {
                    "id": "5-client-pc", "name": "Só no Player", "link": "https://games.dmm.com/detail/x",
                    "type": "DOWNLOAD", "siteType": "GENERAL"
                }
            }
        }))
    }

    #[test]
    fn le_meus_jogos() {
        let parsed = parse_my_games(&sample()).unwrap();
        assert!(parsed.logged_in);
        assert_eq!(parsed.nickname.as_deref(), Some("Mascote007"));
        assert_eq!(parsed.games.len(), 3);
        let chaos = &parsed.games[0];
        assert_eq!(chaos.id, "710892-social-pc");
        assert_eq!(chaos.name, "カオス★スマッシュ");
        assert!(chaos.browser && !chaos.adult);
        assert!(parsed.games[1].adult && parsed.games[1].thumbnail.is_none());
        assert!(!parsed.games[2].browser);
    }

    #[test]
    fn jogo_vira_cadastro() {
        let parsed = parse_my_games(&sample()).unwrap();
        let general = to_catalog_game(&parsed.games[0]).unwrap();
        assert_eq!(general.url, "https://play.games.dmm.com/game/chaos_smash_860791");
        assert_eq!(general.id, "chaos_smash_860791");
        assert_eq!(general.homepage.as_deref(), Some("https://games.dmm.com/detail/chaos_smash_860791"));
        assert_eq!(general.tags, ["ストラテジー", "dmm"]);
        assert_eq!(general.source.as_deref(), Some(catalog::DMM_SOURCE));
        assert_eq!(general.dmm_id.as_deref(), Some("710892-social-pc"));
        let adult = to_catalog_game(&parsed.games[1]).unwrap();
        assert_eq!(adult.url, "https://play.games.dmm.co.jp/game/monmusutdx");
        assert_eq!(adult.tags, ["シミュレーション", "18+", "fanza"]);
    }

    #[test]
    fn sem_login_ou_bloqueado() {
        let guest = page(json!({ "memberStatus": "GUEST", "__APOLLO_STATE__": { "ROOT_QUERY": {} } }));
        let parsed = parse_my_games(&guest).unwrap();
        assert!(!parsed.logged_in && parsed.games.is_empty());
        assert!(parse_my_games("<html>Not available in your region</html>").is_err());
    }

    #[test]
    fn endereco_de_jogar() {
        assert_eq!(play_url("https://games.dmm.com/detail/kancolle/").as_deref(), Some("https://play.games.dmm.com/game/kancolle"));
        assert_eq!(play_url("https://games.dmm.co.jp/detail/x?y=1").as_deref(), Some("https://play.games.dmm.co.jp/game/x"));
        assert_eq!(play_url("https://evil.com/detail/x"), None);
        assert_eq!(play_url("https://games.dmm.com/list/"), None);
    }
}

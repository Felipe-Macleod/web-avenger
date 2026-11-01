//! Aba "Explorar" da Biblioteca: jogos de navegador do itch.io, do DMM GAMES e do FANZA
//! GAMES (só com a opção "Mostrar jogos +18").
//!
//! A Biblioteca chama `avengerExplore(json, callback)` com `{action, ...}`; o processo
//! principal baixa as listas (fora das regras de CORS) e responde, como em `crate::dmm`.

use crate::{
    catalog, dmm, http,
    itch::sync::parse_collection_page,
    library, page_info, settings, shell,
};
use cef::*;
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};

/// Renderização → principal: `[id, json]`.
pub const REQUEST_MESSAGE: &str = "avenger.explore.request";
/// Principal → renderização: `[id, json]`.
pub const RESPONSE_MESSAGE: &str = "avenger.explore.response";

/// Origem dos jogos temporários abertos pelo Explorar (rodam no perfil principal).
const EXPLORE_SOURCE: &str = "Explorar";
/// Tags que marcam um jogo do itch.io como +18 (as mesmas `ADULT_TAGS` da Biblioteca).
const ADULT_TAGS: &[&str] = &["nsfw", "porn", "hentai", "adult", "erotic", "18+"];

type Reply = Box<dyn FnOnce(Value)>;

pub fn handle_request(frame: &mut Frame, message: &ProcessMessage) -> bool {
    if CefStringUtf16::from(&message.name()).to_string() != REQUEST_MESSAGE {
        return false;
    }
    let Some(args) = message.argument_list() else {
        return true;
    };
    let id = args.int(0);
    let request: Value = serde_json::from_str(&CefStringUtf16::from(&args.string(1)).to_string()).unwrap_or_default();
    let frame = frame.clone();
    let reply: Reply = Box::new(move |value| respond(&frame, id, &value));
    let show_adult = settings::load().show_adult;

    match request["action"].as_str().unwrap_or_default() {
        "list" => match request["store"].as_str().unwrap_or_default() {
            "itch" => {
                let page = request["page"].as_u64().unwrap_or(1).max(1);
                match itch_list_url(request["sort"].as_str().unwrap_or_default(), page, show_adult) {
                    Ok(url) => list_itch(url, reply),
                    Err(error) => reply(json!({ "ok": false, "error": error })),
                }
            }
            store @ ("dmm" | "fanza") => {
                let adult = store == "fanza";
                match dmm_list_url(adult, request["sort"].as_str().unwrap_or_default(), show_adult) {
                    Ok(url) if adult => allow_fanza(Box::new(move || list_dmm(url, true, reply))),
                    Ok(url) => list_dmm(url, false, reply),
                    Err(error) => reply(json!({ "ok": false, "error": error })),
                }
            }
            "feed" => match request["url"].as_str().filter(|url| is_http(url)) {
                Some(url) => list_feed(url.to_string(), show_adult, reply),
                None => reply(json!({ "ok": false, "error": "Endereço de feed inválido." })),
            },
            other => reply(json!({ "ok": false, "error": format!("loja desconhecida: {other}") })),
        },
        "play_page" => play_page(&request, show_adult, reply),
        "play_dmm" => play_dmm(&request, show_adult, reply),
        other => reply(json!({ "ok": false, "error": format!("ação desconhecida: {other}") })),
    }
    true
}

fn respond(frame: &Frame, id: i32, value: &Value) {
    let Some(mut message) = process_message_create(Some(&CefString::from(RESPONSE_MESSAGE))) else {
        return;
    };
    if let Some(args) = message.argument_list() {
        args.set_int(0, id);
        args.set_string(1, Some(&CefString::from(value.to_string().as_str())));
    }
    frame.send_process_message(ProcessId::RENDERER, Some(&mut message));
}

/// Página de navegação do itch.io (só jogos de navegador). `sort`: `""` (populares),
/// `top-rated`, `newest`, `top-sellers` ou `nsfw` (só com +18).
fn itch_list_url(sort: &str, page: u64, show_adult: bool) -> Result<String, String> {
    let path = match sort {
        "" => "platform-web".to_string(),
        "top-rated" | "newest" | "top-sellers" => format!("{sort}/platform-web"),
        "nsfw" if show_adult => "platform-web/tag-nsfw".to_string(),
        "nsfw" => return Err("Ligue \"Mostrar jogos +18\" para ver jogos +18.".into()),
        other => return Err(format!("ordem desconhecida: {other}")),
    };
    Ok(format!("https://itch.io/games/{path}?format=json&page={page}"))
}

/// Feed RSS/Atom cadastrado pelo usuário (sem cookies).
fn list_feed(url: String, show_adult: bool, reply: Reply) {
    http::fetch(
        http::Request::get(&url),
        Box::new(move |result| {
            let xml = match result {
                Ok(response) if response.ok() => response.text(),
                Ok(response) => return reply(json!({ "ok": false, "error": format!("o feed respondeu HTTP {}", response.status) })),
                Err(error) => return reply(json!({ "ok": false, "error": error })),
            };
            let Some(feed) = parse_feed(&xml) else {
                return reply(json!({ "ok": false, "error": "Esse endereço não é um feed RSS ou Atom." }));
            };
            let items: Vec<Value> = feed
                .items
                .into_iter()
                .filter(|item| show_adult || !item.categories.iter().any(|c| ADULT_TAGS.contains(&c.to_lowercase().as_str())))
                .map(|item| {
                    let domain = item.link.split_once("://").map(|(_, rest)| rest).unwrap_or_default();
                    let domain = domain.split(['/', '?', '#']).next().unwrap_or_default().trim_start_matches("www.");
                    json!({
                        "store": "feed",
                        "id": item.link,
                        "name": item.title,
                        "description": item.description,
                        "thumbnail": item.image,
                        "page_url": item.link,
                        "genre": domain,
                    })
                })
                .collect();
            reply(json!({ "ok": true, "title": feed.title, "more": false, "items": items }))
        }),
    );
}

#[derive(Debug, Default)]
struct Feed {
    title: Option<String>,
    items: Vec<FeedItem>,
}

#[derive(Debug, Default, PartialEq)]
struct FeedItem {
    title: String,
    link: String,
    description: Option<String>,
    image: Option<String>,
    categories: Vec<String>,
}

/// Lê um feed RSS 2.0 (`<item>`) ou Atom (`<entry>`). Tolerante: não valida o XML, só
/// procura os elementos conhecidos. `None` se não parecer um feed.
fn parse_feed(xml: &str) -> Option<Feed> {
    let (open, close) = if xml.contains("<rss") || xml.contains("<rdf:RDF") {
        ("<item", "</item>")
    } else if xml.contains("<feed") {
        ("<entry", "</entry>")
    } else {
        return None;
    };
    let mut items = Vec::new();
    let mut rest = xml;
    let mut header = None;
    while let Some(start) = find_tag(rest, open) {
        header.get_or_insert(&rest[..start]);
        let Some(end) = rest[start..].find(close) else { break };
        let block = &rest[start..start + end];
        rest = &rest[start + end + close.len()..];
        let Some(title) = element_text(block, "title").filter(|t| !t.is_empty()) else { continue };
        let link = element_text(block, "link")
            .filter(|l| !l.is_empty())
            .or_else(|| attribute(block, "<link", "href"))
            .unwrap_or_default();
        if !is_http(&link) {
            continue;
        }
        let raw_description = ["description", "summary", "content:encoded", "content"]
            .iter()
            .find_map(|name| element_raw(block, name));
        let image = ["<media:thumbnail", "<media:content"]
            .iter()
            .find_map(|tag| attribute(block, tag, "url"))
            .or_else(|| {
                find_tag(block, "<enclosure")
                    .filter(|&at| block[at..].split('>').next().is_some_and(|tag| tag.contains("image/")))
                    .and_then(|at| attribute(&block[at..], "<enclosure", "url"))
            })
            .or_else(|| raw_description.as_deref().and_then(|html| attribute(&decode(html), "<img", "src")))
            .filter(|url| is_http(url));
        let description = raw_description.map(|html| collapse(&strip_html(&decode(&html)))).filter(|d| !d.is_empty());
        let mut categories = Vec::new();
        let mut cats = block;
        while let Some(at) = find_tag(cats, "<category") {
            let tag = &cats[at..];
            // RSS: `<category>texto</category>`; Atom: `<category term="…"/>`.
            match attribute(tag, "<category", "term").or_else(|| element_text(tag, "category")) {
                Some(category) if !category.is_empty() => categories.push(category),
                _ => {}
            }
            cats = &tag[1..];
        }
        items.push(FeedItem { title, link, description, image, categories });
    }
    let title = element_text(header.unwrap_or(xml), "title").filter(|t| !t.is_empty());
    Some(Feed { title, items })
}

/// Posição de `<nome` seguido de espaço, `>` ou `/` (para `<link` não casar com `<linkx`).
fn find_tag(text: &str, open: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = text[from..].find(open) {
        let at = from + at;
        if matches!(text.as_bytes().get(at + open.len()), Some(b' ' | b'>' | b'/' | b'\n' | b'\t' | b'\r')) {
            return Some(at);
        }
        from = at + open.len();
    }
    None
}

/// Conteúdo de `<nome …>…</nome>`, sem CDATA (ainda com entidades).
fn element_raw(block: &str, name: &str) -> Option<String> {
    let start = find_tag(block, &format!("<{name}"))?;
    let tag_end = start + block[start..].find('>')?;
    if block[..tag_end].ends_with('/') {
        return None; // `<nome/>` vazio
    }
    let close = format!("</{name}>");
    let end = tag_end + block[tag_end..].find(&close)?;
    let inner = block[tag_end + 1..end].trim();
    Some(match inner.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")) {
        // CDATA guarda o texto literal: as entidades não se aplicam, então são escapadas
        // de volta para o `decode` posterior não mexer nelas.
        Some(cdata) => cdata.replace('&', "&amp;"),
        None => inner.to_string(),
    })
}

fn element_text(block: &str, name: &str) -> Option<String> {
    element_raw(block, name).map(|raw| collapse(&decode(&raw)))
}

/// Valor de `atributo="…"` (ou com aspas simples) na primeira tag `<nome`.
fn attribute(block: &str, open: &str, name: &str) -> Option<String> {
    let start = find_tag(block, open)?;
    let tag = &block[start..start + block[start..].find('>')?];
    for quote in ['"', '\''] {
        let prefix = format!(" {name}={quote}");
        if let Some(at) = tag.find(&prefix) {
            let value = &tag[at + prefix.len()..];
            return Some(decode(&value[..value.find(quote)?]));
        }
    }
    None
}

/// Entidades XML/HTML, inclusive numéricas (`&#8217;`, `&#x27;`).
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("&#") {
        out.push_str(&rest[..at]);
        let entity = &rest[at + 2..];
        let decoded = entity.find(';').filter(|&end| end <= 8).and_then(|end| {
            let code = &entity[..end];
            let value = match code.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => code.parse().ok()?,
            };
            Some((char::from_u32(value)?, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &entity[end + 1..];
            }
            None => {
                out.push_str("&#");
                rest = entity;
            }
        }
    }
    out.push_str(rest);
    crate::itch::sync::decode_html(&out.replace("&apos;", "'").replace("&nbsp;", " "))
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lista de jogos de navegador do DMM (ou do FANZA, só com +18). `sort`: `new`,
/// `popular` (padrão) ou `start` (mais jogados).
fn dmm_list_url(adult: bool, sort: &str, show_adult: bool) -> Result<String, String> {
    if adult && !show_adult {
        return Err("Ligue \"Mostrar jogos +18\" para ver o FANZA.".into());
    }
    let sort = match sort {
        "" => "popular",
        "new" | "popular" | "start" => sort,
        other => return Err(format!("ordem desconhecida: {other}")),
    };
    let host = if adult { "games.dmm.co.jp" } else { "games.dmm.com" };
    Ok(format!("https://{host}/list/pc?game=browser&sort={sort}"))
}

/// Sem cookies: a listagem anônima do itch.io não traz jogos +18 (a conta do usuário
/// poderia trazer).
fn list_itch(url: String, reply: Reply) {
    http::fetch(
        http::Request::get(&url),
        Box::new(move |result| {
            let json: Value = match result {
                Ok(response) if response.ok() => serde_json::from_slice(&response.body).unwrap_or_default(),
                Ok(response) => return reply(json!({ "ok": false, "error": format!("o itch.io respondeu HTTP {}", response.status) })),
                Err(error) => return reply(json!({ "ok": false, "error": error })),
            };
            let Some(content) = json["content"].as_str() else {
                return reply(json!({ "ok": false, "error": "resposta inesperada do itch.io" }));
            };
            let items: Vec<Value> = parse_collection_page(content)
                .into_iter()
                .map(|game| {
                    json!({
                        "store": "itch",
                        "id": game["id"],
                        "name": game["title"],
                        "description": game["short_text"],
                        "thumbnail": game["cover_url"],
                        "page_url": game["url"],
                    })
                })
                .collect();
            reply(json!({ "ok": true, "more": !items.is_empty(), "items": items }))
        }),
    );
}

/// Lista de jogos de navegador da loja (`__NEXT_DATA__`), lida com os cookies do perfil.
fn list_dmm(url: String, adult: bool, reply: Reply) {
    let mut request = http::Request::get(&url);
    request.cookies = true;
    let url = url.clone(); // para o log
    http::fetch(
        request,
        Box::new(move |result| {
            let items = match result {
                Ok(response) if response.ok() => parse_dmm_list(&response.text(), adult),
                Ok(response) => Err(format!("o DMM respondeu HTTP {}", response.status)),
                Err(error) => Err(error),
            };
            match items {
                Ok(items) => reply(json!({ "ok": true, "more": false, "items": items })),
                Err(error) => {
                    eprintln!("[web-avenger] Explorar: {url}: {error}");
                    // Fora do Japão o DMM devolve uma página de bloqueio, sem a lista.
                    reply(json!({ "ok": false, "error": "Não foi possível ler a lista do DMM. Fora do Japão, ative a VPN e tente de novo." }))
                }
            }
        }),
    );
}

fn parse_dmm_list(html: &str, adult: bool) -> Result<Vec<Value>, String> {
    let data = dmm::sync::next_data(html)?;
    let items = data["props"]["pageProps"]["initialGames"]["items"].as_array().ok_or("página sem initialGames")?;
    Ok(items
        .iter()
        .filter_map(dmm::sync::parse_game)
        .filter(|game| game.browser)
        .map(|game| {
            json!({
                "store": if adult { "fanza" } else { "dmm" },
                "id": game.id,
                "name": game.name,
                "description": game.description,
                "thumbnail": game.thumbnail,
                "link": game.link,
                "genre": game.genre,
                "adult": adult,
            })
        })
        .collect())
}

/// O FANZA pede a verificação de idade (cookie `age_check_done`). A opção +18 ligada é
/// a confirmação do usuário.
fn allow_fanza(next: Box<dyn FnOnce()>) {
    let next = Rc::new(RefCell::new(Some(next)));
    let run = |next: &Rc<RefCell<Option<Box<dyn FnOnce()>>>>| {
        if let Some(next) = next.borrow_mut().take() {
            next();
        }
    };
    let Some(manager) = cookie_manager_get_global_manager(None) else {
        return run(&next);
    };
    let cookie = Cookie {
        name: CefString::from("age_check_done"),
        value: CefString::from("1"),
        domain: CefString::from(".dmm.co.jp"),
        path: CefString::from("/"),
        secure: 1,
        ..Default::default()
    };
    let mut callback = CookieSet::new(next.clone());
    if manager.set_cookie(Some(&CefString::from("https://games.dmm.co.jp/")), Some(&cookie), Some(&mut callback)) == 0 {
        run(&next);
    }
}

wrap_set_cookie_callback! {
    struct CookieSet {
        next: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
    }

    impl SetCookieCallback {
        fn on_complete(&self, _success: ::std::os::raw::c_int) {
            if let Some(next) = self.next.borrow_mut().take() {
                next();
            }
        }
    }
}

/// Abre o jogo de uma página (itch.io ou item de feed) numa janela, sem adicioná-lo à
/// Biblioteca. Páginas que só incorporam o jogo (ex.: itch.io) abrem o jogo em si.
fn play_page(request: &Value, show_adult: bool, reply: Reply) {
    let Some(page_url) = request["page_url"].as_str().filter(|url| is_http(url)).map(str::to_string) else {
        return reply(json!({ "ok": false, "error": "jogo inválido" }));
    };
    let name = request["name"].as_str().unwrap_or("Jogo").to_string();
    let itch_id = request["id"].as_u64();
    page_info::fetch(
        page_url.clone(),
        false,
        Box::new(move |info| {
            if !show_adult && info.tags.iter().any(|tag| ADULT_TAGS.contains(&tag.as_str())) {
                return reply(json!({ "ok": false, "error": "Jogo +18. Ligue \"Mostrar jogos +18\" para jogar." }));
            }
            let game_url = info.game_url.clone().unwrap_or_else(|| page_url.clone());
            // Temporário (não é gravado): roda no perfil principal, como os importados.
            let game = catalog::Game {
                id: format!("explorar-{:x}", hash(&page_url)),
                name: info.title.clone().unwrap_or(name),
                description: info.description.clone().unwrap_or_default(),
                url: game_url.clone(),
                homepage: Some(page_url),
                thumbnail: info.thumbnail.clone(),
                tags: info.tags.clone(),
                removed: false,
                no_adblock: false,
                source: Some(EXPLORE_SOURCE.into()),
                itch_id,
                dmm_id: None,
                added_at: None,
            };
            shell::open_game(game, game_url);
            reply(json!({ "ok": true }));
        }),
    );
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn is_http(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Grava o jogo do DMM/FANZA na Biblioteca, como a sincronização faria, e o abre. O DMM
/// coloca o jogo em "Meus jogos" quando ele é iniciado, então a sincronização o mantém.
fn play_dmm(request: &Value, show_adult: bool, reply: Reply) {
    let text = |key: &str| request[key].as_str().map(str::to_string);
    let (Some(id), Some(name), Some(link)) = (text("id"), text("name"), text("link")) else {
        return reply(json!({ "ok": false, "error": "jogo inválido" }));
    };
    let adult = request["adult"].as_bool().unwrap_or(false) || link.contains(".dmm.co.jp/");
    if adult && !show_adult {
        return reply(json!({ "ok": false, "error": "Ligue \"Mostrar jogos +18\" para jogar." }));
    }
    let found = |id: &str| catalog::load().into_iter().find(|game| game.dmm_id.as_deref() == Some(id));
    let mut added = false;
    if !catalog::known_dmm_ids().contains(&id) {
        let game = dmm::sync::MyGame {
            id: id.clone(),
            name,
            description: text("description").unwrap_or_default(),
            thumbnail: text("thumbnail"),
            link,
            genre: text("genre"),
            adult,
            browser: true,
        };
        let Some(new_game) = dmm::sync::to_catalog_game(&game) else {
            return reply(json!({ "ok": false, "error": "Endereço de jogo não reconhecido." }));
        };
        let saved = catalog::add_imported_games(catalog::dmm_catalog_path(), vec![new_game])
            .and_then(|_| library::write(&catalog::load()).map(|_| ()));
        if let Err(error) = saved {
            return reply(json!({ "ok": false, "error": error.to_string() }));
        }
        added = true;
    }
    // Já conhecido mas removido pelo usuário: não volta para a Biblioteca.
    let Some(game) = found(&id) else {
        return reply(json!({ "ok": false, "error": "Você removeu esse jogo da Biblioteca." }));
    };
    let url = game.url.clone();
    shell::open_game(game, url);
    reply(json!({ "ok": true, "added": added }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endereco_da_lista_do_itch() {
        assert_eq!(itch_list_url("", 1, false).unwrap(), "https://itch.io/games/platform-web?format=json&page=1");
        assert_eq!(
            itch_list_url("top-rated", 3, false).unwrap(),
            "https://itch.io/games/top-rated/platform-web?format=json&page=3"
        );
        assert_eq!(
            itch_list_url("nsfw", 1, true).unwrap(),
            "https://itch.io/games/platform-web/tag-nsfw?format=json&page=1"
        );
        assert!(itch_list_url("nsfw", 1, false).is_err());
        assert!(itch_list_url("../x", 1, true).is_err());
    }

    #[test]
    fn le_feed_rss() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><channel>
            <title><![CDATA[Catálogo & cia]]></title><link>https://github.com/x</link>
            <image><url>https://github.com/fluidicon.png</url><title>Outro</title></image>
            <item><title><![CDATA[⭐ 20 Minutes Till Dawn]]></title><link>https://gn-math.dev/?id=819</link>
              <guid isPermaLink="false">c09a</guid><dc:creator><![CDATA[gn-math.dev]]></dc:creator></item>
            <item><title>Jogo &amp; mais &#8217;</title><link>https://a.itch.io/jogo</link>
              <description>&lt;p&gt;Texto &lt;b&gt;curto&lt;/b&gt;&lt;/p&gt;&lt;img src="https://img.exemplo/capa.png"&gt;</description>
              <category>NSFW</category><category>Puzzle</category></item>
            <item><title>Sem link</title></item>
            <item><title>Arquivo</title><link>file:///etc/passwd</link></item>
        </channel></rss>"#;
        let feed = parse_feed(xml).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Catálogo & cia"));
        assert_eq!(feed.items.len(), 2);
        assert_eq!(feed.items[0], FeedItem {
            title: "⭐ 20 Minutes Till Dawn".into(),
            link: "https://gn-math.dev/?id=819".into(),
            ..Default::default()
        });
        let second = &feed.items[1];
        assert_eq!(second.title, "Jogo & mais \u{2019}");
        assert_eq!(second.description.as_deref(), Some("Texto curto"));
        assert_eq!(second.image.as_deref(), Some("https://img.exemplo/capa.png"));
        assert_eq!(second.categories, ["NSFW", "Puzzle"]);
    }

    #[test]
    fn le_feed_atom() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
            <title>Atom</title>
            <entry><title type="html">Jogo</title><link rel="alternate" href="https://exemplo.org/jogo/"/>
              <summary>Resumo</summary><category term="adult"/>
              <media:thumbnail url="https://exemplo.org/t.jpg"/></entry>
        </feed>"#;
        let feed = parse_feed(xml).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Atom"));
        let entry = &feed.items[0];
        assert_eq!(entry.link, "https://exemplo.org/jogo/");
        assert_eq!(entry.description.as_deref(), Some("Resumo"));
        assert_eq!(entry.image.as_deref(), Some("https://exemplo.org/t.jpg"));
        assert_eq!(entry.categories, ["adult"]);
        assert!(parse_feed("<html><body>não é feed</body></html>").is_none());
    }

    #[test]
    fn endereco_da_lista_do_dmm() {
        assert_eq!(dmm_list_url(false, "", false).unwrap(), "https://games.dmm.com/list/pc?game=browser&sort=popular");
        assert_eq!(dmm_list_url(true, "start", true).unwrap(), "https://games.dmm.co.jp/list/pc?game=browser&sort=start");
        assert!(dmm_list_url(true, "new", false).is_err());
        assert!(dmm_list_url(false, "x&y=1", false).is_err());
    }

    #[test]
    fn le_a_lista_do_dmm() {
        let data = json!({ "props": { "pageProps": { "initialGames": { "items": [
            {
                "id": "710892-social-pc", "type": "BROWSER", "name": "カオス★スマッシュ",
                "description": "ド派手に決めろ", "thumbnail": "https://media.games.dmm.com/freegame/app/710892/200.gif",
                "link": "https://games.dmm.com/detail/chaos_smash_860791", "genre": "ストラテジー"
            },
            { "id": "5-client-pc", "type": "DOWNLOAD", "name": "Só no Player", "link": "https://games.dmm.com/detail/x" }
        ] } } } });
        let html = format!(r#"<script id="__NEXT_DATA__" type="application/json">{data}</script>"#);
        let items = parse_dmm_list(&html, false).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "710892-social-pc");
        assert_eq!(items[0]["store"], "dmm");
        assert_eq!(items[0]["link"], "https://games.dmm.com/detail/chaos_smash_860791");
        assert_eq!(parse_dmm_list(&html, true).unwrap()[0]["store"], "fanza");
        assert!(parse_dmm_list("<html>bloqueado</html>", false).is_err());
    }
}

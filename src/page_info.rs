//! Informações de uma página (título, capa, descrição) para o cadastro de jogos.
//!
//! A Biblioteca (`file://`) não pode ler outros sites por causa do CORS, então o
//! processo de renderização pede ao processo principal, que baixa a página com um
//! `CefURLRequest` (fora das regras de CORS) e devolve o resultado por mensagem.

use cef::*;
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// Renderização → principal: `[id, url]`.
pub const REQUEST_MESSAGE: &str = "avenger.page-info.request";
/// Principal → renderização: `[id, json]`.
pub const RESPONSE_MESSAGE: &str = "avenger.page-info.response";

/// O suficiente para o `<head>` de qualquer página.
const MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
pub struct PageInfo {
    pub ok: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    /// URL do jogo em si, quando a página só o incorpora (ex.: jogo HTML5 do itch.io).
    pub game_url: Option<String>,
    pub tags: Vec<String>,
}

/// Processo principal: trata o pedido vindo de um frame. Retorna se a mensagem era nossa.
pub fn handle_request(frame: &mut Frame, message: &ProcessMessage) -> bool {
    if CefStringUtf16::from(&message.name()).to_string() != REQUEST_MESSAGE {
        return false;
    }
    let Some(args) = message.argument_list() else {
        return true;
    };
    let id = args.int(0);
    let url = CefStringUtf16::from(&args.string(1)).to_string();

    let mut client = PageInfoClient::new(id, url.clone(), frame.clone(), Arc::default());
    if !start_request(&url, &mut client) {
        respond(frame, id, &PageInfo::default());
    }
    true
}

fn start_request(url: &str, client: &mut UrlrequestClient) -> bool {
    let Some(mut request) = request_create() else {
        return false;
    };
    request.set_url(Some(&CefString::from(url)));
    request.set_method(Some(&CefString::from("GET")));
    urlrequest_create(Some(&mut request), Some(client), None).is_some()
}

/// Corpo da resposta, se a requisição terminou com sucesso.
fn finished_body(request: Option<&mut Urlrequest>, body: &Mutex<Vec<u8>>) -> Option<String> {
    let success = request.is_some_and(|request| {
        request.request_status() == UrlrequestStatus::SUCCESS
            && request.response().is_some_and(|response| (200..400).contains(&response.status()))
    });
    let body = body.lock().ok()?;
    success.then(|| String::from_utf8_lossy(&body).into_owned())
}

fn append_chunk(body: &Mutex<Vec<u8>>, data: *const u8, data_length: usize) {
    if data.is_null() {
        return;
    }
    // SAFETY: o CEF garante `data_length` bytes válidos durante a chamada.
    let chunk = unsafe { std::slice::from_raw_parts(data, data_length) };
    if let Ok(mut body) = body.lock() {
        let room = MAX_BYTES.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
}

fn respond(frame: &Frame, id: i32, info: &PageInfo) {
    let Some(mut message) = process_message_create(Some(&CefString::from(RESPONSE_MESSAGE))) else {
        return;
    };
    if let Some(args) = message.argument_list() {
        let json = serde_json::to_string(info).unwrap_or_default();
        args.set_int(0, id);
        args.set_string(1, Some(&CefString::from(json.as_str())));
    }
    frame.send_process_message(ProcessId::RENDERER, Some(&mut message));
}

wrap_urlrequest_client! {
    struct PageInfoClient {
        id: i32,
        url: String,
        frame: Frame,
        body: Arc<Mutex<Vec<u8>>>,
    }

    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            append_chunk(&self.body, data, data_length);
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let Some(html) = finished_body(request, &self.body) else {
                respond(&self.frame, self.id, &PageInfo::default());
                return;
            };
            let info = parse(&html, &self.url);
            // No itch.io, completa com o `data.json` público da página (nome limpo e tags).
            if is_itch(&html, &self.url) {
                let mut client = ItchDataClient::new(self.id, self.frame.clone(), info.clone(), Arc::default());
                if start_request(&itch_data_url(&self.url), &mut client) {
                    return;
                }
            }
            respond(&self.frame, self.id, &info);
        }
    }
}

wrap_urlrequest_client! {
    struct ItchDataClient {
        id: i32,
        frame: Frame,
        info: PageInfo,
        body: Arc<Mutex<Vec<u8>>>,
    }

    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            append_chunk(&self.body, data, data_length);
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let mut info = self.info.clone();
            if let Some(json) = finished_body(request, &self.body) {
                merge_itch_data(&mut info, &json);
            }
            respond(&self.frame, self.id, &info);
        }
    }
}

/// Extrai título, descrição e imagem de capa do HTML.
pub fn parse(html: &str, base_url: &str) -> PageInfo {
    let mut info = PageInfo { ok: true, ..Default::default() };
    let lower = html.to_ascii_lowercase();
    let (mut og_title, mut description, mut image) = (None, None, None);

    let mut rest = 0;
    while let Some(start) = lower[rest..].find("<meta").map(|i| i + rest) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| i + start);
        let tag = &html[start..end];
        rest = end;
        let key = attribute(tag, "property").or_else(|| attribute(tag, "name"));
        let Some((key, content)) = key.zip(attribute(tag, "content")) else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "og:title" => og_title = og_title.or(Some(content)),
            "og:description" | "description" => description = description.or(Some(content)),
            "og:image" | "twitter:image" => image = image.or(Some(content)),
            _ => {}
        }
    }

    let title = lower.find("<title").and_then(|start| {
        let open = lower[start..].find('>')? + start + 1;
        let close = lower[open..].find("</title")? + open;
        Some(decode_entities(&html[open..close]))
    });

    let clean = |text: String| {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    };
    info.title = title.and_then(clean).or_else(|| og_title.and_then(clean));
    info.description = description.and_then(clean);
    info.thumbnail = image.and_then(clean).map(|image| resolve_url(base_url, &image));
    if is_itch(html, base_url) {
        parse_itch(html, &lower, base_url, &mut info);
    }
    info
}

// ---- itch.io ----
//
// Sem chave de API: o nome, a capa e as tags vêm do `data.json` público da página
// (o mesmo usado pelos widgets do itch), e o jogo HTML5 vem do iframe `game_drop`
// que a página carrega ao clicar em "Run game" (o recurso "Embed game" do itch).

fn is_itch(html: &str, url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
    host.ends_with(".itch.io") || html.contains("name=\"itch:path\"")
}

fn itch_data_url(page_url: &str) -> String {
    let page = page_url.split(['?', '#']).next().unwrap_or(page_url);
    format!("{}/data.json", page.trim_end_matches('/'))
}

fn parse_itch(html: &str, lower: &str, base_url: &str, info: &mut PageInfo) {
    // O <title> é "Jogo by autores"; o <h1 class="game_title"> tem só o nome.
    if let Some(title) = element_text(html, lower, "game_title") {
        info.title = Some(title);
    }
    info.game_url = itch_game_url(html, lower).map(|src| {
        // O `?v=` só força o cache; sem ele a URL sempre aponta para o build atual.
        let src = resolve_url(base_url, &src);
        src.split('?').next().unwrap_or(&src).to_string()
    });
    let mut tags = Vec::new();
    for row in ["Genre", "Tags", "Made with"] {
        tags.extend(info_table_links(html, row));
    }
    info.tags = normalize_tags(tags);
}

/// `src` do iframe do jogo: escapado em `data-iframe` (clique para jogar) ou direto.
fn itch_game_url(html: &str, lower: &str) -> Option<String> {
    if let Some(pos) = lower.find("data-iframe=") {
        let start = lower[..pos].rfind('<')?;
        let end = lower[pos..].find('>').map_or(lower.len(), |i| i + pos);
        let iframe = attribute(&html[start..end], "data-iframe")?;
        return attribute(&iframe, "src");
    }
    let mut from = 0;
    while let Some(start) = lower[from..].find("<iframe").map(|i| i + from) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| i + start);
        let tag = &html[start..end];
        from = end;
        if attribute(tag, "id").as_deref() == Some("game_drop") {
            return attribute(tag, "src");
        }
    }
    None
}

/// Texto de um elemento pela classe (sem as tags internas).
fn element_text(html: &str, lower: &str, class: &str) -> Option<String> {
    let mut from = 0;
    while let Some(start) = lower[from..].find('<').map(|i| i + from) {
        let end = lower[start..].find('>')? + start;
        from = end;
        let tag = &html[start..end];
        let has_class = attribute(tag, "class")
            .is_some_and(|classes| classes.split_whitespace().any(|c| c == class));
        if !has_class {
            continue;
        }
        let name: String = lower[start + 1..end].chars().take_while(char::is_ascii_alphanumeric).collect();
        let close = lower[end..].find(&format!("</{name}"))? + end;
        let text = strip_tags(&html[end + 1..close]);
        return (!text.is_empty()).then_some(text);
    }
    None
}

fn strip_tags(fragment: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    decode_entities(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Textos dos links de uma linha da tabela de informações (`<td>Genre</td><td>…</td>`).
fn info_table_links(html: &str, row: &str) -> Vec<String> {
    let Some(start) = html.find(&format!("<td>{row}</td><td>")) else {
        return Vec::new();
    };
    let cell_start = start + row.len() + "<td></td><td>".len();
    let cell_end = html[cell_start..].find("</td>").map_or(html.len(), |i| i + cell_start);
    html[cell_start..cell_end]
        .split("</a>")
        .filter_map(|part| part.rfind('>').map(|i| strip_tags(&part[i + 1..])))
        .filter(|text| !text.is_empty())
        .collect()
}

fn normalize_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags.into_iter().chain(["itch.io".to_string()]) {
        let tag = tag.trim().to_lowercase();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Completa com o `data.json` público: `{"title", "cover_image", "tags", ...}`.
fn merge_itch_data(info: &mut PageInfo, json: &str) {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let text = |key: &str| data.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    if let Some(title) = text("title") {
        info.title = Some(decode_entities(title));
    }
    // O `og:image` da página é a capa original; a do data.json vem recortada.
    if info.thumbnail.is_none() {
        info.thumbnail = text("cover_image").map(str::to_string);
    }
    if let Some(tags) = data.get("tags").and_then(|v| v.as_array()).filter(|tags| !tags.is_empty()) {
        info.tags = normalize_tags(tags.iter().filter_map(|t| t.as_str()).map(str::to_string));
    }
}

/// Valor de um atributo dentro de uma tag (`name="valor"`, `name='valor'` ou `name=valor`).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name).map(|i| i + from) {
        from = pos + name.len();
        let boundary = pos == 0 || lower.as_bytes()[pos - 1].is_ascii_whitespace();
        let after = lower[from..].trim_start();
        if !boundary || !after.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - after.len() + 1;
        let value = tag[value_start..].trim_start();
        let raw = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or(""),
        };
        return Some(decode_entities(raw));
    }
    None
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(10)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Resolve uma URL relativa (ex.: `og:image` = `/capa.jpg`) contra a URL da página.
fn resolve_url(base: &str, url: &str) -> String {
    if url.contains("://") || url.starts_with("data:") {
        return url.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let scheme = &base[..scheme_end.saturating_sub(3)];
    if let Some(rest) = url.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    let path_start = base[scheme_end..].find('/').map_or(base.len(), |i| i + scheme_end);
    let origin = &base[..path_start];
    if url.starts_with('/') {
        return format!("{origin}{url}");
    }
    let path = base[path_start..].split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    format!("{origin}{dir}{url}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrai_titulo_descricao_e_capa() {
        let html = r#"<html><head>
            <meta charset="utf-8"><title> Skate 3 &amp; Amigos
            </title><meta name="description" content="Um jogo &quot;legal&quot;">
            <meta property='og:image' content='/img/capa.jpg'></head></html>"#;
        let info = parse(html, "https://exemplo.com/jogos/skate?x=1");
        assert_eq!(info.title.as_deref(), Some("Skate 3 & Amigos"));
        assert_eq!(info.description.as_deref(), Some("Um jogo \"legal\""));
        assert_eq!(info.thumbnail.as_deref(), Some("https://exemplo.com/img/capa.jpg"));
    }

    #[test]
    fn resolve_urls_relativas() {
        assert_eq!(resolve_url("https://a.com/x/y.html", "z.png"), "https://a.com/x/z.png");
        assert_eq!(resolve_url("https://a.com", "z.png"), "https://a.com/z.png");
        assert_eq!(resolve_url("https://a.com/x/", "//cdn.b.com/z.png"), "https://cdn.b.com/z.png");
        assert_eq!(resolve_url("https://a.com/x/", "https://c.com/z.png"), "https://c.com/z.png");
    }

    const ITCH_HTML: &str = r#"<title>Colorful Path by ricardobadan, Felipe Inoue</title>
        <meta name="itch:path" content="games/1622793"/>
        <meta property="og:image" content="https://img.itch.zone/capa.jpg"/>
        <h1 class="game_title">Colorful Path</h1>
        <div data-width="1280" class="game_frame game_pending"><div data-iframe="&lt;iframe allowfullscreen=&quot;true&quot; src=&quot;https://html-classic.itch.zone/html/6182212/index.html?v=1782773036&quot; id=&quot;game_drop&quot;&gt;&lt;/iframe&gt;" class="iframe_placeholder"><button>Run game</button></div></div>
        <tr><td>Genre</td><td><a href="https://itch.io/games/genre-puzzle">Puzzle</a></td></tr><tr><td>Made with</td><td><a href="https://itch.io/games/made-with-unity">Unity</a></td></tr>"#;

    #[test]
    fn itch_extrai_nome_jogo_e_tags_da_pagina() {
        let info = parse(ITCH_HTML, "https://ricardobadan.itch.io/colorful-path");
        assert_eq!(info.title.as_deref(), Some("Colorful Path"));
        assert_eq!(info.game_url.as_deref(), Some("https://html-classic.itch.zone/html/6182212/index.html"));
        assert_eq!(info.thumbnail.as_deref(), Some("https://img.itch.zone/capa.jpg"));
        assert_eq!(info.tags, ["puzzle", "unity", "itch.io"]);
    }

    #[test]
    fn itch_iframe_direto() {
        let html = r#"<iframe frameborder="0" id="game_drop" src="//html.itch.zone/html/42/game.html"></iframe>"#;
        let info = parse(html, "https://autor.itch.io/jogo");
        assert_eq!(info.game_url.as_deref(), Some("https://html.itch.zone/html/42/game.html"));
    }

    #[test]
    fn itch_data_json_substitui_tags_e_titulo() {
        let mut info = parse(ITCH_HTML, "https://ricardobadan.itch.io/colorful-path");
        merge_itch_data(&mut info, r#"{"title":"Colorful Path","cover_image":"https://x/c.jpg","tags":["gmtkjam","puzzle"]}"#);
        assert_eq!(info.tags, ["gmtkjam", "puzzle", "itch.io"]);
        assert_eq!(info.thumbnail.as_deref(), Some("https://img.itch.zone/capa.jpg"));
        assert_eq!(itch_data_url("https://a.itch.io/jogo/?x=1"), "https://a.itch.io/jogo/data.json");
    }

    #[test]
    fn pagina_comum_nao_e_itch() {
        let info = parse("<title>Outro</title>", "https://exemplo.com/itch.io");
        assert!(info.game_url.is_none() && info.tags.is_empty());
    }

    #[test]
    fn usa_og_title_sem_title() {
        let info = parse(r#"<meta property="og:title" content="Jogo">"#, "https://a.com");
        assert_eq!(info.title.as_deref(), Some("Jogo"));
    }
}

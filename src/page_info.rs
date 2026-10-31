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

#[derive(Debug, Default, Serialize)]
pub struct PageInfo {
    pub ok: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
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

    let request = request_create();
    let started = request.as_ref().is_some_and(|request| {
        request.set_url(Some(&CefString::from(url.as_str())));
        request.set_method(Some(&CefString::from("GET")));
        let mut client = PageInfoClient::new(id, url.clone(), frame.clone(), Arc::default());
        urlrequest_create(Some(&mut request.clone()), Some(&mut client), None).is_some()
    });
    if !started {
        respond(frame, id, &PageInfo::default());
    }
    true
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
            if data.is_null() {
                return;
            }
            // SAFETY: o CEF garante `data_length` bytes válidos durante a chamada.
            let chunk = unsafe { std::slice::from_raw_parts(data, data_length) };
            if let Ok(mut body) = self.body.lock() {
                let room = MAX_BYTES.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let success = request.is_some_and(|request| {
                request.request_status() == UrlrequestStatus::SUCCESS
                    && request.response().is_some_and(|response| (200..400).contains(&response.status()))
            });
            let info = match self.body.lock() {
                Ok(body) if success => parse(&String::from_utf8_lossy(&body), &self.url),
                _ => PageInfo::default(),
            };
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
    info
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

    #[test]
    fn usa_og_title_sem_title() {
        let info = parse(r#"<meta property="og:title" content="Jogo">"#, "https://a.com");
        assert_eq!(info.title.as_deref(), Some("Jogo"));
    }
}
